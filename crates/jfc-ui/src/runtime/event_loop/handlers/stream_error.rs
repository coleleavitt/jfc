//! `StreamEvent::Error(e)` handler — error handling, retries, network
//! recovery.

use jfc_provider::FallbackReason;

use crate::app::{App, NetworkRecoveryProvider};
use crate::runtime::{
    AppEvent, EventSender, UiEvent, drain_queued_prompts, record_network_recovery,
    restart_stream_in_place,
};
use crate::types::*;
use crate::{toast, types};

/// Handle `StreamEvent::Error(e)`.
pub(crate) async fn handle_stream_error(app: &mut App, tx: &EventSender, e: String) {
    app.record_stream_activity();
    tracing::error!(
        target: "jfc::stream",
        error = %e,
        is_streaming = app.is_streaming,
        cancelled = app.cancel_token.is_cancelled(),
        interrupt_flag = app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst),
        streaming_response_bytes = app.streaming_response_bytes,
        streaming_assistant_idx = ?app.streaming_assistant_idx,
        "StreamEvent::Error — resetting stream state"
    );
    if e == "Interrupted by user"
        && !app.cancel_token.is_cancelled()
        && !app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst)
    {
        tracing::info!(
            target: "jfc::stream",
            "dropping stale interrupt from superseded stream"
        );
        return;
    }
    // Interrupt-on-submit (key_dispatch) cancels the old stream's token,
    // then immediately mints a fresh token and clears the shared interrupt
    // flag *before* the cancelled task observes it. The old task therefore
    // reports a watchdog-style "Stream timed out" (cancel_reason reads the
    // now-false flag) rather than "Interrupted by user". If a fresh stream
    // is already live (is_streaming + uncancelled current token + clear
    // flag), this timeout belongs to the superseded stream and must be
    // dropped — otherwise it resets the brand-new turn. A *genuine* watchdog
    // timeout differs: check_stream_watchdog sets is_streaming=false before
    // its task's error lands here, so it falls through and surfaces normally.
    //
    // The same supersession can fire *before the connection even opens*:
    // when the interrupted stream was still inside
    // `open_stream_with_cancel_and_timeout`, cancelling its token makes that
    // select! bail with "Stream cancelled before connection opened" (or
    // "Stream open timed out…" if the open-timeout arm wins the race). Those
    // strings don't start with "Stream timed out", so without listing them
    // here the superseded pre-connection error lands as a hard `**Error:**`
    // on the brand-new turn (the reported "Stream cancelled before connection
    // opened" bug). Gate on the identical is_streaming && !cancelled &&
    // !interrupt condition so a *genuine* ESC (sets interrupt_flag) and a
    // *genuine* watchdog (clears is_streaming) both still surface.
    let is_superseded_stream_lifecycle_error = e.starts_with("Stream timed out")
        || e.starts_with("Stream cancelled before connection opened")
        || e.starts_with("Stream open timed out");
    if is_superseded_stream_lifecycle_error
        && app.is_streaming
        && !app.cancel_token.is_cancelled()
        && !app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst)
    {
        tracing::info!(
            target: "jfc::stream",
            error = %e,
            "dropping stale lifecycle error from superseded stream (a fresh turn is already streaming)"
        );
        return;
    }

    // ─── Synthetic tool_result injection on interrupt ────────
    // When a stream is interrupted with pending/running tool_use
    // entries in the conversation, inject a user-message with
    // tool_result is_error=true for each dangling tool_use.
    // Without this, the next API call fails because Anthropic's
    // API requires every tool_use to have a matching tool_result.
    // Mirrors claude-code 2.1.141's createSyntheticErrorMessage.
    if e.contains("Interrupted by user")
        && let Some(assistant_idx) = app.streaming_assistant_idx
        && let Some(msg) = app.messages.get(assistant_idx)
    {
        let dangling_tool_ids: Vec<crate::ids::ToolId> = msg
            .parts
            .iter()
            .filter_map(|p| {
                if let types::MessagePart::Tool(tc) = p
                    && matches!(
                        tc.status,
                        types::ToolStatus::Pending | types::ToolStatus::Running
                    )
                {
                    return Some(tc.id.clone());
                }
                None
            })
            .collect();
        if !dangling_tool_ids.is_empty() {
            tracing::info!(
                target: "jfc::stream",
                count = dangling_tool_ids.len(),
                "injecting synthetic tool_result for interrupted tool_use(s)"
            );
            // Mark each tool as Failed in the assistant message.
            if let Some(msg) = app.messages.get_mut(assistant_idx) {
                for part in &mut msg.parts {
                    if let types::MessagePart::Tool(tc) = part
                        && dangling_tool_ids.contains(&tc.id)
                    {
                        tc.status = types::ToolStatus::Failed;
                        tc.output =
                            types::ToolOutput::Text("[Request interrupted by user]".to_owned());
                    }
                }
            }
        }
    }
    // ─── End synthetic tool_result injection ─────────────────
    let auto_retry_openwebui_signal =
        e.starts_with(crate::providers::openwebui::AUTO_RETRY_SENTINEL);
    let auto_retry_anthropic_signal =
        e.starts_with(crate::providers::anthropic::AUTO_RETRY_SENTINEL);
    let auto_retry_anthropic_oauth_signal =
        e.starts_with(crate::providers::anthropic_oauth::AUTO_RETRY_SENTINEL);
    let auto_retry_signal = auto_retry_openwebui_signal
        || auto_retry_anthropic_signal
        || auto_retry_anthropic_oauth_signal;
    let visible_error = if auto_retry_openwebui_signal {
        e.trim_start_matches(crate::providers::openwebui::AUTO_RETRY_SENTINEL)
    } else if auto_retry_anthropic_signal {
        e.trim_start_matches(crate::providers::anthropic::AUTO_RETRY_SENTINEL)
    } else if auto_retry_anthropic_oauth_signal {
        e.trim_start_matches(crate::providers::anthropic_oauth::AUTO_RETRY_SENTINEL)
    } else {
        e.as_str()
    }
    .trim();
    if auto_retry_openwebui_signal {
        record_network_recovery(
            app,
            NetworkRecoveryProvider::OpenWebUI,
            e.trim_start_matches(crate::providers::openwebui::AUTO_RETRY_SENTINEL),
        );
    } else if auto_retry_anthropic_signal {
        record_network_recovery(
            app,
            NetworkRecoveryProvider::Anthropic,
            e.trim_start_matches(crate::providers::anthropic::AUTO_RETRY_SENTINEL),
        );
    } else if auto_retry_anthropic_oauth_signal {
        record_network_recovery(
            app,
            NetworkRecoveryProvider::AnthropicOAuth,
            e.trim_start_matches(crate::providers::anthropic_oauth::AUTO_RETRY_SENTINEL),
        );
    } else {
        app.network_recovery_status = None;
        app.network_recovery_attempts = 0;
    }
    // v132 mid-stream auto-compact: stream.rs prefixes
    // its `auto-compact:` sentinel when the API rejected
    // the prompt for size reasons. We force a compact
    // and re-queue the last user prompt instead of
    // surfacing the failure to the user — they shouldn't
    // have to manually trigger /compact + retype every
    // time the estimator drifts.
    let auto_compact_signal = e.starts_with("auto-compact:");
    if auto_compact_signal {
        app.force_compact_pending = true;
        toast::push_with_cap(
            &mut app.toasts,
            toast::Toast::new(
                toast::ToastKind::Warning,
                "Auto-compacting (prompt exceeded model window)…",
            ),
        );
        // Try to recover the last user prompt so we can
        // re-queue it after compaction.
        let last_user_text = app
            .messages
            .iter()
            .rfind(|m| matches!(m.role, types::Role::User))
            .and_then(|m| {
                m.parts.iter().find_map(|p| match p {
                    types::MessagePart::Text(t) if !t.trim().is_empty() => Some(t.clone()),
                    _ => None,
                })
            });
        if let Some(text) = last_user_text {
            let tx_compact = tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                let _ = tx_compact.send(AppEvent::Ui(UiEvent::Submit(text))).await;
            });
        }
    }
    let retry_assistant_idx = app.streaming_assistant_idx;
    let retry_turn_started_at = app.turn_started_at;
    app.is_streaming = false;
    app.last_stream_event_at = None;
    app.streaming_started_at = None;
    app.streaming_last_token_at = None;
    app.thinking_started_at = None;
    app.thinking_ended_at = None;
    app.streaming_text = String::new();
    app.streaming_reasoning = String::new();
    app.render_cache.borrow_mut().clear_streaming();
    app.streaming_response_bytes = 0;
    app.streaming_assistant_idx = None;
    app.current_stream_request = None;
    // Clear the turn clock and any pending tool calls so the
    // spinner row stops rendering. Without this, the
    // `show_spinner` condition stays true (it checks
    // `turn_started_at.is_some()` and `!pending_tool_calls.is_empty()`)
    // and the spinner/counter keeps animating after an
    // interrupt or network error.
    if !auto_retry_signal {
        app.turn_started_at = None;
    }
    app.pending_tool_calls.clear();
    app.pre_dispatched_tool_ids.clear();
    app.deferred_tool_uses.clear();
    app.in_progress_tool_use_ids.clear();
    app.in_flight_eager_dispatches = 0;
    app.in_flight_tool_batches = 0;
    // Reset the interrupt flag so background tasks or the
    // next auto-retry don't see a stale `true`. Also mint
    // a fresh cancel token — the previous one may already
    // be cancelled, and we don't want to poison the next
    // spawn.
    app.interrupt_flag
        .store(false, std::sync::atomic::Ordering::SeqCst);
    app.cancel_token = tokio_util::sync::CancellationToken::new();
    let mut auto_retry_restarted = false;
    if auto_retry_signal {
        if let Some(idx) = retry_assistant_idx {
            restart_stream_in_place(app, tx, idx, retry_turn_started_at);
            auto_retry_restarted = true;
        } else {
            tracing::warn!(
                target: "jfc::stream",
                error = %visible_error,
                "auto-retry stream error had no assistant slot; surfacing as hard error"
            );
            app.network_recovery_status = None;
            app.network_recovery_attempts = 0;
            app.turn_started_at = None;
            app.messages.push(ChatMessage::assistant(format!(
                "**Error:** {visible_error}\n\n_Press Ctrl+R to retry the last prompt._"
            )));
            let mut preview_cap = visible_error.len().min(120);
            while preview_cap > 0 && !visible_error.is_char_boundary(preview_cap) {
                preview_cap -= 1;
            }
            let preview = &visible_error[..preview_cap];
            toast::push_with_cap(
                &mut app.toasts,
                toast::Toast::new(toast::ToastKind::Error, format!("Stream error: {preview}")),
            );
        }
    } else if !auto_compact_signal {
        app.messages.push(ChatMessage::assistant(format!(
            "**Error:** {e}\n\n_Press Ctrl+R to retry the last prompt._"
        )));
        // Surface as a toast too so the user sees the failure
        // even if they aren't looking at the bottom of the
        // transcript when it lands. Cap to 120 chars so a
        // multi-paragraph error stays readable in the strip.
        let mut preview_cap = e.len().min(120);
        while preview_cap > 0 && !e.is_char_boundary(preview_cap) {
            preview_cap -= 1;
        }
        let preview = &e[..preview_cap];
        toast::push_with_cap(
            &mut app.toasts,
            toast::Toast::new(toast::ToastKind::Error, format!("Stream error: {preview}")),
        );
    }
    app.scroll_to_bottom();
    // v137 VC4 (cli.2.1.137.deob.js:580338) auto-fires queued
    // commands once the queryGuard goes idle. jfc had no
    // equivalent: after ESC×2 abort or a network error the
    // queue would sit visible-but-stranded until the user
    // submitted again. Drain here so queued prompts run on
    // the next opportunity. Skipped on auto-compact since
    // that path already re-queues the last user prompt.
    if !auto_compact_signal && !auto_retry_restarted && !app.queued_prompts.is_empty() {
        tracing::info!(
            target: "jfc::ui::queue",
            count = app.queued_prompts.len(),
            "draining queued prompts after StreamError"
        );
        drain_queued_prompts(app, tx).await;
    }
}

