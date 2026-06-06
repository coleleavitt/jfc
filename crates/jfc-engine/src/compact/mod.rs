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

// v126 threshold algorithm — `gG6` / `ZB7` in cli.js (lines 397177-397203).
// The model's nominal window minus three headrooms gives three trigger levels.
// Using fixed token offsets (not percentages) keeps behavior consistent across
// 200K and 1M-context models — the buffer needed for the next user turn + the
// outgoing compaction summary doesn't scale with window size.
//
//   tokens >= window - BLOCKED_HEADROOM → can't even submit; force compact
//   tokens >= window - COMPACT_HEADROOM → auto-compact triggers (this turn)
//   tokens >= window - WARN_HEADROOM    → UI warning, no action
const COMPACT_HEADROOM: usize = 13_000;
const BLOCKED_HEADROOM: usize = 3_000;
// warn = compact_threshold - 20_000 (matches v126's `_ - 2e4` in ZB7);
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
    let base: usize = messages
        .iter()
        .map(|m| {
            let content_chars: usize = m.parts.iter().map(|p| p.approx_text_len()).sum();
            content_chars / CHARS_PER_TOKEN
        })
        .sum();
    let est = base * OVERHEAD_MULTIPLIER_NUM / OVERHEAD_MULTIPLIER_DEN;
    trace!(target: "jfc::compact", message_count = messages.len(), base, est, "estimate_tokens (with overhead)");
    est
}

/// Read `JFC_AUTOCOMPACT_PCT_OVERRIDE` (1-100) once per call. v126 has the
/// same env knob (`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`) used by integration tests
/// to force compaction at non-default thresholds without rebuilding.
fn pct_override() -> Option<f64> {
    let v = std::env::var("JFC_AUTOCOMPACT_PCT_OVERRIDE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|p| (0.0..=100.0).contains(p) && *p > 0.0);
    if let Some(pct) = v {
        trace!(target: "jfc::compact", pct, "JFC_AUTOCOMPACT_PCT_OVERRIDE active");
    }
    v
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

/// Compute the absolute token offset at which auto-compaction triggers.
/// Mirrors v126 `gG6` (cli.js:397177-397182).
///
/// If `autoCompactWindow` is set in the config (and falls within the valid
/// range 100_000–1_000_000), that value is used instead of the caller-supplied
/// `window` argument for the headroom calculation.
pub fn compact_threshold(window: usize) -> usize {
    // Config-level window override (valid range: 100_000–1_000_000).
    let effective_window = crate::config::load_arc()
        .auto_compact_window
        .map(|w| w as usize)
        .filter(|&w| (100_000..=1_000_000).contains(&w))
        .unwrap_or(window);

    let base = effective_window.saturating_sub(COMPACT_HEADROOM);
    if let Some(pct) = pct_override() {
        let from_pct = ((effective_window as f64) * pct / 100.0).floor() as usize;
        let threshold = from_pct.min(base);
        debug!(target: "jfc::compact", window, effective_window, pct, from_pct, base, threshold, "compact_threshold (pct override)");
        return threshold;
    }
    if effective_window != window {
        debug!(target: "jfc::compact", window, effective_window, base, "compact_threshold (config window override)");
    }
    base
}

/// Mirrors v126 `ZB7` (cli.js:397183-397203).
pub fn compact_level(tokens: usize, window: usize) -> CompactLevel {
    let compact = compact_threshold(window);
    let warn = compact.saturating_sub(20_000);
    let blocked = blocked_override().unwrap_or_else(|| window.saturating_sub(BLOCKED_HEADROOM));
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
        tokens, window, compact_threshold = compact, warn_threshold = warn,
        blocked_threshold = blocked, ?level,
        "compact_level evaluated"
    );
    level
}

/// Decide whether compaction should fire for a context of `current_tokens`.
///
/// Callers should pass the *calibrated* context size — i.e. `tool_ctx
/// .approx_tokens`, which `recompute_token_estimate` keeps in sync with the
/// last API-reported usage (mirroring v126's `tokenCountWithEstimation`:
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
    let level = compact_level(current_tokens, max_context_tokens);
    let should = matches!(level, CompactLevel::Compact | CompactLevel::Blocked);
    debug!(
        target: "jfc::compact",
        current_tokens, max_context_tokens, ?level, should,
        "should_compact check"
    );
    should
}

mod engine;

pub use engine::{CompactProgressCb, CompactResult, compact};
