use std::{sync::Arc, time::Duration};

use tokio::sync::mpsc;

use crate::runtime::{
    EngineEvent, StreamEvent, StreamLifecyclePhase, StreamLifecycleStatus, StreamRequestOverrides,
};
use jfc_provider::{ModelId, Provider, ProviderMessage, StopReason, StreamOptions};

use super::{live_events, open_stream_with_bedrock_retries, prepare_stream_request};

// `previous_response_id` chaining was removed from the OpenAI Responses
// transmit path. It produced `previous_response_not_found` 400s because the
// request body sets `"store": false` (see `providers/openai.rs`), which means
// OpenAI never persists the prior response server-side and can't honor the
// chain reference. It was also redundant: JFC sends the full conversation
// history (`responses_input(messages)`) on every turn, so there is no
// server-side state for the API to *continue from* in the first place.
//
// If we ever need true server-side chaining (e.g., to skip resending a huge
// context), we must (a) set `store: true` on the body, (b) accept the privacy
// trade-off, and (c) re-introduce a chain-id store keyed by conversation —
// not a single process-global slot.

#[tracing::instrument(
    target = "jfc::stream",
    skip_all,
    fields(
        provider = %provider.name(),
        model = %model,
        messages = messages.len(),
    ),
)]
pub async fn stream_response(
    provider: Arc<dyn Provider>,
    messages: Vec<ProviderMessage>,
    model: ModelId,
    tx: mpsc::Sender<EngineEvent>,
    interrupt: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // wg-async pattern: spawned tasks holding critical state need an
    // explicit cancellation handle, not just a polled flag. The token
    // races the SSE stream against `.cancelled()` so ESC×2 unwinds in
    // microseconds instead of waiting for the next STREAM_INTERRUPT_POLL.
    cancel: tokio_util::sync::CancellationToken,
    previous_message_id: Option<String>,
    overrides: StreamRequestOverrides,
) {
    let _ = tx.try_send(EngineEvent::Stream(StreamEvent::Lifecycle(
        StreamLifecycleStatus::new(
            StreamLifecyclePhase::PreparingContext,
            Some(format!("{} messages", messages.len())),
        ),
    )));

    let prepared = prepare_stream_request(provider.clone(), &messages, &model, overrides).await;
    let mut opts = prepared.opts;
    if let Some(id) = previous_message_id {
        // Claude Code 2.1.177 pairs diagnostics.previous_message_id with the
        // cache-diagnosis beta so cache-read drops can be attributed by the API.
        opts.cache_diagnosis = true;
        opts.previous_message_id = Some(id);
    }

    // Audit: record the provider/model call to the runtime ledger so spend +
    // "which model did what" is queryable. Offloaded so the locked append
    // never blocks the stream's first byte.
    {
        let model_label = model.to_string();
        tokio::task::spawn_blocking(move || {
            crate::changeset::record_provider_call(&model_label, None);
        });
    }

    // Report system prompt size back to App for post-compaction overhead estimate.
    let _ = tx.try_send(EngineEvent::Stream(StreamEvent::SystemPromptLen(
        prepared.system_prompt_tokens,
    )));
    // Tell the user when this turn pulled in recalled memory.
    if prepared.recalled_memory_chars > 0 {
        let _ = tx.try_send(EngineEvent::Stream(StreamEvent::MemoryRecalled(
            prepared.recalled_memory_chars,
        )));
    }
    if tx
        .send(EngineEvent::Stream(StreamEvent::RequestMetadata(
            prepared.metadata.clone(),
        )))
        .await
        .is_err()
    {
        return;
    }
    // (was: inject `previous_response_id` into provider_options for OpenAI.
    // Removed — incompatible with `store: false` and redundant with
    // full-history `input`. See note at the top of this file.)

    // v132 BeforeStream hook fires after the prompt is fully assembled
    // but before the network call. Handlers that want to inject system
    // reminders, gate on cost budgets, or pre-compact the context can
    // do so here. Default registry is Logger-only so production behavior
    // is byte-for-byte identical when no user hooks are configured.
    crate::hooks::fire(
        crate::hooks::HookPoint::BeforeStream,
        &crate::hooks::HookContext::for_session("stream")
            .with_extra("model", model.as_str().to_string())
            .with_extra("message_count", messages.len().to_string()),
    );

    // Wrap in Arc so the retry loop and thinking-fallback path share the same
    // allocation instead of cloning the full Vec<ProviderMessage> on each attempt.
    let messages = Arc::new(messages);
    let _ = tx.try_send(EngineEvent::Stream(StreamEvent::Lifecycle(
        StreamLifecycleStatus::new(
            StreamLifecyclePhase::WaitingForFirstByte,
            Some(format!("{} · {}", provider.name(), model)),
        ),
    )));
    let stream = match open_stream_with_cancel_and_timeout(
        provider.as_ref(),
        Arc::clone(&messages),
        &opts,
        cancel.clone(),
    )
    .await
    {
        Ok(s) => {
            tracing::debug!(target: "jfc::stream", "stream opened successfully");
            let _ = tx.try_send(EngineEvent::Stream(StreamEvent::Lifecycle(
                StreamLifecycleStatus::new(
                    StreamLifecyclePhase::StreamOpened,
                    Some("waiting for first event".to_string()),
                ),
            )));
            s
        }
        Err(e) => {
            let err_lower = e.to_string().to_lowercase();
            if (err_lower.contains("thinking") && err_lower.contains("not supported"))
                || err_lower.contains("adaptive thinking is not supported")
            {
                tracing::warn!(
                    target: "jfc::stream",
                    model = %model,
                    error = %e,
                    "stream rejected thinking parameter — retrying without thinking"
                );
                let mut fallback_opts = opts.clone();
                fallback_opts.adaptive_thinking = false;
                fallback_opts.thinking_budget = None;
                fallback_opts.thinking_display = None;
                let _ = tx.try_send(EngineEvent::Stream(StreamEvent::Lifecycle(
                    StreamLifecycleStatus::new(
                        StreamLifecyclePhase::RetryingWithoutThinking,
                        Some(model.to_string()),
                    ),
                )));
                match open_stream_with_cancel_and_timeout(
                    provider.as_ref(),
                    Arc::clone(&messages),
                    &fallback_opts,
                    cancel.clone(),
                )
                .await
                {
                    Ok(s) => s,
                    Err(e2) => {
                        if try_nonstreaming_fallback(
                            provider.as_ref(),
                            Arc::clone(&messages),
                            &fallback_opts,
                            &tx,
                            &e2.to_string(),
                            prepared.metadata.advertised_tool_count,
                            prepared.metadata.action_expected,
                        )
                        .await
                        {
                            return;
                        }
                        tracing::error!(target: "jfc::stream", error = %e2, "stream open failed (fallback without thinking)");
                        let _ = tx
                            .send(EngineEvent::Stream(StreamEvent::Error(e2.to_string())))
                            .await;
                        return;
                    }
                }
            } else if err_lower.contains("prompt is too long")
                || err_lower.contains("prompt_too_long")
                || err_lower.contains("input length")
                || err_lower.contains("max_tokens")
                || err_lower.contains("context window")
            {
                // v132 mid-stream compaction trigger. The pre-submit
                // path catches the obvious cases via `compact_level`,
                // but estimator drift means the API can still reject
                // a turn with prompt_too_long. Surface a system-level
                // signal so the main loop fires `/compact` and re-
                // queues the same prompt; the user sees a brief toast
                // instead of a hard failure.
                tracing::warn!(
                    target: "jfc::stream",
                    error = %e,
                    "stream rejected: prompt too long — requesting auto-compact"
                );
                let _ = tx
                    .send(EngineEvent::Stream(StreamEvent::Error(format!(
                        "auto-compact: {e}"
                    ))))
                    .await;
                return;
            } else {
                if try_nonstreaming_fallback(
                    provider.as_ref(),
                    Arc::clone(&messages),
                    &opts,
                    &tx,
                    &e.to_string(),
                    prepared.metadata.advertised_tool_count,
                    prepared.metadata.action_expected,
                )
                .await
                {
                    return;
                }
                tracing::error!(target: "jfc::stream", error = %e, "stream open failed");
                let _ = tx
                    .send(EngineEvent::Stream(StreamEvent::Error(e.to_string())))
                    .await;
                return;
            }
        }
    };

    // Drive the stream to completion. A connection that dies *before any event
    // arrived* (dead socket through a NAT/LB, proxy buffering forever) is
    // re-opened in place up to MAX_PRE_FIRST_EVENT_STREAM_RETRIES before we
    // degrade the whole turn to non-streaming — mirrors Claude 2.1.177's
    // stale-connection (`o$`) and watchdog (`z6`) pre-first-event retry
    // counters. Once output has streamed, re-opening would duplicate
    // transcript text, so a committed-output error never re-opens.
    let mut current_stream = stream;
    let mut pre_first_event_attempt = 0u32;
    let stop_reason = loop {
        match live_events::drain_stream_events(
            current_stream,
            &tx,
            interrupt.clone(),
            cancel.clone(),
        )
        .await
        {
            live_events::DrainOutcome::Done(stop_reason) => break stop_reason,
            live_events::DrainOutcome::Cancelled(message) => {
                let _ = tx
                    .send(EngineEvent::Stream(StreamEvent::Error(message)))
                    .await;
                return;
            }
            live_events::DrainOutcome::Error {
                message,
                committed_output,
            } => {
                if should_retry_pre_first_event(&message, committed_output, pre_first_event_attempt)
                {
                    pre_first_event_attempt += 1;
                    let cause = classify_fallback_cause(&message, committed_output);
                    tracing::warn!(
                        target: "jfc::stream::fallback",
                        retry_attempt = pre_first_event_attempt,
                        max = MAX_PRE_FIRST_EVENT_STREAM_RETRIES,
                        fallback_cause = cause,
                        error = %message,
                        "tengu_streaming_stale_connection_retry: stream died before first event — re-opening streaming connection"
                    );
                    let _ = tx.try_send(EngineEvent::Stream(StreamEvent::Lifecycle(
                        StreamLifecycleStatus::new(
                            StreamLifecyclePhase::WaitingForFirstByte,
                            Some(format!(
                                "reconnecting ({pre_first_event_attempt}/{MAX_PRE_FIRST_EVENT_STREAM_RETRIES})"
                            )),
                        ),
                    )));
                    // Linear backoff (100ms * attempt) before re-opening, the
                    // same shape as upstream's `l8(100 * e$)`.
                    tokio::time::sleep(Duration::from_millis(
                        100 * u64::from(pre_first_event_attempt),
                    ))
                    .await;
                    match open_stream_with_cancel_and_timeout(
                        provider.as_ref(),
                        Arc::clone(&messages),
                        &opts,
                        cancel.clone(),
                    )
                    .await
                    {
                        Ok(s) => {
                            current_stream = s;
                            continue;
                        }
                        Err(reopen_err) => {
                            tracing::warn!(
                                target: "jfc::stream::fallback",
                                error = %reopen_err,
                                "stream re-open failed after pre-first-event retry — degrading to non-streaming"
                            );
                            // Fall through to the non-streaming fallback below
                            // using the ORIGINAL error message/cause.
                        }
                    }
                }

                if !committed_output {
                    let cause = classify_fallback_cause(&message, committed_output);
                    tracing::warn!(
                        target: "jfc::stream::fallback",
                        fallback_cause = cause,
                        error = %message,
                        "tengu_nonstreaming_fallback_started: degrading to non-streaming completion"
                    );
                    if try_nonstreaming_fallback(
                        provider.as_ref(),
                        Arc::clone(&messages),
                        &opts,
                        &tx,
                        &message,
                        prepared.metadata.advertised_tool_count,
                        prepared.metadata.action_expected,
                    )
                    .await
                    {
                        return;
                    }
                } else {
                    tracing::warn!(
                        target: "jfc::stream::fallback",
                        fallback_cause = "partial_yield",
                        error = %message,
                        "stream failed after output was committed; skipping non-streaming fallback to avoid duplicate transcript text"
                    );
                }
                let _ = tx
                    .send(EngineEvent::Stream(StreamEvent::Error(message)))
                    .await;
                return;
            }
        }
    };

    tracing::info!(
        target: "jfc::stream",
        ?stop_reason,
        "stream finished — sending StreamDone"
    );
    // (was: clear `openai_previous_response_id` on end_turn. Removed alongside
    // the inject path — no state to clear.)

    // v132 AfterStream hook — fires after the model finished streaming
    // but before the StreamDone EngineEvent is sent. Handlers that want
    // to surface end-of-turn cost, run telemetry batching, or trigger
    // session auto-naming can land here.
    crate::hooks::fire(
        crate::hooks::HookPoint::AfterStream,
        &crate::hooks::HookContext::for_session("stream")
            .with_extra("stop_reason", format!("{stop_reason:?}")),
    );

    let _ = tx
        .send(EngineEvent::Stream(StreamEvent::Done(stop_reason)))
        .await;
}

