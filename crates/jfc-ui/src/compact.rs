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

use crate::context::ToolContext;
use crate::provider::{Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions};
use crate::types::ChatMessage;

const CHARS_PER_TOKEN: usize = 4;
const MAX_ATTEMPTS: u32 = 8;
const CIRCUIT_BREAKER_LIMIT: u32 = 3;
/// If context refills within this many user turns after a compact, it counts as thrash.
const THRASH_TURN_WINDOW: u32 = 2;

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
    Warn,
    Compact,
    Blocked,
}

pub fn estimate_tokens(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .map(|m| {
            let content_chars: usize = m.parts.iter().map(|p| p.approx_text_len()).sum();
            content_chars / CHARS_PER_TOKEN
        })
        .sum()
}

fn estimate_group_tokens(group: &ConversationGroup) -> usize {
    estimate_tokens(&group.messages)
}

/// Read `JFC_AUTOCOMPACT_PCT_OVERRIDE` (1-100) once per call. v126 has the
/// same env knob (`CLAUDE_AUTOCOMPACT_PCT_OVERRIDE`) used by integration tests
/// to force compaction at non-default thresholds without rebuilding.
fn pct_override() -> Option<f64> {
    std::env::var("JFC_AUTOCOMPACT_PCT_OVERRIDE")
        .ok()
        .and_then(|s| s.parse::<f64>().ok())
        .filter(|p| (0.0..=100.0).contains(p) && *p > 0.0)
}

fn blocked_override() -> Option<usize> {
    std::env::var("JFC_BLOCKING_LIMIT_OVERRIDE")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|n| *n > 0)
}

pub fn auto_compact_disabled() -> bool {
    matches!(
        std::env::var("JFC_DISABLE_COMPACT").as_deref(),
        Ok("1") | Ok("true")
    ) || matches!(
        std::env::var("JFC_DISABLE_AUTO_COMPACT").as_deref(),
        Ok("1") | Ok("true")
    )
}

/// Compute the absolute token offset at which auto-compaction triggers.
/// Mirrors v126 `gG6` (cli.js:397177-397182).
pub fn compact_threshold(window: usize) -> usize {
    let base = window.saturating_sub(COMPACT_HEADROOM);
    if let Some(pct) = pct_override() {
        let from_pct = ((window as f64) * pct / 100.0).floor() as usize;
        return from_pct.min(base);
    }
    base
}

/// Mirrors v126 `ZB7` (cli.js:397183-397203).
pub fn compact_level(tokens: usize, window: usize) -> CompactLevel {
    let compact = compact_threshold(window);
    let warn = compact.saturating_sub(20_000);
    let blocked = blocked_override().unwrap_or_else(|| window.saturating_sub(BLOCKED_HEADROOM));

    if tokens >= blocked {
        CompactLevel::Blocked
    } else if !auto_compact_disabled() && tokens >= compact {
        CompactLevel::Compact
    } else if tokens >= warn {
        CompactLevel::Warn
    } else {
        CompactLevel::Ok
    }
}

pub fn should_compact(messages: &[ChatMessage], max_context_tokens: usize) -> bool {
    let est = estimate_tokens(messages);
    matches!(
        compact_level(est, max_context_tokens),
        CompactLevel::Compact | CompactLevel::Blocked
    )
}

#[derive(Debug, Clone)]
struct ConversationGroup {
    messages: Vec<ChatMessage>,
}

fn split_into_groups(messages: &[ChatMessage]) -> Vec<ConversationGroup> {
    let mut groups: Vec<ConversationGroup> = Vec::new();
    let mut current = Vec::new();

    for msg in messages {
        if msg.role_is_user() && !current.is_empty() {
            groups.push(ConversationGroup {
                messages: std::mem::take(&mut current),
            });
        }
        current.push(msg.clone());
    }
    if !current.is_empty() {
        groups.push(ConversationGroup { messages: current });
    }
    groups
}