/// Handle `StreamEvent::FallbackTriggered` — the provider switched from the
/// requested model to a fallback (e.g. 529 overload triggered Opus→Sonnet).
/// Surfaces a toast so the user knows which model is actually responding.
pub(crate) fn handle_fallback_triggered(
    app: &mut App,
    original_model: &str,
    fallback_model: &str,
    reason: &FallbackReason,
) {
    tracing::info!(
        target: "jfc::stream",
        original_model,
        fallback_model,
        %reason,
        "model fallback triggered"
    );
    let message = match reason {
        FallbackReason::ModelRefusal => {
            format!("⚠ Model refused request, falling back to {fallback_model}")
        }
        _ => format!("Model fallback: using {fallback_model} (from {original_model})"),
    };
    toast::push_with_cap(
        &mut app.toasts,
        toast::Toast::new(toast::ToastKind::Warning, message),
    );
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use tokio::sync::mpsc;

    use super::*;

    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            #[allow(dead_code)] _messages: Vec<ProviderMessage>,
            #[allow(dead_code)] _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for TestProvider {}

    fn test_app() -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = jfc_session::TaskStore::in_memory();
        app
    }

    #[tokio::test]
    async fn stale_interrupt_from_superseded_stream_is_ignored() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("new prompt".into()));
        app.messages.push(ChatMessage::assistant(String::new()));
        app.is_streaming = true;
        app.streaming_assistant_idx = Some(1);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(&mut app, &tx, "Interrupted by user".to_owned()).await;

        assert!(app.is_streaming);
        assert_eq!(app.streaming_assistant_idx, Some(1));
        assert_eq!(app.messages.len(), 2);
    }

    // Normal — REGRESSION (the "Stream cancelled before connection opened"
    // bug): interrupt-on-submit cancels an in-flight stream that was still
    // opening its connection, then spawns a fresh turn. The superseded
    // stream's pre-open bail must be dropped, NOT pushed as a hard error on
    // the new turn. Distinguishing state: a fresh turn is live
    // (is_streaming = true), the *current* cancel token is healthy
    // (uncancelled), and interrupt_flag is clear (the user submitted, didn't
    // ESC). This mirrors the live conditions handle_submit leaves behind.
    #[tokio::test]
    async fn superseded_pre_open_cancel_is_dropped_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("new prompt".into()));
        app.messages.push(ChatMessage::assistant(String::new()));
        app.is_streaming = true;
        app.streaming_assistant_idx = Some(1);
        // Fresh turn's token is healthy; interrupt flag clear (submit, not ESC).
        app.cancel_token = tokio_util::sync::CancellationToken::new();
        app.interrupt_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut app,
            &tx,
            "Stream cancelled before connection opened".to_owned(),
        )
        .await;

        // The new turn must be untouched: still streaming, same slot, no
        // **Error:** message appended.
        assert!(app.is_streaming, "fresh turn must keep streaming");
        assert_eq!(app.streaming_assistant_idx, Some(1));
        assert_eq!(app.messages.len(), 2, "no error message appended");
    }

    // Robust: the open-timeout sibling string ("Stream open timed out…")
    // takes the same superseded path — it also doesn't start with "Stream
    // timed out", so it would otherwise slip through to the hard-error push.
    #[tokio::test]
    async fn superseded_open_timeout_is_dropped_robust() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("new prompt".into()));
        app.messages.push(ChatMessage::assistant(String::new()));
        app.is_streaming = true;
        app.streaming_assistant_idx = Some(1);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut app,
            &tx,
            "Stream open timed out after 45s before first provider response".to_owned(),
        )
        .await;

        assert!(app.is_streaming);
        assert_eq!(app.messages.len(), 2);
    }

    // Robust: a GENUINE pre-open cancel (no fresh turn took over —
    // is_streaming already false because the lifecycle path reset it) must
    // still surface as a hard error so the user can Ctrl+R. The supersession
    // guard must not swallow it.
    #[tokio::test]
    async fn genuine_pre_open_cancel_surfaces_robust() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("only prompt".into()));
        app.messages.push(ChatMessage::assistant(String::new()));
        // No fresh turn: is_streaming was already cleared by the lifecycle.
        app.is_streaming = false;
        app.streaming_assistant_idx = Some(1);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut app,
            &tx,
            "Stream cancelled before connection opened".to_owned(),
        )
        .await;

        // Hard-error path ran: an **Error:** assistant message was appended.
        assert_eq!(app.messages.len(), 3, "error message must be appended");
        let last = app.messages.last().expect("appended message");
        let text: String = last
            .parts
            .iter()
            .map(|p| match p {
                MessagePart::Text(t) => t.as_str(),
                _ => "",
            })
            .collect();
        assert!(
            text.contains("**Error:**"),
            "genuine pre-open cancel must surface a hard error, got: {text}"
        );
    }
}
