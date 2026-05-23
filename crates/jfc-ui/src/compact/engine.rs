use crate::context::ToolContext;
use crate::types::ChatMessage;
use futures::StreamExt;
use jfc_provider::{Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions};
use tracing::{debug, info, instrument, trace, warn};

use super::{BLOCKED_HEADROOM, CHARS_PER_TOKEN, CIRCUIT_BREAKER_LIMIT, MAX_ATTEMPTS, THRASH_TURN_WINDOW, blocked_override, estimate_tokens};
#[cfg(test)]
use super::{CompactLevel, auto_compact_disabled, compact_level, compact_threshold, should_compact};

struct ConversationGroup {
    messages: Vec<ChatMessage>,
}

fn estimate_group_tokens(group: &ConversationGroup) -> usize {
    let tokens = estimate_tokens(&group.messages);
    trace!(target: "jfc::compact", messages_in_group = group.messages.len(), tokens, "estimate_group_tokens");
    tokens
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
/// Callback fired on every text_delta during a streaming compact. The
/// argument is the *cumulative* summary length so far (in chars) — the
/// renderer divides by 4 for a token estimate. Boxed because the
/// compact path is async + `Send`. Using a callback rather than
/// hard-coding `Sender<AppEvent>` keeps `compact.rs` free of
/// `runtime::AppEvent` so the test build doesn't need the full app.
pub type CompactProgressCb = Box<dyn Fn(u64) + Send + Sync>;

/// Whether an error string indicates the provider doesn't support the
/// requested operation at all — used to bail out early instead of retrying.
fn is_unsupported_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("not support") || m.contains("unsupported") || m.contains("not implemented")
}

