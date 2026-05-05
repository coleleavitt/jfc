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
use futures::StreamExt;
use tracing::{debug, info, instrument, trace, warn};

const CHARS_PER_TOKEN: usize = 4;
/// Multiplier applied to the char-based estimate to account for wire overhead
/// (system prompt, tool definitions, JSON framing, role markers) that is not
/// visible in message text. Empirical measurement: API reports ~1.4–1.5× more
/// tokens than naive char_count/4 on tool-heavy conversations.
const OVERHEAD_MULTIPLIER_NUM: usize = 3;
const OVERHEAD_MULTIPLIER_DEN: usize = 2; // 3/2 = 1.5×
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

fn estimate_group_tokens(group: &ConversationGroup) -> usize {
    let tokens = estimate_tokens(&group.messages);
    trace!(target: "jfc::compact", messages_in_group = group.messages.len(), tokens, "estimate_group_tokens");
    tokens
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

fn blocked_override() -> Option<usize> {
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
    let disabled = matches!(
        std::env::var("JFC_DISABLE_COMPACT").as_deref(),
        Ok("1") | Ok("true")
    ) || matches!(
        std::env::var("JFC_DISABLE_AUTO_COMPACT").as_deref(),
        Ok("1") | Ok("true")
    );
    if disabled {
        trace!(target: "jfc::compact", "auto-compact disabled via env var");
    }
    disabled
}

/// Compute the absolute token offset at which auto-compaction triggers.
/// Mirrors v126 `gG6` (cli.js:397177-397182).
pub fn compact_threshold(window: usize) -> usize {
    let base = window.saturating_sub(COMPACT_HEADROOM);
    if let Some(pct) = pct_override() {
        let from_pct = ((window as f64) * pct / 100.0).floor() as usize;
        let threshold = from_pct.min(base);
        debug!(target: "jfc::compact", window, pct, from_pct, base, threshold, "compact_threshold (pct override)");
        return threshold;
    }
    base
}

/// Mirrors v126 `ZB7` (cli.js:397183-397203).
pub fn compact_level(tokens: usize, window: usize) -> CompactLevel {
    let compact = compact_threshold(window);
    let warn = compact.saturating_sub(20_000);
    let blocked = blocked_override().unwrap_or_else(|| window.saturating_sub(BLOCKED_HEADROOM));

    let level = if tokens >= blocked {
        CompactLevel::Blocked
    } else if !auto_compact_disabled() && tokens >= compact {
        CompactLevel::Compact
    } else if tokens >= warn {
        CompactLevel::Warn
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

pub fn should_compact(messages: &[ChatMessage], max_context_tokens: usize) -> bool {
    let est = estimate_tokens(messages);
    let level = compact_level(est, max_context_tokens);
    let should = matches!(level, CompactLevel::Compact | CompactLevel::Blocked);
    debug!(
        target: "jfc::compact",
        est, max_context_tokens, ?level, should,
        "should_compact check"
    );
    should
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
    debug!(
        target: "jfc::compact",
        total_messages = messages.len(),
        group_count = groups.len(),
        "split_into_groups"
    );
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
        let step = (current_split / 2).max(1);
        debug!(
            target: "jfc::compact",
            current_split, step,
            "token_gap_step: no gap info, falling back to halving"
        );
        return step;
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
    let step = step.max(1);
    debug!(
        target: "jfc::compact",
        gap, current_split, freed, step,
        "token_gap_step: computed step from token gap"
    );
    step
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

/// Try `provider.complete()` first; if unsupported, fall back to streaming
/// and collect the full response. This handles providers like OpenWebUI/LiteLLM
/// that only support streaming endpoints.
async fn complete_or_stream(
    provider: &dyn Provider,
    messages: Vec<ProviderMessage>,
    options: &StreamOptions,
) -> Result<crate::provider::CompletionResponse, anyhow::Error> {
    match provider.complete(messages.clone(), options).await {
        Ok(resp) => Ok(resp),
        Err(e) => {
            let err_msg = e.to_string().to_lowercase();
            if err_msg.contains("not support") || err_msg.contains("unsupported") {
                info!(
                    target: "jfc::compact",
                    "provider.complete() unsupported — falling back to streaming"
                );
                let mut stream = provider.stream(messages, options).await?;
                let mut collected = String::new();
                while let Some(event) = stream.next().await {
                    match event {
                        Ok(crate::provider::StreamEvent::TextDelta { delta, .. }) => {
                            collected.push_str(&delta);
                        }
                        Ok(crate::provider::StreamEvent::Done { .. }) => break,
                        Ok(crate::provider::StreamEvent::Error { message }) => {
                            return Err(anyhow::anyhow!("{}", message));
                        }
                        Ok(_) => {} // skip usage, thinking, etc.
                        Err(stream_err) => {
                            return Err(anyhow::anyhow!("{}", stream_err));
                        }
                    }
                }
                debug!(
                    target: "jfc::compact",
                    collected_len = collected.len(),
                    "streaming fallback collected response"
                );
                Ok(crate::provider::CompletionResponse {
                    content: collected,
                    usage: Default::default(),
                })
            } else {
                Err(e)
            }
        }
    }
}

#[instrument(
    target = "jfc::compact",
    skip(messages, provider, options, tool_ctx),
    fields(
        message_count = messages.len(),
        window,
        model = %options.model,
        rapid_refill_count = tool_ctx.rapid_refill_count,
        total_user_turns = tool_ctx.total_user_turns,
        last_compact_turn = tool_ctx.last_compact_turn,
    )
)]
pub async fn compact(
    messages: &[ChatMessage],
    provider: &dyn Provider,
    options: &StreamOptions,
    tool_ctx: &mut ToolContext,
    window: usize,
) -> CompactResult {
    if tool_ctx.rapid_refill_count >= CIRCUIT_BREAKER_LIMIT {
        warn!(
            target: "jfc::compact",
            rapid_refill_count = tool_ctx.rapid_refill_count,
            limit = CIRCUIT_BREAKER_LIMIT,
            "circuit breaker tripped — aborting compaction"
        );
        return CompactResult::CircuitBreakerTripped;
    }

    let groups = split_into_groups(messages);
    if groups.len() < 2 {
        info!(
            target: "jfc::compact",
            group_count = groups.len(),
            "too few groups for compaction"
        );
        return CompactResult::TooFewGroups;
    }

    let pre_tokens = estimate_tokens(messages);
    let group_tokens: Vec<usize> = groups.iter().map(estimate_group_tokens).collect();
    let total_groups = groups.len();
    let mut preserve_count: usize = 1;
    let mut attempt: u32 = 0;
    let mut strip_media = false;
    // Sourced from API error bodies: `actualTokens - limitTokens` for
    // prompt_too_long / 529 responses. Mirrors v126 `ol$` → `To1` in
    // cli.2.1.126.js so `token_gap_step` can size the next compaction
    // step instead of falling back to halving.
    let mut last_token_gap: Option<usize> = None;

    info!(
        target: "jfc::compact",
        pre_tokens, total_groups,
        group_token_sizes = ?group_tokens,
        model = %options.model,
        "starting compaction loop"
    );

    loop {
        attempt += 1;
        if attempt > MAX_ATTEMPTS {
            warn!(
                target: "jfc::compact",
                attempts = attempt - 1,
                "exhausted max attempts"
            );
            return CompactResult::Exhausted {
                attempts: attempt - 1,
            };
        }
        if preserve_count >= total_groups {
            warn!(
                target: "jfc::compact",
                preserve_count, total_groups, attempt,
                "preserve_count >= total_groups — nothing left to summarize"
            );
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

        let summarize_tokens: usize = group_tokens[..split_point].iter().sum();
        let preserve_tokens: usize = group_tokens[split_point..].iter().sum();

        info!(
            target: "jfc::compact",
            attempt, split_point, preserve_count, total_groups,
            summarize_msg_count = to_summarize.len(),
            preserve_msg_count = to_preserve.len(),
            summarize_tokens, preserve_tokens,
            strip_media, last_token_gap = ?last_token_gap,
            "compaction attempt"
        );

        let summary_text = build_summary_text(&to_summarize, strip_media);
        debug!(
            target: "jfc::compact",
            summary_text_len = summary_text.len(),
            summary_text_tokens = summary_text.len() / CHARS_PER_TOKEN,
            "built summary request text"
        );

        let compact_messages = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(summary_text)],
        }];

        let compact_options = StreamOptions::new(options.model.clone())
            .system(COMPACTION_SYSTEM_PROMPT.to_owned())
            .max_tokens(20_000);

        debug!(
            target: "jfc::compact",
            model = %compact_options.model,
            max_tokens = 20_000,
            "sending compaction request to provider"
        );

        match complete_or_stream(provider, compact_messages, &compact_options).await {
            Ok(response) => {
                debug!(
                    target: "jfc::compact",
                    response_len = response.content.len(),
                    response_preview = %&response.content[..response.content.len().min(200)],
                    "received compaction response"
                );

                if !is_usable_summary(&response.content) {
                    warn!(
                        target: "jfc::compact",
                        len = response.content.len(),
                        response_preview = %&response.content[..response.content.len().min(300)],
                        "summary response empty or itself an error — retrying with larger preserve"
                    );
                    let step =
                        token_gap_step(last_token_gap, &group_tokens, split_point);
                    preserve_count =
                        (preserve_count + step).min(total_groups - 1);
                    continue;
                }
                let formatted = format_compact_summary(&response.content);
                debug!(
                    target: "jfc::compact",
                    formatted_len = formatted.len(),
                    "formatted compact summary"
                );
                let summary_msg = ChatMessage::compact_boundary(&formatted, pre_tokens);
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
                if post_tokens >= blocked {
                    if preserve_count > 0 {
                        info!(
                            target: "jfc::compact",
                            post_tokens, blocked, preserve_count,
                            "post-compact still blocked — dropping a preserved group and retrying"
                        );
                        preserve_count -= 1;
                        strip_media = true;
                        last_token_gap = Some(post_tokens.saturating_sub(blocked));
                        continue;
                    }
                    // Returning Success while still over `blocked` would let
                    // the caller immediately resubmit and re-trigger compaction
                    // forever. Surface Exhausted instead.
                    warn!(
                        target: "jfc::compact",
                        post_tokens, blocked, attempts = attempt,
                        "post-compact still blocked with no groups left — returning Exhausted"
                    );
                    return CompactResult::Exhausted { attempts: attempt };
                }

                let user_turns_since = count_user_turns_since_last_compact(&compacted);
                if user_turns_since <= THRASH_TURN_WINDOW {
                    tool_ctx.rapid_refill_count += 1;
                    info!(
                        target: "jfc::compact",
                        user_turns_since, thrash_window = THRASH_TURN_WINDOW,
                        rapid_refill_count = tool_ctx.rapid_refill_count,
                        "rapid refill detected — incrementing circuit breaker"
                    );
                } else {
                    tool_ctx.rapid_refill_count = 0;
                    debug!(
                        target: "jfc::compact",
                        user_turns_since, thrash_window = THRASH_TURN_WINDOW,
                        "no rapid refill — resetting circuit breaker"
                    );
                }

                tool_ctx.approx_tokens = post_tokens;
                tool_ctx.last_compact_turn = tool_ctx.total_user_turns;
                tool_ctx.read_cache.clear();

                info!(
                    target: "jfc::compact",
                    pre_tokens, post_tokens,
                    saved = pre_tokens.saturating_sub(post_tokens),
                    compacted_message_count = compacted.len(),
                    attempts = attempt,
                    model = %options.model,
                    "compaction succeeded"
                );

                return CompactResult::Success {
                    messages: compacted,
                    pre_tokens,
                    post_tokens,
                };
            }
            Err(e) => {
                let err_msg = e.to_string().to_lowercase();
                warn!(
                    target: "jfc::compact",
                    attempt, error = %e,
                    "compaction API call failed"
                );

                if err_msg.contains("too_large") || err_msg.contains("media") {
                    if !strip_media {
                        info!(
                            target: "jfc::compact",
                            "summary call rejected by media size — retrying with strip_media"
                        );
                        strip_media = true;
                        continue;
                    }
                    warn!(
                        target: "jfc::compact",
                        attempts = attempt,
                        "media too large even after strip — returning Unsupported"
                    );
                    return CompactResult::Unsupported;
                }

                if err_msg.contains("too_long")
                    || err_msg.contains("token")
                    || err_msg.contains("context")
                {
                    let parsed_actual = parse_actual_tokens_from_error(&err_msg);
                    let parsed_gap = parse_token_gap_from_error(&err_msg);
                    debug!(
                        target: "jfc::compact",
                        ?parsed_actual, ?parsed_gap, ?last_token_gap,
                        error_snippet = %&err_msg[..err_msg.len().min(200)],
                        "detected token/context limit error"
                    );
                    // Update approx_tokens with the real count from the API
                    // so the status bar and compaction gate show accurate data.
                    if let Some(actual) = parsed_actual {
                        tool_ctx.approx_tokens = actual;
                        info!(
                            target: "jfc::compact",
                            actual,
                            "calibrated approx_tokens from API error"
                        );
                    }
                    last_token_gap = parsed_gap.or(last_token_gap);
                    let step = token_gap_step(last_token_gap, &group_tokens, split_point);
                    preserve_count = (preserve_count + step).min(total_groups - 1);
                    continue;
                }

                if err_msg.contains("not support") {
                    info!(
                        target: "jfc::compact",
                        error = %e,
                        "provider does not support compaction"
                    );
                    return CompactResult::Unsupported;
                }

                debug!(
                    target: "jfc::compact",
                    "unrecognized error — increasing preserve_count"
                );
                let step = token_gap_step(last_token_gap, &group_tokens, split_point);
                preserve_count = (preserve_count + step).min(total_groups - 1);
            }
        }
    }
}