/// Smart step calculator (mirrors CC `To1`).
///
/// Given a token gap (how many tokens need to be freed), walk groups backward
/// from the current split point, accumulating each group's tokens until we've
/// freed enough. Returns the number of additional groups to preserve.
///
/// Falls back to exponential doubling when `token_gap` is None.
fn token_gap_step(token_gap: Option<usize>, group_tokens: &[usize], current_split: usize) -> usize {
    let Some(gap) = token_gap else {
        return (current_split / 2).max(1);
    };

    let mut freed: usize = 0;
    let mut step: usize = 0;
    for i in (0..current_split).rev() {
        if freed >= gap {
            break;
        }
        freed += group_tokens.get(i).copied().unwrap_or(0);
        step += 1;
    }
    step.max(1)
}

#[derive(Debug)]
pub enum CompactResult {
    Success {
        messages: Vec<ChatMessage>,
        pre_tokens: usize,
        post_tokens: usize,
    },
    TooFewGroups,
    CircuitBreakerTripped,
    Exhausted {
        attempts: u32,
    },
    Unsupported,
}

pub async fn compact(
    messages: &[ChatMessage],
    provider: &dyn Provider,
    options: &StreamOptions,
    tool_ctx: &mut ToolContext,
    window: usize,
) -> CompactResult {
    if tool_ctx.rapid_refill_count >= CIRCUIT_BREAKER_LIMIT {
        return CompactResult::CircuitBreakerTripped;
    }

    let groups = split_into_groups(messages);
    if groups.len() < 2 {
        return CompactResult::TooFewGroups;
    }

    let pre_tokens = estimate_tokens(messages);
    let group_tokens: Vec<usize> = groups.iter().map(estimate_group_tokens).collect();
    let total_groups = groups.len();
    let mut preserve_count: usize = 1;
    let mut attempt: u32 = 0;
    let mut strip_media = false;
    let last_token_gap: Option<usize> = None;

    loop {
        attempt += 1;
        if attempt > MAX_ATTEMPTS {
            return CompactResult::Exhausted {
                attempts: attempt - 1,
            };
        }
        if preserve_count >= total_groups {
            return CompactResult::Exhausted { attempts: attempt };
        }

        let split_point = total_groups - preserve_count;
        let to_summarize: Vec<ChatMessage> = groups[..split_point]
            .iter()
            .flat_map(|g| g.messages.clone())
            .collect();
        let to_preserve: Vec<ChatMessage> = groups[split_point..]
            .iter()
            .flat_map(|g| g.messages.clone())
            .collect();

        let summary_text = build_summary_text(&to_summarize, strip_media);

        let compact_messages = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(summary_text)],
        }];

        let compact_options = StreamOptions::new(options.model.clone())
            .system(COMPACTION_SYSTEM_PROMPT.to_owned())
            .max_tokens(4096);

        match provider.complete(compact_messages, &compact_options).await {
            Ok(response) => {
                let summary_msg = ChatMessage::compact_boundary(&response.content, pre_tokens);
                let mut compacted = vec![summary_msg];
                compacted.extend(to_preserve);

                let post_tokens = estimate_tokens(&compacted);

                // If the preserved groups still push us past the blocked
                // threshold, the summary itself didn't help — the recent
                // group's tool outputs are too big to keep verbatim. Drop
                // a preserved group and retry. Without this, a session
                // with a huge final assistant message (e.g. resumed from
                // a long agentic batch with multi-tens-of-KB Read outputs)
                // gets stuck in a compact-resubmit loop because each
                // pass produces a Success that's still over Blocked.
                let blocked = blocked_override()
                    .unwrap_or_else(|| window.saturating_sub(BLOCKED_HEADROOM));
                if post_tokens >= blocked && preserve_count > 0 {
                    tracing::info!(
                        target: "jfc::compact",
                        post_tokens, blocked, preserve_count,
                        "post-compact still blocked — dropping a preserved group and retrying"
                    );
                    preserve_count -= 1;
                    strip_media = true;
                    continue;
                }

                let user_turns_since = count_user_turns_since_last_compact(&compacted);
                if user_turns_since <= THRASH_TURN_WINDOW {
                    tool_ctx.rapid_refill_count += 1;
                } else {
                    tool_ctx.rapid_refill_count = 0;
                }

                tool_ctx.approx_tokens = post_tokens;
                tool_ctx.last_compact_turn = tool_ctx.total_user_turns;
                tool_ctx.read_cache.clear();

                return CompactResult::Success {
                    messages: compacted,
                    pre_tokens,
                    post_tokens,
                };
            }
            Err(e) => {
                let err_msg = e.to_string().to_lowercase();

                if err_msg.contains("too_large") || err_msg.contains("media") {
                    if !strip_media {
                        strip_media = true;
                        continue;
                    }
                }

                if err_msg.contains("too_long")
                    || err_msg.contains("token")
                    || err_msg.contains("context")
                {
                    let step = token_gap_step(last_token_gap, &group_tokens, split_point);
                    preserve_count = (preserve_count + step).min(total_groups - 1);
                    continue;
                }

                if err_msg.contains("not support") {
                    return CompactResult::Unsupported;
                }

                let step = token_gap_step(last_token_gap, &group_tokens, split_point);
                preserve_count = (preserve_count + step).min(total_groups - 1);
            }
        }
    }
}