async fn open_stream_with_cancel_and_timeout(
    provider: &dyn Provider,
    messages: Arc<Vec<ProviderMessage>>,
    opts: &StreamOptions,
    cancel: tokio_util::sync::CancellationToken,
) -> anyhow::Result<jfc_provider::EventStream> {
    let open = open_stream_with_bedrock_retries(provider, messages, opts);
    let timeout = stream_open_timeout();
    tokio::pin!(open);
    if let Some(timeout) = timeout {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => anyhow::bail!("Stream cancelled before connection opened"),
            result = &mut open => result,
            _ = tokio::time::sleep(timeout) => anyhow::bail!(
                "Stream open timed out after {}s before first provider response",
                timeout.as_secs()
            ),
        }
    } else {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => anyhow::bail!("Stream cancelled before connection opened"),
            result = &mut open => result,
        }
    }
}

fn stream_open_timeout() -> Option<Duration> {
    match std::env::var("JFC_STREAM_OPEN_TIMEOUT_SECS") {
        Ok(raw) if matches!(raw.as_str(), "0" | "off" | "false" | "disabled") => None,
        Ok(raw) => raw
            .parse::<u64>()
            .ok()
            .filter(|secs| *secs > 0)
            .map(Duration::from_secs),
        Err(_) => Some(Duration::from_secs(45)),
    }
}