/// Reject summary outputs that are unusable as a compact boundary:
/// empty, whitespace-only, or themselves an API error string the LLM
/// echoed back. Mirrors v126's `!V || Od(V)` check before accepting a
/// summary.
fn is_usable_summary(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        debug!(target: "jfc::compact", "is_usable_summary: rejected — empty/whitespace");
        return false;
    }
    let lower = trimmed.to_lowercase();
    const ERROR_MARKERS: &[&str] = &[
        "prompt is too long",
        "input is too long",
        "context window",
        "exceeds context",
        "rate_limit",
        "rate limit exceeded",
        "overloaded_error",
        "request_too_large",
    ];
    let has_error_marker = ERROR_MARKERS.iter().any(|m| lower.contains(m));
    if has_error_marker {
        debug!(
            target: "jfc::compact",
            text_preview = %&trimmed[..trimmed.len().min(150)],
            "is_usable_summary: rejected — contains error marker"
        );
    } else {
        trace!(
            target: "jfc::compact",
            text_len = trimmed.len(),
            "is_usable_summary: accepted"
        );
    }
    !has_error_marker
}

/// Extract `actualTokens - limitTokens` from an Anthropic error body.
///
/// Mirrors `ol$` in cli.2.1.126.js — recognises:
///   "prompt is too long: 410234 tokens > 200000 maximum"
///   "input length and `max_tokens` exceed context limit: 350000 + 4096 > 200000"
fn parse_token_gap_from_error(err_msg: &str) -> Option<usize> {
    let msg = err_msg.to_lowercase();
    let bytes = msg.as_bytes();
    let mut i = 0;
    let mut nums: Vec<usize> = Vec::new();
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if let Ok(n) = msg[start..i].parse::<usize>() {
                nums.push(n);
            }
        } else {
            i += 1;
        }
    }
    if nums.len() >= 2 {
        let &actual = nums.first()?;
        let &limit = nums.last()?;
        if actual > limit {
            let gap = actual - limit;
            debug!(
                target: "jfc::compact",
                actual, limit, gap,
                "parsed token gap from error"
            );
            return Some(gap);
        }
    }
    trace!(target: "jfc::compact", "could not parse token gap from error");
    None
}

