//! Iterative group-based conversation compaction.
//!
//! When the context window fills up, split the conversation into groups
//! (each = user turn + assistant reply + tool results), summarize the oldest
//! groups via a non-streaming API call, keep the most recent groups verbatim.
//!
//! Algorithm (mirrors CC v126 `biK` + `To1` smart step):
//!
//! 1. Split messages into groups via `split_into_groups`.
//! 2. Preserve the most-recent N groups, summarize the rest.
//! 3. If summarization is too long → use `token_gap_step` to calculate
//!    exactly how many more groups to preserve based on per-group token
//!    counts, falling back to exponential doubling when no gap info.
//! 4. If media_too_large → strip images/PDFs and retry once.
//! 5. Circuit breaker: if context refills within `THRASH_TURN_WINDOW`
//!    turns of the last compact, `CIRCUIT_BREAKER_LIMIT` times in a row,
//!    stop trying.

use crate::types::ChatMessage;
use tracing::{debug, trace};

pub const CHARS_PER_TOKEN: usize = 4;
/// Multiplier applied to the char-based estimate to account for wire overhead
/// (system prompt, tool definitions, JSON framing, role markers) that is not
/// visible in message text. Empirical measurement: API reports ~1.4–1.5× more
/// tokens than naive char_count/4 on tool-heavy conversations.
const OVERHEAD_MULTIPLIER_NUM: usize = 3;
const OVERHEAD_MULTIPLIER_DEN: usize = 2; // 3/2 = 1.5×
pub const MAX_ATTEMPTS: u32 = 8;
pub const CIRCUIT_BREAKER_LIMIT: u32 = 3;
/// If context refills within this many user turns after a compact, it counts
/// as thrash. Mirrors v126's `lG6 = 3` (cli.2.1.126.deob.js:397362) — was 2,
/// which made the breaker trip one turn earlier than upstream.
pub const THRASH_TURN_WINDOW: u32 = 3;

// v177 threshold algorithm — `Q9H` / `mu8` / `ZB7` in cli.js.
// The model's nominal window minus output headroom gives the effective window,
// then three headrooms from that give the trigger levels. Using fixed token
// offsets (not percentages) keeps behavior consistent across 200K and 1M-context
// models — the buffer needed for the next user turn + the outgoing compaction
// summary doesn't scale with window size.
//
// Step 1 (Q9H): effective_window = window - min(max_output_tokens, OUTPUT_HEADROOM_CAP)
// Step 2 (mu8): compact_threshold = effective_window - COMPACT_HEADROOM
//
// Trigger levels:
//   tokens >= effective_window - BLOCKED_HEADROOM → can't even submit; force compact
//   tokens >= effective_window - COMPACT_HEADROOM → auto-compact triggers (this turn)
//   tokens >= compact_threshold - 20_000          → UI warning, no action
const COMPACT_HEADROOM: usize = 13_000;
const BLOCKED_HEADROOM: usize = 3_000;
/// Cap on how much max_output_tokens is subtracted from the window to compute
/// the effective context. CC 177's `In4 = 2e4` — even models with 64k/128k output
/// only reserve 20k for this calculation.
const OUTPUT_HEADROOM_CAP: usize = 20_000;
// warn = compact_threshold - 20_000 (matches v177's `_ - 2e4` in ZB7);
// computed inline rather than as a const since it depends on the runtime
// compact threshold (which itself shifts with the pct override).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompactLevel {
    Ok,
    /// Context is approaching the threshold — good time to speculatively
    /// precompute a summary in the background. Fires at ~80% of compact
    /// threshold (mirrors CC 2.1.144's `Ae7` precompute buffer).
    Precompute,
    Warn,
    Compact,
    Blocked,
}

pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    let content_chars: usize = messages
        .iter()
        .map(|m| {
            m.parts
                .iter()
                .map(|part| part.approx_text_len())
                .sum::<usize>()
        })
        .sum();
    let base = content_chars / CHARS_PER_TOKEN;
    let est = base * OVERHEAD_MULTIPLIER_NUM / OVERHEAD_MULTIPLIER_DEN;
    trace!(target: "jfc::compact", message_count = messages.len(), content_chars, base, est, "estimate_tokens (with overhead)");
    est
}

