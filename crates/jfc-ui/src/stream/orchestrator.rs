use std::{sync::Arc, time::Duration};

use tokio::sync::mpsc;

use crate::runtime::{
    AppEvent, StreamEvent, StreamLifecyclePhase, StreamLifecycleStatus, StreamRequestOverrides,
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
    tx: mpsc::Sender<AppEvent>,
    interrupt: std::sync::Arc<std::sync::atomic::AtomicBool>,
    // wg-async pattern: spawned tasks holding critical state need an
    // explicit cancellation handle, not just a polled flag. The token
    // races the SSE stream against `.cancelled()` so ESC×2 unwinds in
    // microseconds instead of waiting for the next STREAM_INTERRUPT_POLL.
    cancel: tokio_util::sync::CancellationToken,
    previous_message_id: Option<String>,
    overrides: StreamRequestOverrides,
) {
    let _ = tx.try_send(AppEvent::Stream(StreamEvent::Lifecycle(
        StreamLifecycleStatus::new(
            StreamLifecyclePhase::PreparingContext,
            Some(format!("{} messages", messages.len())),
        ),
    )));

    let prepared = prepare_stream_request(provider.clone(), &messages, &model, overrides).await;
    let mut opts = prepared.opts;
    if let Some(id) = previous_message_id {
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
    let _ = tx.try_send(AppEvent::Stream(StreamEvent::SystemPromptLen(
        prepared.system_prompt_tokens,
    )));
    // Tell the user when this turn pulled in recalled memory.
    if prepared.recalled_memory_chars > 0 {
        let _ = tx.try_send(AppEvent::Stream(StreamEvent::MemoryRecalled(
            prepared.recalled_memory_chars,
        )));
    }
    if tx
        .send(AppEvent::Stream(StreamEvent::RequestMetadata(
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
    let _ = tx.try_send(AppEvent::Stream(StreamEvent::Lifecycle(
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
            let _ = tx.try_send(AppEvent::Stream(StreamEvent::Lifecycle(
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
                let _ = tx.try_send(AppEvent::Stream(StreamEvent::Lifecycle(
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
                            .send(AppEvent::Stream(StreamEvent::Error(e2.to_string())))
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
                    .send(AppEvent::Stream(StreamEvent::Error(format!(
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
                    .send(AppEvent::Stream(StreamEvent::Error(e.to_string())))
                    .await;
                return;
            }
        }
    };

    let stop_reason = match live_events::drain_stream_events(stream, &tx, interrupt, cancel).await {
        live_events::DrainOutcome::Done(stop_reason) => stop_reason,
        live_events::DrainOutcome::Cancelled(message) => {
            let _ = tx.send(AppEvent::Stream(StreamEvent::Error(message))).await;
            return;
        }
        live_events::DrainOutcome::Error {
            message,
            committed_output,
        } => {
            if !committed_output {
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
                    error = %message,
                    "stream failed after output was committed; skipping non-streaming fallback to avoid duplicate transcript text"
                );
            }
            let _ = tx.send(AppEvent::Stream(StreamEvent::Error(message))).await;
            return;
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
    // but before the StreamDone AppEvent is sent. Handlers that want
    // to surface end-of-turn cost, run telemetry batching, or trigger
    // session auto-naming can land here.
    crate::hooks::fire(
        crate::hooks::HookPoint::AfterStream,
        &crate::hooks::HookContext::for_session("stream")
            .with_extra("stop_reason", format!("{stop_reason:?}")),
    );

    let _ = tx
        .send(AppEvent::Stream(StreamEvent::Done(stop_reason)))
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

async fn try_nonstreaming_fallback(
    provider: &dyn Provider,
    messages: Arc<Vec<ProviderMessage>>,
    opts: &StreamOptions,
    tx: &tokio::sync::mpsc::Sender<AppEvent>,
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
        .send(AppEvent::Stream(StreamEvent::Lifecycle(
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
            .send(AppEvent::Stream(StreamEvent::Chunk {
                text: Some(response.content),
                reasoning: None,
            }))
            .await;
    }
    let _ = tx
        .send(AppEvent::Stream(StreamEvent::Usage {
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
        .send(AppEvent::Stream(StreamEvent::Done(StopReason::EndTurn)))
        .await;
    true
}
