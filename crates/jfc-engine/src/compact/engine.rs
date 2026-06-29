use crate::context::ToolContext;
use crate::types::{ChatMessage, MessagePart, Role};
use futures::StreamExt;
use jfc_core::context_management::{ContextItem, ContextItemKind, ContextSignals};
use jfc_provider::{
    ModelRequestProfile, Provider, ProviderContent, ProviderMessage, ProviderRole, StreamOptions,
};
use std::collections::HashMap;
use std::fmt::Write as _;
use tracing::{debug, info, instrument, trace, warn};

use super::{
    CHARS_PER_TOKEN, CIRCUIT_BREAKER_LIMIT, MAX_ATTEMPTS, THRASH_TURN_WINDOW,
    blocked_threshold_with_output, estimate_tokens,
};
#[cfg(test)]
use super::{
    CompactLevel, auto_compact_disabled, compact_level, compact_threshold, should_compact,
};

struct ConversationGroup {
    messages: Vec<ChatMessage>,
}

fn estimate_group_tokens(group: &ConversationGroup) -> usize {
    let tokens = estimate_tokens(&group.messages);
    trace!(target: "jfc::compact", messages_in_group = group.messages.len(), tokens, "estimate_group_tokens");
    tokens
}

/// Compute the recency-weighted preserve floor — how many of the newest groups
/// to keep verbatim on the *first* compaction attempt — via
/// [`jfc_core::context_management::recency_preserve_floor`].
///
/// `group_tokens` is oldest-first (matching `split_into_groups`).
/// The core policy keeps the newest turns whose cumulative tokens fit the
/// proof-backed recency budget, then clamps to `[1, total-1]` so there is
/// always at least one group to preserve and at least one to summarize.
fn recency_preserve_floor(group_tokens: &[usize], window: usize) -> usize {
    jfc_core::context_management::recency_preserve_floor(group_tokens, window)
}

fn context_item_kind_for_group(group: &ConversationGroup) -> ContextItemKind {
    if group.messages.iter().any(ChatMessage::is_compact_boundary) {
        return ContextItemKind::CompactSummary;
    }

    if group
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .any(|part| matches!(part, MessagePart::Tool(_)))
    {
        return ContextItemKind::ToolOutput;
    }

    if group
        .messages
        .iter()
        .any(|message| message.role == Role::User)
    {
        ContextItemKind::UserText
    } else {
        ContextItemKind::AssistantText
    }
}

fn context_items_for_groups(
    groups: &[ConversationGroup],
    group_tokens: &[usize],
) -> Vec<ContextItem> {
    groups
        .iter()
        .enumerate()
        .map(|(idx, group)| ContextItem {
            id: idx as u64,
            position: idx as u64,
            token_estimate: group_tokens.get(idx).copied().unwrap_or(1).max(1) as u64,
            kind: context_item_kind_for_group(group),
            duplicate_count: 0,
        })
        .collect()
}

fn provider_signal_preserve_floor(
    groups: &[ConversationGroup],
    group_tokens: &[usize],
    window: usize,
    signals: &ContextSignals,
) -> Option<usize> {
    if groups.len() <= 1 {
        return Some(1);
    }

    let items = context_items_for_groups(groups, group_tokens);
    let signal_budget = jfc_core::context_management::recency_budget(window) as u64;
    let selected = jfc_core::context_management::select_context_items_with_signals(
        &items,
        signal_budget,
        Some(signals),
    );
    let oldest_selected = selected
        .into_iter()
        .filter_map(|id| usize::try_from(id).ok())
        .filter(|idx| *idx < groups.len())
        .min()?;

    Some(
        groups
            .len()
            .saturating_sub(oldest_selected)
            .min(groups.len() - 1)
            .max(1),
    )
}

fn truncated_signal_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }

    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("\n[truncated]");
    out
}

fn context_signal_probe_messages(groups: &[ConversationGroup]) -> Vec<ProviderMessage> {
    let mut body = String::from(
        "JFC context-signal probe. If this provider exposes attention or KV-cache metadata, \
         return scores mapped to the numeric context_item ids below. Do not answer the user.\n",
    );

    for (idx, group) in groups.iter().enumerate() {
        let _ = writeln!(
            body,
            "\n<context_item id=\"{idx}\" messages=\"{}\">",
            group.messages.len()
        );
        for message in &group.messages {
            let _ = writeln!(body, "[role={}]", message.role);
            for part in &message.parts {
                let _ = writeln!(body, "{}", truncated_signal_text(&part.text_only(), 4096));
            }
        }
        body.push_str("</context_item>\n");
    }

    vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(body)],
    }]
}

async fn provider_context_signals_for_groups(
    provider: &dyn Provider,
    model: &str,
    groups: &[ConversationGroup],
) -> Option<ContextSignals> {
    if !provider.supports_context_signals() {
        return None;
    }

    let messages = context_signal_probe_messages(groups);
    match provider.context_signals(model, messages).await {
        Ok(Some(signals)) if !signals.is_empty() => Some(signals),
        Ok(_) => None,
        Err(error) => {
            debug!(
                target: "jfc::compact",
                error = %error,
                "provider context-signal export failed; using synthetic context policy"
            );
            None
        }
    }
}