fn stream_to_nonstreaming_fallback_enabled() -> bool {
    if std::env::var("JFC_DISABLE_STREAMING_TO_NONSTREAMING_FALLBACK")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
        return false;
    }
    std::env::var("JFC_STREAMING_TO_NONSTREAMING_FALLBACK")
        .map(|v| !matches!(v.as_str(), "0" | "false" | "no" | "off"))
        .unwrap_or(true)
}

fn error_looks_stream_stale_or_idle(error: &str) -> bool {
    let lower = error.to_ascii_lowercase();
    lower.contains("stream timed out")
        || lower.contains("stream open timed out")
        || lower.contains("before first event")
        || lower.contains("before first byte")
        || lower.contains("first provider response")
        || lower.contains("first sse")
        || lower.contains("idle")
        || lower.contains("stalled")
        || lower.contains("connection closed")
        || lower.contains("unexpected eof")
        || lower.contains("incomplete")
        || lower.contains("broken pipe")
        || lower.contains("connection reset")
        || lower.contains("body error")
        || lower.contains("decode")
}

/// Maximum number of pre-first-event stream re-opens before degrading to the
/// non-streaming fallback. Mirrors Claude 2.1.177's stale-connection /
/// watchdog pre-first-event retry counters (`o$` / `z6`): a connection that
/// dies *before any event arrived* is almost always a dead socket through a
/// NAT/LB, so re-opening the stream is cheaper and cleaner than immediately
/// degrading the whole turn to non-streaming. Bounded so a persistently
/// failing route still falls back instead of looping.
const MAX_PRE_FIRST_EVENT_STREAM_RETRIES: u32 = 2;