/// Read the effective compaction percentage threshold (1–100). Priority order:
///
/// 1. `JFC_AUTOCOMPACT_PCT_OVERRIDE` env var (CI/integration-test knob).
/// 2. `auto_compact_threshold_pct` from config (default 85 — the legacy
///    hardcoded value). A config value equal to the default (85) is treated as
///    "user did not override" so callers fall through to the fixed-headroom
///    calculation that has always been used.  This preserves the existing
///    compact behaviour unchanged for users who haven't set the field.
fn pct_override() -> Option<f64> {
    // Env var takes highest priority (mirrors CC CLAUDE_AUTOCOMPACT_PCT_OVERRIDE).
    if let Some(v) = std::env::var("JFC_AUTOCOMPACT_PCT_OVERRIDE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|p| (0.0..=100.0).contains(p) && *p > 0.0)
    {
        trace!(target: "jfc::compact", pct = v, "JFC_AUTOCOMPACT_PCT_OVERRIDE active");
        return Some(v);
    }
    // Config-level threshold. Values of exactly 85 are the default and treated
    // as "not set", so the existing fixed-headroom logic is unchanged.
    let cfg_pct = crate::config::load_arc().auto_compact_threshold_pct;
    if cfg_pct != 85 && cfg_pct > 0 {
        let pct = f64::from(cfg_pct);
        trace!(target: "jfc::compact", pct, "auto_compact_threshold_pct from config active");
        return Some(pct);
    }
    None
}

pub fn blocked_override() -> Option<usize> {
    let v = std::env::var("JFC_BLOCKING_LIMIT_OVERRIDE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0);
    if let Some(limit) = v {
        trace!(target: "jfc::compact", limit, "JFC_BLOCKING_LIMIT_OVERRIDE active");
    }
    v
}

pub fn auto_compact_disabled() -> bool {
    // Env vars take priority (both legacy spellings honored).
    let via_env = matches!(
        std::env::var("JFC_DISABLE_COMPACT").as_deref(),
        Ok("1") | Ok("true")
    ) || matches!(
        std::env::var("JFC_DISABLE_AUTO_COMPACT").as_deref(),
        Ok("1") | Ok("true")
    );
    if via_env {
        trace!(target: "jfc::compact", "auto-compact disabled via env var");
        return true;
    }
    // Then check config (autoCompactEnabled / auto_compact_enabled).
    let via_config = !crate::config::load_arc().auto_compact_enabled;
    if via_config {
        trace!(target: "jfc::compact", "auto-compact disabled via config auto_compact_enabled=false");
    }
    via_config
}

/// Compute the effective context window after reserving output headroom.
/// Mirrors v177's `Q9H` (cli.js:496582): `window - min(maxOutputTokens, In4)`.
///
/// This is the window used for all threshold calculations — it accounts for
/// the tokens the model needs for its response.
pub fn effective_window(window: usize, max_output_tokens: Option<usize>) -> usize {
    // Config-level window override (valid range: 100_000–1_000_000).
    let config_window = crate::config::load_arc()
        .auto_compact_window
        .map(|w| w as usize)
        .filter(|&w| (100_000..=1_000_000).contains(&w))
        .unwrap_or(window);

    // v177: subtract min(max_output_tokens, OUTPUT_HEADROOM_CAP) from the window.
    // For models with 64k/128k output, we still only reserve 20k — that's the cap.
    // If max_output_tokens is unknown (None), assume modern models with large
    // output limits and use the full cap. This matches CC 177's default behavior.
    let output_reserve = max_output_tokens
        .map(|v| v.min(OUTPUT_HEADROOM_CAP))
        .unwrap_or(OUTPUT_HEADROOM_CAP);

    config_window.saturating_sub(output_reserve)
}

/// Compute the absolute token offset at which auto-compaction triggers.
/// Mirrors v177's `mu8` (cli.js:496636): `effective_window - 13000`.
///
/// If `autoCompactWindow` is set in the config (and falls within the valid
/// range 100_000–1_000_000), that value is used instead of the caller-supplied
/// `window` argument for the headroom calculation.
pub fn compact_threshold(window: usize) -> usize {
    compact_threshold_with_output(window, None)
}

/// Compute the compact threshold with explicit max_output_tokens.
/// Use this when you have access to the model's output limit.
pub fn compact_threshold_with_output(window: usize, max_output_tokens: Option<usize>) -> usize {
    let eff_window = effective_window(window, max_output_tokens);
    let base = eff_window.saturating_sub(COMPACT_HEADROOM);
    if let Some(pct) = pct_override() {
        let from_pct = ((eff_window as f64) * pct / 100.0).floor() as usize;
        let threshold = from_pct.min(base);
        debug!(target: "jfc::compact", window, eff_window, pct, from_pct, base, threshold, "compact_threshold (pct override)");
        return threshold;
    }
    base
}

/// Mirrors v177's `ZB7` (cli.js:397183-397203).
pub fn compact_level(tokens: usize, window: usize) -> CompactLevel {
    compact_level_with_output(tokens, window, None)
}

/// Compute compact level with explicit max_output_tokens for accurate thresholds.
pub fn compact_level_with_output(
    tokens: usize,
    window: usize,
    max_output_tokens: Option<usize>,
) -> CompactLevel {
    let eff_window = effective_window(window, max_output_tokens);
    let compact = compact_threshold_with_output(window, max_output_tokens);
    let warn = compact.saturating_sub(20_000);
    let blocked = blocked_override().unwrap_or_else(|| eff_window.saturating_sub(BLOCKED_HEADROOM));
    // Precompute threshold: 80% of the compact threshold. When context
    // hits this level, the system could start a speculative compact in
    // the background so it's ready if the session continues growing.
    let precompute = (compact as f64 * 0.8) as usize;

    let level = if tokens >= blocked {
        CompactLevel::Blocked
    } else if !auto_compact_disabled() && tokens >= compact {
        CompactLevel::Compact
    } else if tokens >= warn {
        CompactLevel::Warn
    } else if !auto_compact_disabled() && tokens >= precompute {
        CompactLevel::Precompute
    } else {
        CompactLevel::Ok
    };

    debug!(
        target: "jfc::compact",
        tokens, window, eff_window, compact_threshold = compact, warn_threshold = warn,
        blocked_threshold = blocked, ?level,
        "compact_level evaluated"
    );
    level
}

/// Decide whether compaction should fire for a context of `current_tokens`.
///
/// Callers should pass the *calibrated* context size — i.e. `tool_ctx
/// .approx_tokens`, which `recompute_token_estimate` keeps in sync with the
/// last API-reported usage (mirroring v177's `tokenCountWithEstimation`:
/// API anchor + rough estimate of messages added after the anchor).
///
/// We do NOT recompute `estimate_tokens(messages)` here. The raw estimator
/// over-counts tool outputs because it sums their full byte length, while
/// the wire format truncates each tool result to `MAX_TOOL_RESULT_CHARS`.
/// Triggering off the over-estimate caused compaction to fire on every
/// turn that contained a large Read/Bash output, even when the API saw a
/// context with plenty of headroom — the "randomly starts compacting"
/// symptom.
pub fn should_compact(current_tokens: usize, max_context_tokens: usize) -> bool {
    should_compact_with_output(current_tokens, max_context_tokens, None)
}

/// Decide whether compaction should fire, with explicit max_output_tokens.
pub fn should_compact_with_output(
    current_tokens: usize,
    max_context_tokens: usize,
    max_output_tokens: Option<usize>,
) -> bool {
    let level = compact_level_with_output(current_tokens, max_context_tokens, max_output_tokens);
    let should = matches!(level, CompactLevel::Compact | CompactLevel::Blocked);
    debug!(
        target: "jfc::compact",
        current_tokens, max_context_tokens, ?level, should,
        "should_compact check"
    );
    should
}

mod engine;
pub mod microcompact;

pub use engine::{CompactProgressCb, CompactResult, compact};
pub use microcompact::{microcompact, microcompact_if_helpful, microcompact_savings};