/// Quantified before/after for the recency-weighted compaction floor.
///
/// The RCT finding is that maximally-aggressive compaction (preserve only the
/// single newest group) backfires: the model re-derives the recent context it
/// lost and writes *longer* outputs, raising total cost. The fix is the recency
/// floor ([`recency_preserve_floor`]). This makes that claim measurable rather
/// than asserted: it reports how many newest-context tokens the floor preserves
/// verbatim on the first pass versus the old `preserve_count = 1` baseline.
///
/// Measurement-only (consumed by the recency tests), so it is `#[cfg(test)]`
/// like the other compaction-policy probes in this module.
#[cfg(test)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecencyMeasurement {
    /// Newest-context tokens preserved verbatim on the first pass with the
    /// recency floor.
    pub tokens_preserved_with_floor: usize,
    /// Newest-context tokens preserved by the old `preserve_count = 1` policy
    /// (just the last group).
    pub tokens_preserved_baseline: usize,
    /// Groups preserved with the floor (vs always 1 for the baseline).
    pub groups_preserved_with_floor: usize,
}

#[cfg(test)]
impl RecencyMeasurement {
    /// Extra newest-context tokens kept verbatim by the floor — the headline
    /// number. Larger = less recent detail thrown away on the first pass.
    pub fn extra_recent_tokens_preserved(&self) -> usize {
        self.tokens_preserved_with_floor
            .saturating_sub(self.tokens_preserved_baseline)
    }
}