pub async fn complete_or_stream(
    provider: &dyn Provider,
    messages: Vec<ProviderMessage>,
    options: &StreamOptions,
    on_progress: Option<&CompactProgressCb>,
) -> Result<jfc_provider::CompletionResponse, anyhow::Error> {
    match provider.complete(messages.clone(), options).await {
        Ok(resp) => {
            if let Some(cb) = on_progress {
                cb(resp.content.len() as u64);
            }
            Ok(resp)
        }
        Err(e) if is_unsupported_error(&e.to_string()) => {
            info!(
                target: "jfc::compact",
                "provider.complete() unsupported — falling back to streaming"
            );
            let mut stream = match provider.stream(messages, options).await {
                Ok(s) => s,
                Err(stream_err) => {
                    // Both complete and stream failed — provider can't compact.
                    // Return a tagged error so the caller can bail out cleanly
                    // instead of retrying in a hot loop.
                    return Err(anyhow::anyhow!(
                        "compaction unsupported: complete failed ({e}); stream also failed ({stream_err})"
                    ));
                }
            };
            let mut collected = String::new();
            while let Some(event) = stream.next().await {
                match event {
                    Ok(jfc_provider::StreamEvent::TextDelta { delta, .. }) => {
                        collected.push_str(&delta);
                        if let Some(cb) = on_progress {
                            cb(collected.len() as u64);
                        }
                    }
                    Ok(jfc_provider::StreamEvent::Done { .. }) => break,
                    Ok(jfc_provider::StreamEvent::Error { message }) => {
                        return Err(anyhow::anyhow!("{}", message));
                    }
                    Ok(_) => {}
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
            Ok(jfc_provider::CompletionResponse {
                content: collected,
                usage: Default::default(),
            })
        }
        Err(e) => Err(e),
    }
}

#[instrument(
    target = "jfc::compact",
    skip(messages, provider, options, tool_ctx, on_progress),
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
    on_progress: Option<CompactProgressCb>,
) -> CompactResult {
    // Recovery: reset the rapid-refill counter when enough turns have
    // elapsed since the last compact. v126's `cli.2.1.126.deob.js:397270`
    // re-evaluates this each turn — `consecutiveRapidRefills` resets to 0
    // whenever `turnCounter >= lG6`. Without this, jfc only reset inside
    // the success path, so once tripped the breaker stayed latched until
    // the user noticed and ran something to clear it.
    let turns_since_compact = tool_ctx
        .total_user_turns
        .saturating_sub(tool_ctx.last_compact_turn);
    if turns_since_compact >= THRASH_TURN_WINDOW && tool_ctx.rapid_refill_count > 0 {
        info!(
            target: "jfc::compact",
            turns_since_compact,
            thrash_window = THRASH_TURN_WINDOW,
            prev_count = tool_ctx.rapid_refill_count,
            "auto-clearing circuit breaker — enough turns elapsed"
        );
        tool_ctx.rapid_refill_count = 0;
    }

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

        // Catch-22 guard: if the chunk we'd send for summarization is itself
        // bigger than the model's context window, no single pass can compact it.
        // In this case we recursively chunk: summarize the first half of the
        // to_summarize slice into a stub and treat that as a single message,
        // then retry. This is a best-effort safeguard — the real fix is to
        // never let sessions grow to 1.4M tokens in the first place.
        if summarize_tokens > window {
            warn!(
                target: "jfc::compact",
                summarize_tokens, window, attempt,
                "to-summarize slice exceeds context window — increasing preserve_count to reduce slice"
            );
            // Drop the oldest group from the to-summarize slice each time
            // until it fits, or until we've exhausted groups.
            let step = token_gap_step(
                Some(summarize_tokens.saturating_sub(window)),
                &group_tokens,
                split_point,
            );
            preserve_count = (preserve_count + step.max(1)).min(total_groups - 1);
            continue;
        }

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

        // Build system prompt with optional custom instructions from config.
        let system_prompt = {
            let mut prompt = COMPACTION_SYSTEM_PROMPT.to_owned();
            if let Some(ref instructions) = crate::config::load().compact_instructions
                && !instructions.trim().is_empty()
            {
                prompt.push_str("\n\nAdditional Instructions:\n");
                prompt.push_str(instructions);
            }
            prompt
        };

        let compact_options = StreamOptions::new(options.model.clone())
            .system(system_prompt)
            .max_tokens(20_000);

        debug!(
            target: "jfc::compact",
            model = %compact_options.model,
            max_tokens = 20_000,
            "sending compaction request to provider"
        );

        match complete_or_stream(
            provider,
            compact_messages,
            &compact_options,
            on_progress.as_ref(),
        )
        .await
        {
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
                    let step = token_gap_step(last_token_gap, &group_tokens, split_point);
                    preserve_count = (preserve_count + step).min(total_groups - 1);
                    continue;
                }
                // `format_compact_summary` returns `None` when it detects a
                // truncated tag (e.g. `<summary>` with no matching close, or
                // `<analysis>` with no close). A truncated stream would
                // otherwise leak draft scratchpad content into the boundary
                // summary, so we treat it as a streaming failure and retry.
                let Some(formatted) = format_compact_summary(&response.content) else {
                    warn!(
                        target: "jfc::compact",
                        len = response.content.len(),
                        response_preview = %&response.content[..response.content.len().min(300)],
                        "summary response had unmatched tags (likely truncated stream) — retrying with larger preserve"
                    );
                    let step = token_gap_step(last_token_gap, &group_tokens, split_point);
                    preserve_count = (preserve_count + step).min(total_groups - 1);
                    continue;
                };
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
                let blocked =
                    blocked_override().unwrap_or_else(|| window.saturating_sub(BLOCKED_HEADROOM));
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

                // Post-compact file restoration: re-inject the most recently
                // read files as context so the model doesn't "forget what it
                // was editing". CC 2.1.144 does this via `iM8` (up to dM8=5
                // files, capped at N45 total tokens). We snapshot the cache
                // paths before clearing, then inject shortened file contents.
                let restored_files = restore_recent_files(&tool_ctx.read_cache);
                tool_ctx.read_cache.clear();

                if !restored_files.is_empty() {
                    let restore_text = restored_files.join("\n\n");
                    let restore_msg = ChatMessage::assistant(format!(
                        "[Post-compact context restoration — recently accessed files:]\n\n{}",
                        restore_text
                    ));
                    compacted.push(restore_msg);
                    // Recompute post_tokens with the restored files included
                    let post_tokens = estimate_tokens(&compacted);
                    tool_ctx.approx_tokens = post_tokens;
                }

                info!(
                    target: "jfc::compact",
                    pre_tokens,
                    post_tokens = tool_ctx.approx_tokens,
                    saved = pre_tokens.saturating_sub(tool_ctx.approx_tokens),
                    compacted_message_count = compacted.len(),
                    restored_files = restored_files.len(),
                    attempts = attempt,
                    model = %options.model,
                    "compaction succeeded"
                );

                return CompactResult::Success {
                    messages: compacted,
                    pre_tokens,
                    post_tokens: tool_ctx.approx_tokens,
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

                if is_unsupported_error(&err_msg) {
                    info!(
                        target: "jfc::compact",
                        error = %e,
                        "provider does not support compaction — aborting"
                    );
                    return CompactResult::Unsupported;
                }

                // Exponential backoff between retry attempts to prevent
                // hot-loop storms (39,563 failures/day observed in logs).
                let backoff_ms = 250u64.saturating_mul(1u64 << attempt.min(6));
                tracing::debug!(
                    target: "jfc::compact",
                    attempt, backoff_ms,
                    "unrecognized error — backing off before next attempt"
                );
                tokio::time::sleep(std::time::Duration::from_millis(backoff_ms)).await;
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

    // Positive evidence a real summary is present — short-circuit accept.
    // A turn that summarized a conversation about errors (e.g. the user
    // debugging context-window bugs) will mention "prompt is too long"
    // *as content*. Without this short-circuit a legitimate 16k-char
    // summary got rejected because the body discussed those error
    // strings — the bug shown in the user's compaction log.
    //
    // Invariant: if an opening tag is present, the matching closing tag
    // must also be present. A truncated mid-stream response (e.g. the
    // provider hung up after `<summary>` but before `</summary>`) would
    // otherwise pass this gate and be treated as a valid boundary by
    // `format_compact_summary`, leaking draft scratchpad content. Reject
    // any half-open tag pair so the compaction loop retries with a
    // larger preserve count instead.
    let has_open_summary = trimmed.contains("<summary>");
    let has_close_summary = trimmed.contains("</summary>");
    let has_open_analysis = trimmed.contains("<analysis>");
    let has_close_analysis = trimmed.contains("</analysis>");
    if (has_open_summary && !has_close_summary) || (has_open_analysis && !has_close_analysis) {
        debug!(
            target: "jfc::compact",
            has_open_summary, has_close_summary,
            has_open_analysis, has_close_analysis,
            text_len = trimmed.len(),
            "is_usable_summary: rejected — half-open tag pair (likely truncated stream)"
        );
        return false;
    }
    if has_open_summary || has_open_analysis {
        trace!(
            target: "jfc::compact",
            text_len = trimmed.len(),
            "is_usable_summary: accepted (summary/analysis tag present and closed)"
        );
        return true;
    }

    // Fallback rejection: response *itself* is just an API error string
    // the proxy echoed back. v126's `Od()` (cli.js:179986) does a strict
    // `startsWith` against the known error-prefix constants — not a
    // substring scan. We mirror that, plus a length cap so a runaway
    // prefix-match on a legitimate long response can't fire.
    const ERROR_PREFIX_PATTERNS: &[&str] = &[
        "litellm.", // LiteLLM exception prefix (BedrockException, ContextWindowExceededError, etc.)
        "{\"error\":", // OWUI/OpenAI proxy JSON-error blob
        "{\"message\":", // alt JSON envelope
        "Error:",   // generic prefix
        "BadRequestError:", // litellm BadRequestError
        "BedrockException:",
        "AnthropicException:",
        "context_window_fallback", // litellm fallback message
    ];
    let starts_with_error = ERROR_PREFIX_PATTERNS.iter().any(|p| trimmed.starts_with(p));
    let is_short_enough_to_be_only_error = trimmed.len() < 2_000;
    let rejected = starts_with_error && is_short_enough_to_be_only_error;

    if rejected {
        debug!(
            target: "jfc::compact",
            text_preview = %&trimmed[..trimmed.len().min(150)],
            text_len = trimmed.len(),
            "is_usable_summary: rejected — response begins with API-error prefix"
        );
    } else {
        trace!(
            target: "jfc::compact",
            text_len = trimmed.len(),
            "is_usable_summary: accepted"
        );
    }
    !rejected
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
            if let Ok(n) = msg[start..i].parse::<usize>()
                && n > 10_000
            {
                nums.push(n);
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
    let mut text = String::from("Here is the conversation to summarize:\n\n");

    // Observation masking threshold: tool outputs larger than this get
    // replaced with a placeholder. Based on "The Complexity Trap"
    // (NeurIPS DL4C '25) — simple observation masking halves cost with
    // zero accuracy loss compared to full LLM summarization.
    const MASK_THRESHOLD_CHARS: usize = 2000;

    for msg in messages {
        let role = if msg.role_is_user() {
            "H" // Human
        } else {
            "A" // Assistant
        };
        text.push_str(&format!("[{}]\n", role));
        for part in &msg.parts {
            match part {
                crate::types::MessagePart::Tool(tc) => {
                    let output_text = if strip_media {
                        tc.output.text_only()
                    } else {
                        tc.output.to_display_string()
                    };
                    let input_summary = tc.input.summary();
                    // Mask large tool outputs to reduce token cost.
                    // Keep the tool name + input summary + a size indicator
                    // so the summarizer knows what *happened* without the
                    // full output text.
                    if output_text.len() > MASK_THRESHOLD_CHARS {
                        let approx_tokens = output_text.len() / CHARS_PER_TOKEN;
                        text.push_str(&format!(
                            "[Tool: {} | Input: {} | Output: ~{} tokens, truncated]\n",
                            tc.kind.label(),
                            input_summary,
                            approx_tokens,
                        ));
                    } else {
                        text.push_str(&format!(
                            "[Tool: {} | Input: {} | Output: {}]\n",
                            tc.kind.label(),
                            input_summary,
                            output_text,
                        ));
                    }
                }
                _ => {
                    if strip_media {
                        text.push_str(&part.text_only());
                    } else {
                        text.push_str(&part.to_display_string());
                    }
                    text.push('\n');
                }
            }
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
///
/// Returns `None` when the input contains a half-open tag pair (e.g. an
/// `<analysis>` opening tag without a matching `</analysis>`, or a
/// `<summary>` opening tag without `</summary>`). A truncated mid-stream
/// response would otherwise leak the draft scratchpad into the final
/// boundary summary or yield no extracted content at all — both violate
/// the "strip analysis, keep summary" contract. The compaction loop
/// treats `None` as a streaming failure and retries with a larger
/// preserve count.
///
/// Inputs that contain neither `<analysis>` nor `<summary>` (an LLM that
/// ignored the format instructions but produced usable plaintext) are
/// returned trimmed and unmodified.
fn format_compact_summary(raw: &str) -> Option<String> {
    // Detect truncation BEFORE any rewriting. We treat an opening tag
    // without a matching closing tag as a streaming failure rather than
    // (a) silently dropping the open tag, or (b) consuming everything
    // after it as analysis. Both alternatives risk corrupting the
    // boundary message — option (b) silently strips real summary
    // content, option (a) leaks scratchpad text. Returning `None` so
    // the caller retries is the only safe choice.
    if raw.contains("<analysis>") && !raw.contains("</analysis>") {
        warn!(
            target: "jfc::compact",
            raw_len = raw.len(),
            "format_compact_summary: <analysis> opened but never closed — treating as truncation"
        );
        return None;
    }
    if raw.contains("<summary>") && !raw.contains("</summary>") {
        warn!(
            target: "jfc::compact",
            raw_len = raw.len(),
            "format_compact_summary: <summary> opened but never closed — treating as truncation"
        );
        return None;
    }

    let mut result = raw.to_string();

    // Strip analysis section — it's a drafting scratchpad. The
    // truncation guard above already rejected unmatched opens, so the
    // inner `if let` only executes for properly paired tags.
    if let Some(start) = result.find("<analysis>")
        && let Some(end) = result.find("</analysis>")
    {
        let end_tag_end = end + "</analysis>".len();
        let analysis_len = end_tag_end - start;
        debug!(
            target: "jfc::compact",
            analysis_len,
            "stripped <analysis> block from summary"
        );
        result = format!("{}{}", &result[..start], &result[end_tag_end..]);
    }

    // Extract summary content. Same guarantee as above — if an opening
    // tag is present, the closing tag is too.
    if let Some(start) = result.find("<summary>")
        && let Some(end) = result.find("</summary>")
    {
        let content_start = start + "<summary>".len();
        let content = result[content_start..end].trim();
        debug!(
            target: "jfc::compact",
            summary_content_len = content.len(),
            "extracted <summary> block"
        );
        result = format!("Summary:\n{content}");
    }

    // Clean up extra whitespace
    while result.contains("\n\n\n") {
        result = result.replace("\n\n\n", "\n\n");
    }

    Some(result.trim().to_string())
}

/// Post-compact file restoration: read the most recently accessed files
/// from the read cache and return truncated snippets. CC 2.1.144 does
/// this via `iM8` — re-reads up to 5 files capped at 20K total tokens
/// so the model has fresh context for files it was actively editing.
///
/// We read the first 200 lines or 8K chars of each file, whichever is
/// less — enough for the model to orient but not enough to bust the
/// post-compact token budget.
fn restore_recent_files(cache: &crate::context::ReadDedupCache) -> Vec<String> {
    const MAX_FILES: usize = 5;
    const MAX_CHARS_PER_FILE: usize = 8_000;
    const MAX_TOTAL_CHARS: usize = 20_000 * CHARS_PER_TOKEN; // ~20K tokens

    let paths = cache.paths();
    if paths.is_empty() {
        return Vec::new();
    }

    let mut results = Vec::new();
    let mut total_chars = 0usize;

    // Sort by modification time (most recent first) so we prioritize
    // the files the user was most recently working on.
    let mut sorted_paths = paths;
    sorted_paths.sort_by(|a, b| {
        let a_mtime = std::fs::metadata(a).and_then(|m| m.modified()).ok();
        let b_mtime = std::fs::metadata(b).and_then(|m| m.modified()).ok();
        b_mtime.cmp(&a_mtime)
    });

    for path in sorted_paths.iter().take(MAX_FILES) {
        if total_chars >= MAX_TOTAL_CHARS {
            break;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let truncated: String = content.chars().take(MAX_CHARS_PER_FILE).collect();
        let was_truncated = content.len() > MAX_CHARS_PER_FILE;
        let suffix = if was_truncated {
            format!("\n… [truncated at {} chars]", MAX_CHARS_PER_FILE)
        } else {
            String::new()
        };
        let entry = format!("--- {} ---\n{}{}", path.display(), truncated, suffix,);
        let entry_len = entry.len();
        total_chars += entry_len;
        results.push(entry);

        debug!(
            target: "jfc::compact",
            path = %path.display(),
            chars = entry_len,
            truncated = was_truncated,
            "restored file post-compact"
        );
    }

    info!(
        target: "jfc::compact",
        files_restored = results.len(),
        total_chars,
        "post-compact file restoration complete"
    );
    results
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
        // ok zone (below 80% of compact threshold)
        // compact threshold = 187K, 80% = 149_600
        assert_eq!(compact_level(0, W), CompactLevel::Ok);
        assert_eq!(compact_level(149_599, W), CompactLevel::Ok);
        // precompute at 80% of compact threshold ≈ 149_600
        assert_eq!(compact_level(149_600, W), CompactLevel::Precompute);
        assert_eq!(compact_level(166_999, W), CompactLevel::Precompute);
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

    #[serial_test::serial]
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

    #[serial_test::serial]
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

    #[serial_test::serial]
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
        assert_eq!(parse_token_gap_from_error("ok: 100 tokens of 200000"), None);
        assert_eq!(parse_token_gap_from_error("connection reset"), None);
    }

    #[test]
    fn is_usable_summary_rejects_empty_or_whitespace() {
        assert!(!is_usable_summary(""));
        assert!(!is_usable_summary("   \n\t  "));
    }

    // Echoed API errors that the proxy returned as the entire response
    // body. These start with a recognizable error prefix and are short.
    #[test]
    fn is_usable_summary_rejects_echoed_api_errors() {
        assert!(!is_usable_summary(
            "litellm.ContextWindowExceededError: prompt is too long: 410234 tokens > 200000 maximum"
        ));
        assert!(!is_usable_summary(
            r#"{"error":{"message":"BedrockException: Context Window Error"}}"#
        ));
        assert!(!is_usable_summary("BedrockException: Context Window Error"));
    }

    #[test]
    fn is_usable_summary_accepts_real_summary() {
        let real = "Session summary:\n- User asked about compaction.\n- \
                    Implemented post-compact validation.";
        assert!(is_usable_summary(real));
    }

    // Regression: a legitimate v126-format summary whose CONTENT
    // discusses context-window or prompt-too-long errors used to be
    // false-positive rejected by the old substring matcher. The user's
    // log showed a 16k-char `<analysis>...` body rejected because the
    // assistant was summarizing a debug session about that very error.
    // The presence of `<summary>` or `<analysis>` is positive evidence
    // and short-circuits acceptance regardless of error-string content.
    #[test]
    fn is_usable_summary_accepts_summary_about_errors_robust() {
        let body = "<analysis>\nThe user reported `prompt is too long: \
                    1267440 tokens > 1000000 maximum` while debugging\n\
                    </analysis>\n<summary>\nFixed compaction context \
                    window handling.\n</summary>";
        assert!(
            is_usable_summary(body),
            "summary that discusses error strings as content should pass"
        );
    }

    // Regression: a long summary with rich content that happens to
    // mention "rate limit" anywhere should still be accepted. v126's
    // `Od()` is a startsWith check, not a substring scan.
    #[test]
    fn is_usable_summary_accepts_long_response_mentioning_errors_robust() {
        // 2.5k chars of legit content that includes an error phrase.
        let body = format!(
            "Session covered many topics including rate_limit handling \
             and context window debugging. {}",
            "x".repeat(2_500)
        );
        assert!(
            is_usable_summary(&body),
            "long response should be accepted regardless of substring mentions"
        );
    }

    // Robust: a JSON error blob that's clearly the proxy echoing the
    // upstream error verbatim — no `<summary>`, starts with `{"error":`,
    // short — is correctly rejected.
    #[test]
    fn is_usable_summary_rejects_short_json_error_blob_robust() {
        let body = r#"{"error":{"message":"litellm.BadRequestError: too long","code":"400"}}"#;
        assert!(!is_usable_summary(body));
    }

    // ──────────────────────────────────────────────────────────────────
    // Pure-helper coverage: split_into_groups, estimate_tokens,
    // count_user_turns_since_last_compact, token_gap_step,
    // format_compact_summary, parse_actual_tokens_from_error,
    // should_compact boundary, and is_usable_summary additional paths.
    // ──────────────────────────────────────────────────────────────────

    use crate::types::ChatMessage;

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage::user(text.to_owned())
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage::assistant(text.to_owned())
    }

    // Normal: splitting on user-turn boundaries collects groups so each
    // starts with the user message that initiated it.
    #[test]
    fn split_into_groups_separates_at_user_turns_normal() {
        let messages = vec![
            user_msg("first prompt"),
            assistant_msg("first reply"),
            user_msg("second prompt"),
            assistant_msg("second reply"),
            assistant_msg("more reply"),
        ];
        let groups = split_into_groups(&messages);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].messages.len(), 2);
        assert_eq!(groups[1].messages.len(), 3);
    }

    // Robust: an empty messages slice produces no groups.
    #[test]
    fn split_into_groups_empty_robust() {
        let groups = split_into_groups(&[]);
        assert!(groups.is_empty());
    }

    // Robust: assistant-first conversation collects everything into one group
    // because the loop only splits when a *user* message is seen with prior
    // content already buffered.
    #[test]
    fn split_into_groups_assistant_first_robust() {
        let messages = vec![assistant_msg("starts here"), user_msg("then user")];
        let groups = split_into_groups(&messages);
        assert_eq!(groups.len(), 2);
        // First group: just the assistant message.
        assert_eq!(groups[0].messages.len(), 1);
        // Second group: the user message that triggered the split.
        assert_eq!(groups[1].messages.len(), 1);
    }

    // Normal: estimate_tokens scales with input length and applies the
    // overhead multiplier (3/2 = 1.5x).
    #[test]
    fn estimate_tokens_applies_overhead_normal() {
        // 16-char message → base = 4 tokens → est = 6.
        let messages = vec![user_msg("0123456789abcdef")];
        let est = estimate_tokens(&messages);
        assert_eq!(est, 6);
    }

    // Normal: estimate_tokens on empty input returns 0.
    #[test]
    fn estimate_tokens_empty_is_zero_normal() {
        assert_eq!(estimate_tokens(&[]), 0);
    }

    // Normal: count_user_turns counts back from the end and stops at the
    // first compact boundary it sees.
    #[test]
    fn count_user_turns_stops_at_compact_boundary_normal() {
        let messages = vec![
            user_msg("very old"),
            ChatMessage::compact_boundary("summary", 1234),
            user_msg("after compact 1"),
            assistant_msg("reply"),
            user_msg("after compact 2"),
        ];
        let count = count_user_turns_since_last_compact(&messages);
        assert_eq!(count, 2);
    }

    // Robust: with no compact boundary at all, every user turn counts.
    #[test]
    fn count_user_turns_no_boundary_counts_all_robust() {
        let messages = vec![
            user_msg("u1"),
            assistant_msg("a1"),
            user_msg("u2"),
            user_msg("u3"),
        ];
        assert_eq!(count_user_turns_since_last_compact(&messages), 3);
    }

    // Normal: token_gap_step with `None` falls back to halving (current/2),
    // never zero.
    #[test]
    fn token_gap_step_falls_back_to_halving_normal() {
        let group_tokens = vec![100, 200, 300, 400];
        assert_eq!(token_gap_step(None, &group_tokens, 4), 2);
        assert_eq!(token_gap_step(None, &group_tokens, 1), 1); // never zero
    }

    // Normal: with a token_gap, walk groups backward accumulating tokens
    // until enough has been freed.
    #[test]
    fn token_gap_step_walks_until_gap_freed_normal() {
        let group_tokens = vec![100, 200, 300, 400];
        // gap=350 starting at split=4: walk 400 (>=350) → 1 group.
        assert_eq!(token_gap_step(Some(350), &group_tokens, 4), 1);
        // gap=500: 400 + 300 = 700 covers it → 2 groups.
        assert_eq!(token_gap_step(Some(500), &group_tokens, 4), 2);
        // gap=999_999: walks all 4 groups.
        assert_eq!(token_gap_step(Some(999_999), &group_tokens, 4), 4);
    }

    // Robust: token_gap_step returns at least 1 even when gap is 0.
    #[test]
    fn token_gap_step_returns_at_least_one_robust() {
        let group_tokens = vec![100, 200];
        assert_eq!(token_gap_step(Some(0), &group_tokens, 2), 1);
    }

    // Normal: format_compact_summary strips <analysis> blocks and keeps the
    // <summary> body, prefixed with "Summary:".
    #[test]
    fn format_compact_summary_strips_analysis_normal() {
        let raw = "<analysis>\nDraft notes here.\n</analysis>\n<summary>\nFinal summary text.\n</summary>";
        let formatted = format_compact_summary(raw).expect("matched tags should yield Some");
        assert!(!formatted.contains("Draft notes"));
        assert!(formatted.starts_with("Summary:"));
        assert!(formatted.contains("Final summary text."));
    }

    // Robust: a response without tags is returned trimmed (whitespace).
    #[test]
    fn format_compact_summary_passes_through_untagged_robust() {
        let raw = "  Just a plain summary, no tags.  ";
        let formatted = format_compact_summary(raw).expect("untagged input should yield Some");
        assert_eq!(formatted, "Just a plain summary, no tags.");
    }

    // Robust: triple newlines collapse to double in the cleanup pass.
    #[test]
    fn format_compact_summary_collapses_triple_newlines_robust() {
        let raw = "first\n\n\nsecond";
        let formatted = format_compact_summary(raw).expect("untagged input should yield Some");
        assert!(!formatted.contains("\n\n\n"));
        assert!(formatted.contains("first"));
        assert!(formatted.contains("second"));
    }

    // Regression: an `<analysis>` opening tag with no closing tag (a
    // truncated mid-stream response) must NOT be silently passed through —
    // it would either leak scratchpad content or strip the rest of the
    // body. Returning `None` lets the compaction retry loop surface a new
    // request with a larger preserve count.
    #[test]
    fn format_compact_summary_rejects_unclosed_analysis_robust() {
        let raw = "<analysis>\nDraft notes that never finished";
        assert!(format_compact_summary(raw).is_none());
    }

    // Regression: same contract for the `<summary>` half-open case.
    #[test]
    fn format_compact_summary_rejects_unclosed_summary_robust() {
        let raw = "<analysis>\nok\n</analysis>\n<summary>\nTruncated summary text";
        assert!(format_compact_summary(raw).is_none());
    }

    // Regression: `is_usable_summary` rejects half-open tags before the
    // formatter ever sees them, so the compaction loop bails on the
    // earlier gate without paying for the formatter call.
    #[test]
    fn is_usable_summary_rejects_unclosed_summary_tag_robust() {
        let body = "<summary>\nTruncated mid-stream";
        assert!(
            !is_usable_summary(body),
            "half-open <summary> must be rejected at the gate"
        );
    }

    #[test]
    fn is_usable_summary_rejects_unclosed_analysis_tag_robust() {
        let body = "<analysis>\nDraft scratchpad cut off";
        assert!(
            !is_usable_summary(body),
            "half-open <analysis> must be rejected at the gate"
        );
    }

    // Normal: parse_actual_tokens picks the FIRST integer >10_000 from
    // an Anthropic too-long error, so the calibrated approx_tokens lines
    // up with the API's view.
    #[test]
    fn parse_actual_tokens_picks_first_large_integer_normal() {
        let msg = "prompt is too long: 1456365 tokens > 1000000 maximum";
        assert_eq!(parse_actual_tokens_from_error(msg), Some(1_456_365));
    }

    // Robust: small numbers (<=10_000) are skipped — they're status codes,
    // line numbers, etc., not token counts.
    #[test]
    fn parse_actual_tokens_skips_small_numbers_robust() {
        let msg = "error 400: 200 tokens isn't right";
        // No number is > 10_000, so None.
        assert_eq!(parse_actual_tokens_from_error(msg), None);
    }

    // Normal: should_compact fires when level is Compact or Blocked.
    #[test]
    fn should_compact_fires_at_compact_threshold_normal() {
        let _g = lock();
        clear_env();
        // Compact threshold for 200K window = 187K.
        assert!(!should_compact(186_999, W));
        assert!(should_compact(187_000, W));
        assert!(should_compact(199_000, W));
    }

    // Robust: when auto-compact is disabled, should_compact only fires at
    // the hard Blocked level (api-enforced ceiling).
    #[serial_test::serial]
    #[test]
    fn should_compact_disabled_only_blocks_robust() {
        let _g = lock();
        clear_env();
        unsafe {
            std::env::set_var("JFC_DISABLE_AUTO_COMPACT", "1");
        }
        assert!(!should_compact(195_000, W));
        // Blocked still fires regardless of disable.
        assert!(should_compact(198_000, W));
        clear_env();
    }

    // Normal: blocked override env var lowers the blocked threshold.
    #[serial_test::serial]
    #[test]
    fn blocked_override_lowers_threshold_normal() {
        let _g = lock();
        clear_env();
        unsafe {
            std::env::set_var("JFC_BLOCKING_LIMIT_OVERRIDE", "50000");
        }
        // Now anything >= 50K should be Blocked.
        assert_eq!(compact_level(50_000, W), CompactLevel::Blocked);
        clear_env();
    }

    // Robust: `auto_compact_disabled()` reflects either env var.
    #[serial_test::serial]
    #[test]
    fn auto_compact_disabled_responds_to_env_robust() {
        let _g = lock();
        clear_env();
        assert!(!auto_compact_disabled());
        unsafe {
            std::env::set_var("JFC_DISABLE_COMPACT", "true");
        }
        assert!(auto_compact_disabled());
        clear_env();
        unsafe {
            std::env::set_var("JFC_DISABLE_AUTO_COMPACT", "1");
        }
        assert!(auto_compact_disabled());
        clear_env();
    }

    // Robust: zero or invalid pct_override values are ignored — the default
    // threshold applies.
    #[serial_test::serial]
    #[test]
    fn pct_override_ignores_invalid_values_robust() {
        let _g = lock();
        clear_env();
        unsafe {
            std::env::set_var("JFC_AUTOCOMPACT_PCT_OVERRIDE", "not-a-number");
        }
        assert_eq!(compact_threshold(W), 187_000);
        unsafe {
            std::env::set_var("JFC_AUTOCOMPACT_PCT_OVERRIDE", "0");
        }
        assert_eq!(compact_threshold(W), 187_000);
        unsafe {
            std::env::set_var("JFC_AUTOCOMPACT_PCT_OVERRIDE", "200");
        }
        // Out of range — ignored.
        assert_eq!(compact_threshold(W), 187_000);
        clear_env();
    }

    // Normal: estimate_group_tokens of a single-message group equals
    // estimate_tokens of that one message. (Sanity round-trip.)
    #[test]
    fn estimate_group_tokens_matches_estimate_tokens_normal() {
        let group = ConversationGroup {
            messages: vec![user_msg("0123456789abcdef")], // 16 chars → 6 tokens
        };
        assert_eq!(estimate_group_tokens(&group), 6);
    }

    // Normal: format_compact_summary trims trailing whitespace from extracted
    // summary content even when nested in surrounding text.
    #[test]
    fn format_compact_summary_extracts_inner_summary_normal() {
        let raw = "Some preamble.\n<summary>\n  inner content  \n</summary>\nignored after";
        let formatted = format_compact_summary(raw).expect("matched tags should yield Some");
        assert!(formatted.starts_with("Summary:"));
        assert!(formatted.contains("inner content"));
    }
}