/// Extract the actual token count from an API error like
/// "prompt is too long: 1456365 tokens > 1000000 maximum"
fn parse_actual_tokens_from_error(err_msg: &str) -> Option<usize> {
    let msg = err_msg.to_lowercase();
    let bytes = msg.as_bytes();
    let mut i = 0;
    let mut nums: Vec<usize> = Vec::new();
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if let Ok(n) = msg[start..i].parse::<usize>() {
                if n > 10_000 {
                    nums.push(n);
                }
            }
        } else {
            i += 1;
        }
    }
    nums.first().copied()
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
    trace!(
        target: "jfc::compact",
        count, total_messages = messages.len(),
        "count_user_turns_since_last_compact"
    );
    count
}

fn build_summary_text(messages: &[ChatMessage], strip_media: bool) -> String {
    debug!(
        target: "jfc::compact",
        message_count = messages.len(), strip_media,
        "building summary text"
    );
    let mut text = String::from(
        "Here is the conversation to summarize:\n\n",
    );

    for msg in messages {
        let role = if msg.role_is_user() {
            "H" // Human
        } else {
            "A" // Assistant
        };
        text.push_str(&format!("[{}]\n", role));
        for part in &msg.parts {
            if strip_media {
                text.push_str(&part.text_only());
            } else {
                text.push_str(&part.to_display_string());
            }
            text.push('\n');
        }
        text.push('\n');
    }

    trace!(
        target: "jfc::compact",
        text_len = text.len(),
        text_tokens_approx = text.len() / CHARS_PER_TOKEN,
        "summary text built"
    );
    text
}