/// Classify why a stream is degrading to non-streaming, mirroring Claude
/// 2.1.177's `fallback_cause` field on `tengu_nonstreaming_fallback_started`.
/// Used purely for telemetry/observability.
fn classify_fallback_cause(error: &str, committed_output: bool) -> &'static str {
    if committed_output {
        return "partial_yield";
    }
    let lower = error.to_ascii_lowercase();
    if lower.contains("before first event")
        || lower.contains("before first byte")
        || lower.contains("first provider response")
        || lower.contains("first sse")
        || lower.contains("ended before")
        || lower.contains("without receiving")
    {
        "stream_no_events"
    } else if lower.contains("connection closed")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("unexpected eof")
    {
        "stale_connection"
    } else if lower.contains("idle") || lower.contains("stalled") || lower.contains("timed out") {
        "watchdog"
    } else {
        "other"
    }
}

/// Whether a pre-first-event stream error is eligible for an in-place stream
/// re-open (vs. degrading straight to non-streaming). Only stale/idle-shaped
/// errors qualify, and only when nothing was committed — once output streamed,
/// re-opening would duplicate transcript text.
fn should_retry_pre_first_event(error: &str, committed_output: bool, attempt: u32) -> bool {
    !committed_output
        && attempt < MAX_PRE_FIRST_EVENT_STREAM_RETRIES
        && error_looks_stream_stale_or_idle(error)
}