/// Measure the recency floor against the `preserve_count = 1` baseline for an
/// oldest-first `group_tokens` vector and a context `window`.
#[cfg(test)]
pub fn measure_recency_floor(group_tokens: &[usize], window: usize) -> RecencyMeasurement {
    let total = group_tokens.len();
    let sum_newest = |count: usize| -> usize { group_tokens.iter().rev().take(count).sum() };
    let floor_groups = recency_preserve_floor(group_tokens, window);
    RecencyMeasurement {
        tokens_preserved_with_floor: sum_newest(floor_groups),
        tokens_preserved_baseline: if total == 0 { 0 } else { sum_newest(1) },
        groups_preserved_with_floor: floor_groups,
    }
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
    let step = jfc_core::context_management::token_gap_step(token_gap, group_tokens, current_split);
    match token_gap {
        None => {
            debug!(
                target: "jfc::compact",
                current_split, step,
                "token_gap_step: no gap info, falling back to halving"
            );
        }
        Some(gap) => {
            let freed: usize = group_tokens[..current_split.min(group_tokens.len())]
                .iter()
                .rev()
                .take(step)
                .fold(0usize, |sum, tokens| sum.saturating_add(*tokens));
            debug!(
                target: "jfc::compact",
                gap, current_split, freed, step,
                "token_gap_step: computed step from token gap"
            );
        }
    }
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
/// hard-coding `Sender<EngineEvent>` keeps `compact.rs` free of
/// `runtime::EngineEvent` so the test build doesn't need the full app.
pub type CompactProgressCb = Box<dyn Fn(u64) + Send + Sync>;

/// Whether an error string indicates the provider doesn't support the
/// requested operation at all — used to bail out early instead of retrying.
fn is_unsupported_error(msg: &str) -> bool {
    let m = msg.to_lowercase();
    m.contains("not support") || m.contains("unsupported") || m.contains("not implemented")
}

fn compact_stream_options(
    provider: &dyn Provider,
    base_options: &StreamOptions,
    system_prompt: String,
) -> StreamOptions {
    let selected_model_info = provider
        .available_models()
        .into_iter()
        .find(|info| info.id == base_options.model);
    let model_profile = ModelRequestProfile::from_provider_model(
        provider.name(),
        base_options.model.as_str(),
        selected_model_info
            .as_ref()
            .and_then(|info| info.context_window_tokens),
        selected_model_info
            .as_ref()
            .and_then(|info| info.max_output_tokens),
    );
    model_profile.clamp_options(
        StreamOptions::new(base_options.model.clone())
            .system(system_prompt)
            .max_tokens(20_000),
    )
}

fn is_output_token_limit_error(err_msg: &str) -> bool {
    (err_msg.contains("max_tokens:") || err_msg.contains("max_output_tokens"))
        && (err_msg.contains("maximum allowed number of output tokens")
            || err_msg.contains("maximum output tokens")
            || err_msg.contains("output token"))
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
                context_signals: None,
                reasoning: None,
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
    max_output_tokens: Option<usize>,
    on_progress: Option<CompactProgressCb>,
) -> CompactResult {
    let _ls = linkscope::phase("turn.compact");
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
    // Recency-weighted preserve floor (RCT finding: uniform aggressive
    // compression backfires — preserving only the newest group makes the model
    // re-derive lost context and write *longer* outputs, so total cost rises).
    // `recency_preserve_floor` keeps the newest groups that fit a fraction of
    // the window verbatim, so the very first compaction attempt isn't maximally
    // aggressive. The retry loop only ever *raises* preserve_count, so starting
    // at this floor (>= 1) is always safe.
    let mut preserve_count: usize = recency_preserve_floor(&group_tokens, window);
    if let Some(signals) =
        provider_context_signals_for_groups(provider, options.model.as_str(), &groups).await
        && let Some(signal_floor) =
            provider_signal_preserve_floor(&groups, &group_tokens, window, &signals)
    {
        let previous_floor = preserve_count;
        preserve_count = preserve_count.max(signal_floor);
        debug!(
            target: "jfc::compact",
            previous_floor,
            signal_floor,
            preserve_count,
            attention_items = signals.attention_tokens.len(),
            kv_entries = signals.kv_entries.len(),
            "provider context signals adjusted initial preserve floor"
        );
    }
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
            if let Some(ref instructions) = crate::config::load_arc().compact_instructions
                && !instructions.trim().is_empty()
            {
                prompt.push_str("\n\nAdditional Instructions:\n");
                prompt.push_str(instructions);
            }
            prompt
        };

        let compact_options = compact_stream_options(provider, options, system_prompt);

        debug!(
            target: "jfc::compact",
            model = %compact_options.model,
            max_tokens = compact_options.max_tokens,
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
                let mut boundary_summary = formatted;
                match crate::compact_archive::archive_current_project(
                    &to_summarize,
                    pre_tokens,
                    &boundary_summary,
                ) {
                    Ok(Some(meta)) => {
                        let _ = writeln!(
                            &mut boundary_summary,
                            "\nRaw compacted transcript archive: `{}` ({} messages). Use `/expand {}` to recover the exact pre-compaction messages.",
                            meta.id, meta.message_count, meta.id
                        );
                        debug!(
                            target: "jfc::compact",
                            archive_id = %meta.id,
                            path = %meta.path.display(),
                            "archived compacted transcript range"
                        );
                    }
                    Ok(None) => {}
                    Err(error) => {
                        warn!(
                            target: "jfc::compact",
                            error = %error,
                            "failed to archive compacted transcript range"
                        );
                    }
                }
                let summary_msg = ChatMessage::compact_boundary(&boundary_summary, pre_tokens);
                let mut compacted = vec![summary_msg];
                compacted.extend(to_preserve);
                clear_usage_metadata_after_compact(&mut compacted);

                let post_tokens = estimate_tokens(&compacted);

                // If the preserved groups still push us past the blocked
                // threshold, the summary itself didn't help — the recent
                // group's tool outputs are too big to keep verbatim. Drop
                // a preserved group and retry. Without this, a session
                // with a huge final assistant message (e.g. resumed from
                // a long agentic batch with multi-tens-of-KB Read outputs)
                // gets stuck in a compact-resubmit loop because each
                // pass produces a Success that's still over Blocked.
                let blocked = blocked_threshold_with_output(window, max_output_tokens);
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

                if !restored_files.is_empty() {
                    insert_restored_files(&mut compacted, &restored_files);
                    // Recompute post_tokens with the restored files included
                    let post_tokens = estimate_tokens(&compacted);
                    tool_ctx.approx_tokens = post_tokens;
                    if post_tokens >= blocked {
                        warn!(
                            target: "jfc::compact",
                            post_tokens, blocked, restored_files = restored_files.len(),
                            attempts = attempt,
                            "post-compact restored files pushed context back over blocked threshold"
                        );
                        return CompactResult::Exhausted { attempts: attempt };
                    }
                }
                tool_ctx.read_cache.clear();

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

                if is_output_token_limit_error(&err_msg) {
                    warn!(
                        target: "jfc::compact",
                        attempt, error = %e,
                        max_tokens = compact_options.max_tokens,
                        "compaction rejected for output-token limit — not a context-window retry"
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

fn clear_usage_metadata_after_compact(messages: &mut [ChatMessage]) {
    for message in messages {
        message.usage = None;
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

#[derive(Debug, Clone)]
struct SeenSummaryOutput {
    label: usize,
    text: String,
    approx_tokens: usize,
}

#[derive(Debug, Default)]
struct SummaryOutputCompressor {
    by_hash: HashMap<u64, Vec<SeenSummaryOutput>>,
    full_outputs: Vec<SeenSummaryOutput>,
    next_label: usize,
}

impl SummaryOutputCompressor {
    const MIN_COMPRESS_CHARS: usize = 128;
    const MIN_DIFF_PREFIX_CHARS: usize = 256;
    const MAX_DIFF_SUFFIX_CHARS: usize = 1024;

    fn render(&mut self, output_text: &str) -> String {
        if output_text.chars().count() < Self::MIN_COMPRESS_CHARS {
            return output_text.to_owned();
        }

        let hash = jfc_core::semantic_hash::content_hash_bytes(output_text.as_bytes());
        if let Some(seen) = self
            .by_hash
            .get(&hash)
            .and_then(|candidates| candidates.iter().find(|seen| seen.text == output_text))
        {
            return format!(
                "[duplicate of earlier tool output #{} | ~{} tokens omitted]",
                seen.label, seen.approx_tokens
            );
        }

        if let Some((seen, prefix_chars, suffix)) = self.best_prefix_delta(output_text) {
            return format!(
                "[same first {prefix_chars} chars as earlier tool output #{}; new suffix follows]\n{}",
                seen.label, suffix
            );
        }

        let seen = SeenSummaryOutput {
            label: self.next_label,
            text: output_text.to_owned(),
            approx_tokens: output_text.len() / CHARS_PER_TOKEN,
        };
        self.next_label += 1;
        self.by_hash.entry(hash).or_default().push(seen.clone());
        self.full_outputs.push(seen);
        output_text.to_owned()
    }

    fn best_prefix_delta<'a>(
        &'a self,
        output_text: &'a str,
    ) -> Option<(&'a SeenSummaryOutput, usize, String)> {
        let current_chars: Vec<u64> = output_text.chars().map(|ch| ch as u64).collect();
        let mut best: Option<(&SeenSummaryOutput, usize)> = None;
        for seen in &self.full_outputs {
            let seen_chars: Vec<u64> = seen.text.chars().map(|ch| ch as u64).collect();
            let prefix = jfc_core::diff_compression::common_prefix_len(&seen_chars, &current_chars);
            if prefix < Self::MIN_DIFF_PREFIX_CHARS {
                continue;
            }
            if best.is_none_or(|(_, best_prefix)| prefix > best_prefix) {
                best = Some((seen, prefix));
            }
        }

        let (seen, prefix_chars) = best?;
        let suffix: String = output_text.chars().skip(prefix_chars).collect();
        let suffix_chars = suffix.chars().count();
        if suffix_chars == 0 || suffix_chars > Self::MAX_DIFF_SUFFIX_CHARS {
            return None;
        }
        Some((seen, prefix_chars, suffix))
    }
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
    const MAX_TEXT_PART_CHARS: usize = 6000;

    let mut output_compressor = SummaryOutputCompressor::default();

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
                        let output_text = output_compressor.render(&output_text);
                        text.push_str(&format!(
                            "[Tool: {} | Input: {} | Output: {}]\n",
                            tc.kind.label(),
                            input_summary,
                            output_text,
                        ));
                    }
                }
                _ => {
                    let rendered = if strip_media {
                        part.text_only()
                    } else {
                        part.to_display_string()
                    };
                    let rendered = jfc_core::context_management::prune_text_with_sink_window(
                        &rendered,
                        MAX_TEXT_PART_CHARS,
                    );
                    text.push_str(&rendered);
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

    let path_count = sorted_paths.len();
    let candidates = sorted_paths
        .iter()
        .enumerate()
        .map(|(idx, path)| {
            let token_estimate = std::fs::metadata(path)
                .map(|meta| meta.len() / CHARS_PER_TOKEN as u64)
                .unwrap_or(1)
                .max(1);
            jfc_core::context_management::ContextItem {
                id: idx as u64,
                // `sorted_paths` is most-recent first; the synthetic attention
                // model scores later positions as more recent.
                position: path_count.saturating_sub(idx + 1) as u64,
                token_estimate,
                kind: jfc_core::context_management::ContextItemKind::Memory,
                duplicate_count: 0,
            }
        })
        .collect::<Vec<_>>();
    let selected_ids = jfc_core::context_management::select_context_items_by_count(
        &candidates,
        MAX_FILES.min(path_count),
    );

    for (idx, path) in sorted_paths.iter().enumerate() {
        if !selected_ids.contains(&(idx as u64)) {
            continue;
        }
        if total_chars >= MAX_TOTAL_CHARS {
            break;
        }
        let Ok(content) = std::fs::read_to_string(path) else {
            continue;
        };
        let remaining_chars = MAX_TOTAL_CHARS.saturating_sub(total_chars);
        if remaining_chars == 0 {
            break;
        }
        let content_budget = MAX_CHARS_PER_FILE.min(remaining_chars);
        let restored =
            jfc_core::context_management::prune_text_with_sink_window(&content, content_budget);
        let was_pruned = content.chars().count() > content_budget;
        let entry = format!("--- {} ---\n{}", path.display(), restored);
        let entry_len = entry.len();
        total_chars += entry_len;
        results.push(entry);

        debug!(
            target: "jfc::compact",
            path = %path.display(),
            chars = entry_len,
            pruned = was_pruned,
            "restored file post-compact via context salience"
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

/// Insert the post-compact restored-file context block into a freshly-built
/// compacted transcript. `compacted[0]` is the summary boundary and
/// `compacted[1..]` is the preserved recent tail.
///
/// The block is a **user-role** message (the restored files are *supplied
/// context*, not the model's own prior output — the boundary itself is
/// `Role::User`, so this belongs on the user side too) inserted at **index 1**,
/// immediately after the summary boundary and ahead of the preserved tail. That
/// mirrors Claude/OpenClaude's `[boundary, summary, file-context, kept]` shape
/// and keeps the freshest restored context from being appended at the very end
/// where it would shadow the actual last turn. `merge_consecutive_same_role`
/// collapses the boundary + restore pair on the wire.
fn insert_restored_files(compacted: &mut Vec<ChatMessage>, restored_files: &[String]) {
    debug_assert!(
        !compacted.is_empty(),
        "compacted transcript always begins with the summary boundary"
    );
    let restore_text = restored_files.join("\n\n");
    let restore_msg = ChatMessage::user(format!(
        "[Post-compact context restoration — recently accessed files:]\n\n{}",
        restore_text
    ));
    compacted.insert(1, restore_msg);
}

#[cfg(test)]
mod level_tests {
    use super::*;
    use crate::compact::{
        blocked_threshold_with_output, compact_level_with_output, compact_threshold_with_output,
    };
    use futures::stream;
    use jfc_provider::{EventStream, ModelInfo};

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

    struct CatalogCapProvider;

    #[async_trait::async_trait]
    impl Provider for CatalogCapProvider {
        fn name(&self) -> &str {
            "anthropic"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            vec![
                ModelInfo::new("small-summary-model", "Small Summary Model", "anthropic")
                    .with_max_output_tokens(4096usize),
            ]
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for CatalogCapProvider {}

    struct SummaryProvider;

    #[async_trait::async_trait]
    impl Provider for SummaryProvider {
        fn name(&self) -> &str {
            "anthropic"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            vec![]
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(stream::empty()))
        }

        async fn complete(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<jfc_provider::CompletionResponse> {
            Ok(jfc_provider::CompletionResponse {
                content: "<summary>short summary</summary>".to_owned(),
                usage: Default::default(),
                context_signals: None,
                reasoning: None,
            })
        }
    }

    impl jfc_provider::seal::Sealed for SummaryProvider {}

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
    fn threshold_default_is_window_minus_output_headroom_minus_13k_normal() {
        let _g = lock();
        clear_env();
        // v177 parity: effective_window = window - min(max_output, 20k)
        // With None (default 20k): threshold = (200k - 20k) - 13k = 167k
        assert_eq!(compact_threshold(W), 167_000);
        // 1M window: (1M - 20k) - 13k = 967k
        assert_eq!(compact_threshold(1_000_000), 967_000);
    }

    #[test]
    fn levels_match_v177_at_each_boundary_normal() {
        let _g = lock();
        clear_env();
        // v177 parity: effective_window = 200k - 20k = 180k
        // compact threshold = 180k - 13k = 167k
        // warn = compact - 20k = 147k
        // blocked = effective_window - 3k = 177k
        // precompute = 80% of compact = 133_600
        assert_eq!(compact_level(0, W), CompactLevel::Ok);
        assert_eq!(compact_level(133_599, W), CompactLevel::Ok);
        // precompute at 80% of compact threshold ≈ 133_600
        assert_eq!(compact_level(133_600, W), CompactLevel::Precompute);
        assert_eq!(compact_level(146_999, W), CompactLevel::Precompute);
        // warn at compact - 20K = 147K
        assert_eq!(compact_level(147_000, W), CompactLevel::Warn);
        assert_eq!(compact_level(166_999, W), CompactLevel::Warn);
        // compact at effective_window - 13K = 167K
        assert_eq!(compact_level(167_000, W), CompactLevel::Compact);
        assert_eq!(compact_level(176_999, W), CompactLevel::Compact);
        // blocked at effective_window - 3K = 177K
        assert_eq!(compact_level(177_000, W), CompactLevel::Blocked);
        assert_eq!(compact_level(W + 999, W), CompactLevel::Blocked);
    }

    #[test]
    fn compact_summary_options_respect_model_output_cap_regression() {
        let provider = CatalogCapProvider;
        let base = StreamOptions::new("small-summary-model").max_tokens(20_000);
        let opts = compact_stream_options(&provider, &base, "summarize".to_owned());

        assert_eq!(opts.max_tokens, 4096);
    }

    #[test]
    fn output_token_limit_is_not_context_retry_signal_regression() {
        assert!(is_output_token_limit_error(
            "max_tokens: 20000 exceeds maximum allowed number of output tokens 4096"
        ));
        assert!(!is_output_token_limit_error(
            "prompt is too long: 120000 tokens > 100000 maximum"
        ));
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
        // pct=50 → effective = 180k, 50% = 90k (min of 90k and base 167k).
        // warn = compact - 20K = 70K; blocked = effective - 3K = 177K.
        assert_eq!(compact_threshold(W), 90_000);
        assert_eq!(compact_level(69_999, W), CompactLevel::Ok);
        assert_eq!(compact_level(70_000, W), CompactLevel::Warn);
        assert_eq!(compact_level(89_999, W), CompactLevel::Warn);
        assert_eq!(compact_level(90_000, W), CompactLevel::Compact);
        assert_eq!(compact_level(177_000, W), CompactLevel::Blocked);
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
        // 99% of effective_window (180K) = 178.2K, but compact base = 167K → min wins.
        assert_eq!(compact_threshold(W), 167_000);
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
        // Even at 170K (would be compact at 167k), level should fall back to warn —
        // user disabled auto-compact, but blocked still applies (it's a hard
        // API constraint, not a preference).
        assert_eq!(compact_level(170_000, W), CompactLevel::Warn);
        // Blocked still applies though (at effective_window - 3k = 177k).
        assert_eq!(compact_level(177_000, W), CompactLevel::Blocked);
        clear_env();
    }

    #[test]
    fn small_window_saturates_without_underflow_robust() {
        let _g = lock();
        clear_env();
        // A 5K window can't even hold 20K output headroom — saturating arithmetic
        // collapses effective_window to 0, then compact and blocked thresholds
        // also collapse to 0. Everything is "blocked" territory.
        // Importantly: no panic, no underflow.
        assert_eq!(compact_threshold(5_000), 0);
        // At 1 token with all thresholds at 0, we're in Blocked territory.
        assert_eq!(compact_level(1, 5_000), CompactLevel::Blocked);
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

    use crate::types::{
        ChatMessage, MessagePart, ModelUsage, ToolCall, ToolDisplayState, ToolInput, ToolKind,
        ToolOutput, ToolStatus,
    };

    fn user_msg(text: &str) -> ChatMessage {
        ChatMessage::user(text.to_owned())
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        ChatMessage::assistant(text.to_owned())
    }

    fn tool_msg(output: &str) -> ChatMessage {
        ChatMessage::assistant_parts(vec![MessagePart::tool(ToolCall {
            id: crate::ids::ToolId::from("tool_1"),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Generic {
                summary: "run command".into(),
            },
            output: ToolOutput::Text(output.to_owned()),
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })])
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

    // Robust: estimate_tokens divides after summing visible chars. Dividing
    // each message first loses remainders and underestimates fragmented
    // transcripts; TokenEstimation.v proves the aggregate formula.
    #[test]
    fn estimate_tokens_uses_aggregate_chars_robust() {
        let messages = vec![user_msg("abc"), assistant_msg("de")];
        // total chars = 5 -> base = 1 -> overhead floor(1 * 3 / 2) = 1.
        // A per-message floor would be 0 + 0 -> 0.
        assert_eq!(estimate_tokens(&messages), 1);
    }

    // Normal: estimate_tokens on empty input returns 0.
    #[test]
    fn estimate_tokens_empty_is_zero_normal() {
        assert_eq!(estimate_tokens(&[]), 0);
    }

    #[test]
    fn summary_text_deduplicates_repeated_tool_outputs_normal() {
        let repeated = format!("header\n{}\nfooter", "same output line\n".repeat(20));
        let messages = vec![tool_msg(&repeated), tool_msg(&repeated)];
        let summary = build_summary_text(&messages, false);
        assert!(summary.contains("same output line"));
        assert!(summary.contains("duplicate of earlier tool output #0"));
    }

    #[test]
    fn summary_text_uses_prefix_delta_for_similar_tool_outputs_normal() {
        let shared = "shared-prefix-line\n".repeat(20);
        let first = format!("{shared}first unique suffix");
        let second = format!("{shared}second unique suffix");
        let messages = vec![tool_msg(&first), tool_msg(&second)];
        let summary = build_summary_text(&messages, false);
        assert!(summary.contains("same first"));
        assert!(summary.contains("earlier tool output #0"));
        assert!(summary.contains("second unique suffix"));
    }

    #[test]
    fn summary_text_large_outputs_still_take_mask_path_robust() {
        let large = "x".repeat(2_500);
        let messages = vec![tool_msg(&large), tool_msg(&large)];
        let summary = build_summary_text(&messages, false);
        assert!(summary.contains("Output: ~625 tokens, truncated"));
        assert!(!summary.contains("duplicate of earlier tool output"));
    }

    #[test]
    fn summary_text_prunes_long_regular_text_with_sink_window_normal() {
        let long = format!(
            "{}{}{}",
            "A".repeat(2_000),
            "M".repeat(8_000),
            "Z".repeat(2_000)
        );
        let summary = build_summary_text(&[user_msg(&long)], false);
        assert!(summary.contains("omitted"));
        assert!(summary.contains(&"A".repeat(100)));
        assert!(summary.contains(&"Z".repeat(100)));
        assert!(summary.len() < long.len());
        assert!(!summary.contains(&"M".repeat(4_000)));
    }

    // Normal: the recency preserve floor keeps the newest groups that fit ~30%
    // of the window, instead of preserving only the last group. 5 groups of 100
    // tokens, window 1000 → budget 300 → keep newest 3.
    #[test]
    fn recency_preserve_floor_keeps_recent_groups_normal() {
        let group_tokens = vec![100usize, 100, 100, 100, 100];
        assert_eq!(recency_preserve_floor(&group_tokens, 1000), 3);
    }

    // Normal: the recency floor preserves strictly MORE newest-context tokens
    // than the old preserve_count=1 baseline — the measured RCT win. 5 groups of
    // 100 tokens, window 1000 (budget 300) → floor keeps 3 groups (300 tokens)
    // vs the baseline's 1 group (100 tokens): +200 recent tokens kept verbatim.
    #[test]
    fn measure_recency_floor_preserves_more_recent_tokens_normal() {
        let group_tokens = vec![100usize, 100, 100, 100, 100];
        let m = measure_recency_floor(&group_tokens, 1000);
        assert_eq!(m.groups_preserved_with_floor, 3);
        assert_eq!(m.tokens_preserved_with_floor, 300);
        assert_eq!(m.tokens_preserved_baseline, 100);
        assert_eq!(m.extra_recent_tokens_preserved(), 200);
        // The direction is the whole point: the floor never preserves less.
        assert!(m.tokens_preserved_with_floor >= m.tokens_preserved_baseline);
    }

    // Robust: when groups are huge relative to the window, the floor collapses
    // to the baseline (1 group) and the measured win is 0 — no false positive.
    #[test]
    fn measure_recency_floor_no_win_when_groups_oversized_robust() {
        let group_tokens = vec![5000usize, 5000, 5000];
        let m = measure_recency_floor(&group_tokens, 100);
        assert_eq!(m.groups_preserved_with_floor, 1);
        assert_eq!(m.extra_recent_tokens_preserved(), 0);
    }

    // Robust: a tiny window still preserves at least one group and always leaves
    // at least one to summarize (never returns total).
    #[test]
    fn recency_preserve_floor_clamps_bounds_robust() {
        let group_tokens = vec![5000usize, 5000, 5000];
        // Budget (30% of 100 = 30) fits no full group → clamp up to 1.
        assert_eq!(recency_preserve_floor(&group_tokens, 100), 1);
        // Huge window would keep all, but must leave 1 to summarize.
        let floor = recency_preserve_floor(&group_tokens, 10_000_000);
        assert_eq!(floor, group_tokens.len() - 1);
        // A single group always returns 1.
        assert_eq!(recency_preserve_floor(&[42], 1000), 1);
    }

    #[test]
    fn provider_attention_signals_raise_preserve_floor_normal() {
        let messages = vec![
            user_msg("g0"),
            assistant_msg("a0"),
            user_msg("important old group"),
            assistant_msg("a1"),
            user_msg("g2"),
            assistant_msg("a2"),
            user_msg("g3"),
            assistant_msg("a3"),
        ];
        let groups = split_into_groups(&messages);
        let group_tokens = vec![100usize, 100, 100, 100];
        let signals = jfc_core::context_management::ContextSignals {
            attention_tokens: vec![jfc_core::attention::ScoredToken {
                token_id: 1,
                token_position: 1,
                attention_weight: 1000,
            }],
            kv_entries: Vec::new(),
        };

        assert_eq!(
            provider_signal_preserve_floor(&groups, &group_tokens, 500, &signals),
            Some(3)
        );
    }

    #[test]
    fn provider_kv_signals_take_precedence_over_attention_normal() {
        let messages = vec![
            user_msg("g0"),
            assistant_msg("a0"),
            user_msg("attention wants this"),
            assistant_msg("a1"),
            user_msg("kv wants this"),
            assistant_msg("a2"),
            user_msg("g3"),
            assistant_msg("a3"),
        ];
        let groups = split_into_groups(&messages);
        let group_tokens = vec![100usize, 100, 100, 100];
        let signals = jfc_core::context_management::ContextSignals {
            attention_tokens: vec![jfc_core::attention::ScoredToken {
                token_id: 1,
                token_position: 1,
                attention_weight: 1000,
            }],
            kv_entries: vec![jfc_core::kv_cache::KVEntry {
                position: 2,
                layer: 0,
                attention_score: 1000,
                recent_access: 100,
                size_bytes: 400,
            }],
        };

        assert_eq!(
            provider_signal_preserve_floor(&groups, &group_tokens, 500, &signals),
            Some(2)
        );
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

    #[test]
    fn compact_clears_preserved_tail_usage_robust() {
        let mut preserved = assistant_msg("preserved assistant");
        preserved.usage = Some(ModelUsage {
            input_tokens: 180_000,
            output_tokens: 1_000,
            thinking_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        });
        let mut compacted = vec![
            ChatMessage::compact_boundary("summary", 180_000),
            user_msg("preserved user"),
            preserved,
        ];

        clear_usage_metadata_after_compact(&mut compacted);

        assert!(
            compacted.iter().all(|message| message.usage.is_none()),
            "stale pre-compact usage must not survive into compacted transcripts"
        );
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

    // Robust: when the newest `split` groups can cover a positive gap, the
    // returned step covers it, matching CompressionBounds.token_gap_step_covers.
    #[test]
    fn token_gap_step_covers_gap_when_possible_robust() {
        let group_tokens = vec![50usize, 125, 250, 500, 1000];
        let split = 4;
        let gap = 700;
        let step = token_gap_step(Some(gap), &group_tokens, split);
        let covered: usize = group_tokens[..split].iter().rev().take(step).sum();
        assert!(covered >= gap, "step {step} covered {covered}, gap {gap}");
    }

    // Robust: the accumulator saturates instead of overflowing when token
    // estimates are pathological. Nat proofs assume unbounded arithmetic; this
    // is the Rust-side counterpart.
    #[test]
    fn token_gap_step_saturates_pathological_counts_robust() {
        let group_tokens = vec![usize::MAX, usize::MAX];
        assert_eq!(token_gap_step(Some(usize::MAX), &group_tokens, 2), 1);
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
        // v177: effective = 180K, compact threshold = 167K.
        assert!(!should_compact(166_999, W));
        assert!(should_compact(167_000, W));
        assert!(should_compact(180_000, W));
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
        // At 170K, would be Compact normally, but disabled → Warn.
        assert!(!should_compact(170_000, W));
        // Blocked at effective_window - 3k = 177k still fires regardless.
        assert!(should_compact(177_000, W));
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
        assert_eq!(compact_threshold(W), 167_000);
        unsafe {
            std::env::set_var("JFC_AUTOCOMPACT_PCT_OVERRIDE", "0");
        }
        assert_eq!(compact_threshold(W), 167_000);
        unsafe {
            std::env::set_var("JFC_AUTOCOMPACT_PCT_OVERRIDE", "200");
        }
        // Out of range — ignored.
        assert_eq!(compact_threshold(W), 167_000);
        clear_env();
    }

    // Normal: v177 parity — explicit max_output_tokens affects threshold.
    #[test]
    fn threshold_with_explicit_output_matches_cc177_normal() {
        let _g = lock();
        clear_env();
        // CC 177 formula: Q9H = window - min(maxOutput, 20k), mu8 = Q9H - 13k
        // With explicit 64k output (capped to 20k): (200k - 20k) - 13k = 167k
        assert_eq!(compact_threshold_with_output(W, Some(64_000)), 167_000);
        // With explicit 8k output: (200k - 8k) - 13k = 179k
        assert_eq!(compact_threshold_with_output(W, Some(8_000)), 179_000);
        // With explicit 0 output: (200k - 0) - 13k = 187k
        assert_eq!(compact_threshold_with_output(W, Some(0)), 187_000);
        // With None (defaults to 20k cap): (200k - 20k) - 13k = 167k
        assert_eq!(compact_threshold_with_output(W, None), 167_000);
    }

    // Normal: v177 parity — compact_level_with_output uses the correct effective window.
    #[test]
    fn level_with_output_uses_correct_thresholds_normal() {
        let _g = lock();
        clear_env();
        // Model with 8k output: effective = 200k - 8k = 192k
        // compact = 192k - 13k = 179k
        // precompute = 80% of 179k = 143_200
        // warn = 179k - 20k = 159k
        // blocked = 192k - 3k = 189k
        assert_eq!(
            compact_level_with_output(143_199, W, Some(8_000)),
            CompactLevel::Ok
        );
        assert_eq!(
            compact_level_with_output(143_200, W, Some(8_000)),
            CompactLevel::Precompute
        );
        assert_eq!(
            compact_level_with_output(159_000, W, Some(8_000)),
            CompactLevel::Warn
        );
        assert_eq!(
            compact_level_with_output(179_000, W, Some(8_000)),
            CompactLevel::Compact
        );
        assert_eq!(
            compact_level_with_output(189_000, W, Some(8_000)),
            CompactLevel::Blocked
        );
    }

    #[test]
    fn blocked_threshold_with_output_matches_level_boundary_robust() {
        let _g = lock();
        clear_env();
        assert_eq!(blocked_threshold_with_output(W, Some(8_000)), 189_000);
        assert_eq!(
            compact_level_with_output(188_999, W, Some(8_000)),
            CompactLevel::Compact
        );
        assert_eq!(
            compact_level_with_output(189_000, W, Some(8_000)),
            CompactLevel::Blocked
        );
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

    // ─── post-compact restored-file placement (regression: fix #5) ──────

    use crate::types::Role;

    /// Build a minimal compacted transcript: [boundary, preserved tail…].
    fn compacted_with_tail() -> Vec<ChatMessage> {
        vec![
            ChatMessage::compact_boundary("summary", 120_000),
            ChatMessage::assistant("preserved assistant turn".into()),
            ChatMessage::user("the most recent user turn".into()),
        ]
    }

    // Regression: restored files land as a USER-role block (not assistant —
    // that made the model treat supplied files as its own prior output).
    #[test]
    fn restored_files_are_user_role_robust() {
        let mut compacted = compacted_with_tail();
        insert_restored_files(&mut compacted, &["--- a.rs ---\nfn main(){}".to_owned()]);
        assert_eq!(
            compacted[1].role,
            Role::User,
            "restored-file context must be user-role supplied context"
        );
    }

    // Regression: the block is inserted at index 1 (right after the summary
    // boundary, AHEAD of the preserved tail) — not appended at the end where
    // it would shadow the actual last turn.
    #[test]
    fn restored_files_inserted_after_boundary_before_tail_robust() {
        let mut compacted = compacted_with_tail();
        let before_len = compacted.len();
        insert_restored_files(&mut compacted, &["--- a.rs ---\ncontents".to_owned()]);

        assert_eq!(compacted.len(), before_len + 1);
        // [0] boundary, [1] restored files, [2..] preserved tail unchanged.
        assert!(compacted[0].is_compact_boundary(), "boundary stays first");
        let restored_text: String = compacted[1]
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(
            restored_text.contains("Post-compact context restoration"),
            "index 1 must be the restored-file block"
        );
        // The original last turn is still the last message — not shadowed.
        let last_text: String = compacted
            .last()
            .unwrap()
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(last_text, "the most recent user turn");
    }

    // The restored block carries every file joined, with the restoration
    // marker prefix.
    #[test]
    fn restored_files_join_all_entries_normal() {
        let mut compacted = compacted_with_tail();
        insert_restored_files(
            &mut compacted,
            &["--- a.rs ---\nA".to_owned(), "--- b.rs ---\nB".to_owned()],
        );
        let text: String = compacted[1]
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Text(t) => Some(t.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("a.rs"));
        assert!(text.contains("b.rs"));
    }

    #[tokio::test]
    async fn compact_exhausts_when_restored_files_exceed_blocked_budget_regression() {
        let _g = lock();
        clear_env();
        unsafe {
            std::env::set_var("JFC_BLOCKING_LIMIT_OVERRIDE", "300");
        }

        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("large.rs");
        std::fs::write(&file, "fn restored() {}\n".repeat(1_000)).unwrap();

        let provider = SummaryProvider;
        let opts = StreamOptions::new("claude-summary");
        let mut ctx = ToolContext::default();
        ctx.read_cache.record_read(file);
        let messages = vec![
            user_msg("first user turn"),
            assistant_msg("first assistant turn"),
            user_msg("latest user turn"),
            assistant_msg("latest assistant turn"),
        ];

        let result = compact(&messages, &provider, &opts, &mut ctx, 200_000, None, None).await;

        assert!(matches!(result, CompactResult::Exhausted { .. }));
        assert!(
            !ctx.read_cache.paths().is_empty(),
            "failed post-restore compaction must not clear read cache"
        );

        clear_env();
    }
}

#[cfg(test)]
mod circuit_breaker_tests {
    use super::*;
    use crate::context::ToolContext;
    use crate::types::ChatMessage;

    /// A provider that must NEVER be called: the circuit-breaker trip and the
    /// "too few groups" guard both short-circuit before any provider use, so a
    /// panic here proves the early return fired.
    struct NeverCalledProvider;

    #[async_trait::async_trait]
    impl jfc_provider::Provider for NeverCalledProvider {
        fn name(&self) -> &str {
            "anthropic"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            panic!("provider.stream must not be called once the breaker has tripped");
        }
    }
    impl jfc_provider::seal::Sealed for NeverCalledProvider {}

    #[tokio::test]
    async fn breaker_trips_at_limit_before_calling_provider_normal() {
        let provider = NeverCalledProvider;
        let opts = StreamOptions::new("claude-opus-4-8");
        let mut ctx = ToolContext {
            rapid_refill_count: CIRCUIT_BREAKER_LIMIT,
            // total_user_turns == last_compact_turn → turns_since_compact == 0,
            // so the recovery reset (needs >= THRASH_TURN_WINDOW) does NOT fire.
            total_user_turns: 0,
            last_compact_turn: 0,
            ..Default::default()
        };
        let messages = vec![
            ChatMessage::user("hello".to_owned()),
            ChatMessage::assistant("hi".to_owned()),
        ];
        let result = compact(&messages, &provider, &opts, &mut ctx, 200_000, None, None).await;
        assert!(matches!(result, CompactResult::CircuitBreakerTripped));
    }

    #[tokio::test]
    async fn breaker_auto_clears_after_thrash_window_normal() {
        let provider = NeverCalledProvider;
        let opts = StreamOptions::new("claude-opus-4-8");
        // Tripped count, but enough turns have elapsed → recovery resets it to 0
        // and we fall through to the (provider-free) TooFewGroups guard.
        let mut ctx = ToolContext {
            rapid_refill_count: CIRCUIT_BREAKER_LIMIT,
            total_user_turns: THRASH_TURN_WINDOW + 1,
            last_compact_turn: 0,
            ..Default::default()
        };
        let messages = vec![ChatMessage::user("hello".to_owned())];
        let result = compact(&messages, &provider, &opts, &mut ctx, 200_000, None, None).await;
        // The breaker cleared, so it did NOT return CircuitBreakerTripped.
        assert!(!matches!(result, CompactResult::CircuitBreakerTripped));
        assert_eq!(ctx.rapid_refill_count, 0, "breaker should auto-clear");
    }
}