// Modeled after v126's `getCompactPrompt` (claude-code prompt.ts:61-143).
// The 9-section structure + analysis scratchpad is what claude-code uses to
// keep summaries actionable. The <analysis> block improves quality but gets
// stripped from the final summary (it's a drafting scratchpad).
const COMPACTION_SYSTEM_PROMPT: &str = "\
CRITICAL: Respond with TEXT ONLY. Do NOT call any tools.

- Do NOT use Read, Bash, Grep, Glob, Edit, Write, or ANY other tool.
- You already have all the context you need in the conversation above.
- Tool calls will be REJECTED and will waste your only turn — you will fail the task.
- Your entire response must be plain text: an <analysis> block followed by a <summary> block.

Your task is to create a detailed summary of the conversation so far, paying close \
attention to the user's explicit requests and your previous actions. This summary \
should be thorough in capturing technical details, code patterns, and architectural \
decisions that would be essential for continuing development work without losing context.

Before providing your final summary, wrap your analysis in <analysis> tags to organize \
your thoughts and ensure you've covered all necessary points. In your analysis process:

1. Chronologically analyze each message and section of the conversation. For each section thoroughly identify:
   - The user's explicit requests and intents
   - Your approach to addressing the user's requests
   - Key decisions, technical concepts and code patterns
   - Specific details like file names, full code snippets, function signatures, file edits
   - Errors that you ran into and how you fixed them
   - Pay special attention to specific user feedback, especially if the user told you to do something differently.