async fn try_nonstreaming_fallback(
    provider: &dyn Provider,
    messages: Arc<Vec<ProviderMessage>>,
    opts: &StreamOptions,
    tx: &tokio::sync::mpsc::Sender<EngineEvent>,
    error: &str,
    advertised_tool_count: usize,
    action_expected: bool,
) -> bool {
    if !stream_to_nonstreaming_fallback_enabled() || !error_looks_stream_stale_or_idle(error) {
        return false;
    }
    if action_expected && advertised_tool_count > 0 {
        tracing::warn!(
            target: "jfc::stream::fallback",
            advertised_tool_count,
            error = %error,
            "stream failed but non-streaming fallback skipped because tool execution may be required"
        );
        return false;
    }
    tracing::warn!(
        target: "jfc::stream::fallback",
        error = %error,
        "stream failed; trying non-streaming completion fallback"
    );
    let _ = tx
        .send(EngineEvent::Stream(StreamEvent::Lifecycle(
            StreamLifecycleStatus::new(
                StreamLifecyclePhase::NonStreamingFallback,
                Some(provider.name().to_string()),
            ),
        )))
        .await;
    let response = match provider.complete((*messages).clone(), opts).await {
        Ok(response) => response,
        Err(fallback_error) => {
            tracing::warn!(
                target: "jfc::stream::fallback",
                error = %error,
                fallback_error = %fallback_error,
                "non-streaming completion fallback failed"
            );
            return false;
        }
    };
    if !response.content.is_empty() {
        let _ = tx
            .send(EngineEvent::Stream(StreamEvent::Chunk {
                text: Some(response.content),
                reasoning: None,
            }))
            .await;
    }
    let _ = tx
        .send(EngineEvent::Stream(StreamEvent::Usage {
            input_tokens: response.usage.input_tokens as u32,
            output_tokens: response.usage.output_tokens as u32,
            cache_read_tokens: response.usage.cache_read_tokens as u32,
            cache_write_tokens: response.usage.cache_creation_tokens as u32,
        }))
        .await;
    crate::hooks::fire(
        crate::hooks::HookPoint::AfterStream,
        &crate::hooks::HookContext::for_session("stream")
            .with_extra("stop_reason", "nonstreaming_fallback".to_owned()),
    );
    let _ = tx
        .send(EngineEvent::Stream(StreamEvent::Done(StopReason::EndTurn)))
        .await;
    true
}

#[cfg(test)]
mod fallback_tests {
    use super::*;

    // Normal: committed output always classifies as partial_yield regardless of
    // the error shape — we must never re-stream over text the user already saw.
    #[test]
    fn classify_fallback_cause_partial_yield_when_committed_normal() {
        assert_eq!(
            classify_fallback_cause("connection reset", true),
            "partial_yield"
        );
        assert_eq!(
            classify_fallback_cause("idle timeout", true),
            "partial_yield"
        );
    }

    // Normal: the pre-first-event causes map to their upstream-equivalent labels.
    #[test]
    fn classify_fallback_cause_maps_pre_first_event_shapes_normal() {
        assert_eq!(
            classify_fallback_cause("Stream ended before receiving any events", false),
            "stream_no_events"
        );
        assert_eq!(
            classify_fallback_cause("connection closed before first event", false),
            "stream_no_events"
        );
        assert_eq!(
            classify_fallback_cause("connection reset by peer", false),
            "stale_connection"
        );
        assert_eq!(
            classify_fallback_cause("stream timed out (watchdog)", false),
            "watchdog"
        );
        assert_eq!(classify_fallback_cause("some 500 error", false), "other");
    }

    // Robust: a pre-first-event retry only fires for stale/idle errors with no
    // committed output and within the attempt budget.
    #[test]
    fn should_retry_pre_first_event_respects_gates_robust() {
        // Eligible: stale error, nothing committed, under budget.
        assert!(should_retry_pre_first_event("connection closed", false, 0));
        assert!(should_retry_pre_first_event(
            "stream timed out before first event",
            false,
            MAX_PRE_FIRST_EVENT_STREAM_RETRIES - 1
        ));
        // Ineligible: committed output (would duplicate transcript).
        assert!(!should_retry_pre_first_event("connection closed", true, 0));
        // Ineligible: attempts exhausted.
        assert!(!should_retry_pre_first_event(
            "connection closed",
            false,
            MAX_PRE_FIRST_EVENT_STREAM_RETRIES
        ));
        // Ineligible: not a stale/idle shape (e.g. a hard 400).
        assert!(!should_retry_pre_first_event(
            "invalid_request_error",
            false,
            0
        ));
    }
}