fn count_user_turns_since_last_compact(messages: &[ChatMessage]) -> u32 {
    let mut count = 0u32;
    for msg in messages.iter().rev() {
        if msg.is_compact_boundary() {
            break;
        }
        if msg.role_is_user() {
            count += 1;
        }
    }
    count
}

fn build_summary_text(messages: &[ChatMessage], strip_media: bool) -> String {
    let mut text = String::from(
        "Summarize the following conversation. Preserve all key decisions, \
         file paths, code changes, error messages, and context needed to \
         continue the work. Be concise but complete.\n\n",
    );

    for msg in messages {
        let role = if msg.role_is_user() {
            "User"
        } else {
            "Assistant"
        };
        text.push_str(&format!("--- {} ---\n", role));
        for part in &msg.parts {
            if strip_media {
                text.push_str(&part.text_only());
            } else {
                text.push_str(&part.to_display_string());
            }
            text.push('\n');
        }
    }

    text
}

// Modeled after v126's `getCompactPrompt` (claude-code prompt.ts:61-143),
// flattened to a single string. The 9-section structure is what claude-code
// uses to keep summaries actionable rather than narrative — the assistant
// resuming after compaction can re-orient from the headers alone.
const COMPACTION_SYSTEM_PROMPT: &str = "\
You are summarizing a conversation so it can be continued in a new session. \
CRITICAL: respond with TEXT ONLY — do not call any tools.