2. Double-check for technical accuracy and completeness.

Your summary should include the following sections:

1. Primary Request and Intent: Capture all of the user's explicit requests and intents in detail
2. Key Technical Concepts: List all important technical concepts, technologies, and frameworks discussed.
3. Files and Code Sections: Enumerate specific files and code sections examined, modified, or created. Include full code snippets where applicable.
4. Errors and Fixes: List all errors encountered and how they were fixed. Include user feedback.
5. Problem Solving: Document problems solved and any ongoing troubleshooting efforts.
6. All User Messages: List ALL user messages that are not tool results (critical for understanding changing intent).
7. Pending Tasks: Outline any pending tasks explicitly asked for.
8. Current Work: Describe precisely what was being worked on immediately before this summary request.
9. Optional Next Step: The single most likely next action, with direct quotes from the most recent conversation.

REMINDER: Do NOT call any tools. Respond with plain text only — \
an <analysis> block followed by a <summary> block.";

/// Strip `<analysis>...</analysis>` and extract content from `<summary>...</summary>`.
/// Mirrors v126's `formatCompactSummary()` in prompt.ts:293-313.
fn format_compact_summary(raw: &str) -> String {
    let mut result = raw.to_string();

    // Strip analysis section — it's a drafting scratchpad
    if let Some(start) = result.find("<analysis>") {
        if let Some(end) = result.find("</analysis>") {
            let end_tag_end = end + "</analysis>".len();
            let analysis_len = end_tag_end - start;
            debug!(
                target: "jfc::compact",
                analysis_len,
                "stripped <analysis> block from summary"
            );
            result = format!("{}{}", &result[..start], &result[end_tag_end..]);
        }
    }

    // Extract summary content
    if let Some(start) = result.find("<summary>") {
        if let Some(end) = result.find("</summary>") {
            let content_start = start + "<summary>".len();
            let content = result[content_start..end].trim();
            debug!(
                target: "jfc::compact",
                summary_content_len = content.len(),
                "extracted <summary> block"
            );
            result = format!("Summary:\n{content}");
        }
    }

    // Clean up extra whitespace
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    result.trim().to_string()
}

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

    #[test]
    fn parse_token_gap_recognises_anthropic_too_long_format() {
        let msg = "prompt is too long: 410234 tokens > 200000 maximum";
        assert_eq!(parse_token_gap_from_error(msg), Some(210_234));
    }

    #[test]
    fn parse_token_gap_recognises_input_plus_max_tokens_format() {
        let msg = "input length and `max_tokens` exceed context limit: \
                   350000 + 4096 > 200000 tokens";
        // gap = first integer (actual=350000) - last integer (limit=200000).
        assert_eq!(parse_token_gap_from_error(msg), Some(150_000));
    }

    #[test]
    fn parse_token_gap_returns_none_when_no_overflow() {
        assert_eq!(
            parse_token_gap_from_error("ok: 100 tokens of 200000"),
            None
        );
        assert_eq!(
            parse_token_gap_from_error("connection reset"),
            None
        );
    }

    #[test]
    fn is_usable_summary_rejects_empty_or_whitespace() {
        assert!(!is_usable_summary(""));
        assert!(!is_usable_summary("   \n\t  "));
    }

    #[test]
    fn is_usable_summary_rejects_echoed_api_errors() {
        assert!(!is_usable_summary(
            "prompt is too long: 410234 tokens > 200000 maximum"
        ));
        assert!(!is_usable_summary(
            "Sorry, this exceeds context window."
        ));
        assert!(!is_usable_summary("Rate limit exceeded for tier 4"));
    }

    #[test]
    fn is_usable_summary_accepts_real_summary() {
        let real = "Session summary:\n- User asked about compaction.\n- \
                    Implemented post-compact validation.";
        assert!(is_usable_summary(real));
    }
}
