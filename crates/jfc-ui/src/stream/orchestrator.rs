use std::sync::Arc;

use tokio::sync::mpsc;

use crate::runtime::{AppEvent, StreamEvent};
use jfc_provider::{ModelId, Provider, ProviderMessage, StreamOptions};

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
) {
    let prepared = prepare_stream_request(provider.clone(), &messages, &model).await;
    let mut opts = prepared.opts;
    if let Some(id) = previous_message_id {
        opts.previous_message_id = Some(id);
    }

    // Report system prompt size back to App for post-compaction overhead estimate.
    let _ = tx.try_send(AppEvent::Stream(StreamEvent::SystemPromptLen(
        prepared.system_prompt_tokens,
    )));
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
    let stream = match open_stream_with_bedrock_retries(
        provider.as_ref(),
        Arc::clone(&messages),
        &opts,
    )
    .await
    {
        Ok(s) => {
            tracing::debug!(target: "jfc::stream", "stream opened successfully");
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
                // Reuse opts fields by ref — avoid cloning system prompt + tool defs.
                let fallback_opts = StreamOptions::new(opts.model.clone())
                    .system(opts.system.as_deref().unwrap_or_default().to_owned())
                    .tools(opts.tools.clone())
                    .max_tokens(opts.max_tokens);
                match open_stream_with_bedrock_retries(
                    provider.as_ref(),
                    Arc::clone(&messages),
                    &fallback_opts,
                )
                .await
                {
                    Ok(s) => s,
                    Err(e2) => {
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
                tracing::error!(target: "jfc::stream", error = %e, "stream open failed");
                let _ = tx
                    .send(AppEvent::Stream(StreamEvent::Error(e.to_string())))
                    .await;
                return;
            }
        }
    };

    let Some(stop_reason) = live_events::drain_stream_events(stream, &tx, interrupt, cancel).await
    else {
        return;
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