Produce a structured summary using these nine sections (omit any that don't \
apply, but keep the order). Be factual and dense; no pleasantries, no \
meta-commentary, no apologies.

1. Primary Request and Intent — what the user asked for, in their words.
2. Key Technical Concepts — frameworks, languages, patterns invoked.
3. Files and Code Sections — every file path touched or read, with the \
   relevant snippets or signatures (not the whole file).
4. Errors and Fixes — every error message + the resolution that worked.
5. Problem Solving — non-obvious decisions and the reasoning behind them.
6. All User Messages — chronological list of what the user has said.
7. Pending Tasks — anything still open or blocked.
8. Current Work — exactly where we are right now (last file, last function, \
   last failure).
9. Optional Next Step — the single most likely next action.";

#[cfg(test)]
mod level_tests {
    use super::*;

    const W: usize = 200_000;

    /// Serializes env-var access across the tests in this module — `cargo
    /// test` runs them in parallel by default, and `compact_level` reads
    /// process-global env state. Without this, `JFC_DISABLE_AUTO_COMPACT=1`
    /// set by one test races into another and flips its expected level.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        // Poisoning is fine here — a panic in one test shouldn't cascade
        // into "all subsequent tests fail because the mutex is poisoned."
        ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn clear_env() {
        for k in [
            "JFC_AUTOCOMPACT_PCT_OVERRIDE",
            "JFC_BLOCKING_LIMIT_OVERRIDE",
            "JFC_DISABLE_COMPACT",
            "JFC_DISABLE_AUTO_COMPACT",
        ] {
            // Safety: env mutation must be serialized; tests in this module
            // run sequentially because they all share these variables.
            unsafe {
                std::env::remove_var(k);
            }
        }
    }

    #[test]
    fn threshold_default_is_window_minus_13k_normal() {
        let _g = lock();
        clear_env();
        assert_eq!(compact_threshold(W), 187_000);
        assert_eq!(compact_threshold(1_000_000), 987_000);
    }

    #[test]
    fn levels_match_v126_at_each_boundary_normal() {
        let _g = lock();
        clear_env();
        // ok zone
        assert_eq!(compact_level(0, W), CompactLevel::Ok);
        assert_eq!(compact_level(166_999, W), CompactLevel::Ok);
        // warn at compact - 20K = 167K
        assert_eq!(compact_level(167_000, W), CompactLevel::Warn);
        assert_eq!(compact_level(186_999, W), CompactLevel::Warn);
        // compact at window - 13K = 187K
        assert_eq!(compact_level(187_000, W), CompactLevel::Compact);
        assert_eq!(compact_level(196_999, W), CompactLevel::Compact);
        // blocked at window - 3K = 197K
        assert_eq!(compact_level(197_000, W), CompactLevel::Blocked);
        assert_eq!(compact_level(W + 999, W), CompactLevel::Blocked);
    }

    #[test]
    fn pct_override_caps_threshold_below_default_normal() {
        let _g = lock();
        clear_env();
        // Safety: serial test, env reset above.
        unsafe {
            std::env::set_var("JFC_AUTOCOMPACT_PCT_OVERRIDE", "50");
        }
        // pct=50 → compact at 100K (min of 50% and the default base of 187K).
        // warn = compact - 20K = 80K; blocked = window - 3K = 197K. Verify each
        // band including the boundary just below compact (which falls in warn,
        // not ok, since lowering compact pulls warn down with it).
        assert_eq!(compact_threshold(W), 100_000);
        assert_eq!(compact_level(79_999, W), CompactLevel::Ok);
        assert_eq!(compact_level(80_000, W), CompactLevel::Warn);
        assert_eq!(compact_level(99_999, W), CompactLevel::Warn);
        assert_eq!(compact_level(100_000, W), CompactLevel::Compact);
        assert_eq!(compact_level(197_000, W), CompactLevel::Blocked);
        clear_env();
    }

    #[test]
    fn pct_override_clamped_to_default_when_higher_robust() {
        let _g = lock();
        clear_env();
        unsafe {
            std::env::set_var("JFC_AUTOCOMPACT_PCT_OVERRIDE", "99");
        }
        // 99% of 200K = 198K, but compact base = 187K → min wins.
        assert_eq!(compact_threshold(W), 187_000);
        clear_env();
    }

    #[test]
    fn disable_flag_skips_compact_level_robust() {
        let _g = lock();
        clear_env();
        unsafe {
            std::env::set_var("JFC_DISABLE_AUTO_COMPACT", "1");
        }
        // Even at 195K (would be compact), level should fall back to warn —
        // user disabled auto-compact, but blocked still applies (it's a hard
        // API constraint, not a preference).
        assert_eq!(compact_level(195_000, W), CompactLevel::Warn);
        // Blocked still applies though.
        assert_eq!(compact_level(198_000, W), CompactLevel::Blocked);
        clear_env();
    }

    #[test]
    fn small_window_saturates_without_underflow_robust() {
        let _g = lock();
        clear_env();
        // A 5K window can't even hold 13K headroom — saturating arithmetic
        // means the compact threshold collapses to 0 (everything is "compact"
        // territory). Importantly: no panic, no underflow.
        assert_eq!(compact_threshold(5_000), 0);
        assert_eq!(compact_level(1, 5_000), CompactLevel::Compact);
    }
}
