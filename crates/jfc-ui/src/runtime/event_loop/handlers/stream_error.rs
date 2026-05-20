//! `StreamEvent::Error(e)` handler — error handling, retries, network
//! recovery.

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

    // ─── Synthetic tool_result injection on interrupt ────────
    // When a stream is interrupted with pending/running tool_use
    // entries in the conversation, inject a user-message with
    // tool_result is_error=true for each dangling tool_use.
    // Without this, the next API call fails because Anthropic's
    // API requires every tool_use to have a matching tool_result.
    // Mirrors claude-code 2.1.141's createSyntheticErrorMessage.
    if e.contains("Interrupted by user") {
        if let Some(assistant_idx) = app.streaming_assistant_idx {
            if let Some(msg) = app.messages.get(assistant_idx) {
                let dangling_tool_ids: Vec<crate::ids::ToolId> = msg
                    .parts
                    .iter()
                    .filter_map(|p| {
                        if let types::MessagePart::Tool(tc) = p {
                            if matches!(
                                tc.status,
                                types::ToolStatus::Pending | types::ToolStatus::Running
                            ) {
                                return Some(tc.id.clone());
                            }
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
                            if let types::MessagePart::Tool(tc) = part {
                                if dangling_tool_ids.contains(&tc.id) {
                                    tc.status = types::ToolStatus::Failed;
                                    tc.output = types::ToolOutput::Text(
                                        "[Request interrupted by user]".to_owned(),
                                    );
                                }
                            }
                        }
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
    if !auto_retry_openwebui_signal
        && !auto_retry_anthropic_signal
        && !auto_retry_anthropic_oauth_signal
    {
        app.turn_started_at = None;
    }
    app.pending_tool_calls.clear();
    // Reset the interrupt flag so background tasks or the
    // next auto-retry don't see a stale `true`. Also mint
    // a fresh cancel token — the previous one may already
    // be cancelled, and we don't want to poison the next
    // spawn.
    app.interrupt_flag
        .store(false, std::sync::atomic::Ordering::SeqCst);
    app.cancel_token = tokio_util::sync::CancellationToken::new();
    if auto_retry_openwebui_signal
        || auto_retry_anthropic_signal
        || auto_retry_anthropic_oauth_signal
    {
        if let Some(idx) = retry_assistant_idx {
            restart_stream_in_place(app, tx, idx, retry_turn_started_at);
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
    if !auto_compact_signal
        && !auto_retry_openwebui_signal
        && !auto_retry_anthropic_signal
        && !auto_retry_anthropic_oauth_signal
        && !app.queued_prompts.is_empty()
    {
        tracing::info!(
            target: "jfc::ui::queue",
            count = app.queued_prompts.len(),
            "draining queued prompts after StreamError"
        );
        drain_queued_prompts(app, tx).await;
    }
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
            #[allow(dead_code)] messages: Vec<ProviderMessage>,
            #[allow(dead_code)] options: &StreamOptions,
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
}
