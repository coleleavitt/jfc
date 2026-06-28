//! `StreamEvent::Error(e)` handler — error handling, retries, network
//! recovery.

use jfc_provider::FallbackReason;

use crate::app::{EngineState, NetworkRecoveryProvider};
use crate::context_accounting::{
    parse_detected_context_limit, persist_session_detected_context_limit,
};
use crate::runtime::{
    ControlEvent, EngineEvent, EventSender, drain_queued_prompts, record_network_recovery,
    restart_stream_in_place,
};
use crate::types::*;
use crate::{toast, types};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StreamErrorLogDisposition {
    ExpectedLifecycle,
    Unexpected,
}

fn stream_interrupt_flag(state: &EngineState) -> bool {
    state
        .interrupt_flag
        .load(std::sync::atomic::Ordering::SeqCst)
}

fn is_stream_lifecycle_error(error: &str) -> bool {
    error.starts_with("Stream timed out")
        || error.starts_with("Stream cancelled before connection opened")
        || error.starts_with("Stream open timed out")
        || error.starts_with("stream task cancelled")
}

fn stream_error_interrupted_by_user(state: &EngineState, error: &str) -> bool {
    error.contains("Interrupted by user")
        || (error.starts_with("stream task cancelled")
            && (state.cancel_token.is_cancelled() || stream_interrupt_flag(state)))
}

fn stream_error_log_disposition(state: &EngineState, error: &str) -> StreamErrorLogDisposition {
    let stale_interrupt = error == "Interrupted by user"
        && !state.cancel_token.is_cancelled()
        && !stream_interrupt_flag(state);
    let superseded_lifecycle = is_stream_lifecycle_error(error)
        && state.is_streaming
        && !state.cancel_token.is_cancelled()
        && !stream_interrupt_flag(state);
    let late_cleaned_join = error.starts_with("stream task cancelled")
        && !state.is_streaming
        && state.streaming_assistant_idx.is_none()
        && state.active_stream_handle.is_none();
    if stale_interrupt
        || superseded_lifecycle
        || late_cleaned_join
        || stream_error_interrupted_by_user(state, error)
    {
        StreamErrorLogDisposition::ExpectedLifecycle
    } else {
        StreamErrorLogDisposition::Unexpected
    }
}

fn log_stream_error_received(state: &EngineState, error: &str) {
    match stream_error_log_disposition(state, error) {
        StreamErrorLogDisposition::ExpectedLifecycle => tracing::info!(
            target: "jfc::stream",
            error = %error,
            is_streaming = state.is_streaming,
            cancelled = state.cancel_token.is_cancelled(),
            interrupt_flag = stream_interrupt_flag(state),
            streaming_response_bytes = state.streaming_response_bytes,
            streaming_assistant_idx = ?state.streaming_assistant_idx,
            "StreamEvent::Error — expected stream lifecycle cancellation"
        ),
        StreamErrorLogDisposition::Unexpected => tracing::error!(
            target: "jfc::stream",
            error = %error,
            is_streaming = state.is_streaming,
            cancelled = state.cancel_token.is_cancelled(),
            interrupt_flag = stream_interrupt_flag(state),
            streaming_response_bytes = state.streaming_response_bytes,
            streaming_assistant_idx = ?state.streaming_assistant_idx,
            "StreamEvent::Error — resetting stream state"
        ),
    }
}

/// Handle `StreamEvent::Error(e)`.
pub async fn handle_stream_error(state: &mut EngineState, tx: &EventSender, e: String) {
    state.record_stream_activity();
    state.stream_lifecycle = None;
    log_stream_error_received(state, &e);
    if e == "Interrupted by user"
        && !state.cancel_token.is_cancelled()
        && !stream_interrupt_flag(state)
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
    let is_superseded_stream_lifecycle_error = is_stream_lifecycle_error(&e);
    if is_superseded_stream_lifecycle_error
        && state.is_streaming
        && !state.cancel_token.is_cancelled()
        && !stream_interrupt_flag(state)
    {
        tracing::info!(
            target: "jfc::stream",
            error = %e,
            "dropping stale lifecycle error from superseded stream (a fresh turn is already streaming)"
        );
        return;
    }

    // A "stream task cancelled" JoinError is the supervisor reporting that
    // the inner stream task was forcefully aborted. Every deliberate abort
    // path (interrupt(), the watchdog, the first error event's own cleanup)
    // resets stream state *before* this JoinError lands — fresh cancel
    // token, cleared interrupt flag, is_streaming=false — so neither the
    // supersession guard above (needs is_streaming=true) nor the
    // interrupted_by_user check below (needs a cancelled token / set flag)
    // recognizes it, and it used to surface as a hard
    // "Stream error: stream task cancelled: task N was cancelled" toast on
    // a turn that was already fully cleaned up. If no stream is live
    // anymore (no streaming slot, no active handle), there is nothing left
    // to report on: drop it.
    if e.starts_with("stream task cancelled")
        && !state.is_streaming
        && state.streaming_assistant_idx.is_none()
        && state.active_stream_handle.is_none()
    {
        tracing::info!(
            target: "jfc::stream",
            error = %e,
            "dropping late join error from an already-cleaned-up aborted stream"
        );
        return;
    }

    let interrupted_by_user = stream_error_interrupted_by_user(state, &e);

    // ─── Synthetic tool_result injection on interrupt ────────
    // When a stream is interrupted with pending/running tool_use
    // entries in the conversation, inject a user-message with
    // tool_result is_error=true for each dangling tool_use.
    // Without this, the next API call fails because Anthropic's
    // API requires every tool_use to have a matching tool_result.
    // Mirrors claude-code 2.1.141's createSyntheticErrorMessage.
    if interrupted_by_user
        && let Some(assistant_idx) = state.streaming_assistant_idx
        && let Some(msg) = state.messages.get(assistant_idx)
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
            if let Some(msg) = state.messages.get_mut(assistant_idx) {
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
    let sentinel_signal = auto_retry_openwebui_signal
        || auto_retry_anthropic_signal
        || auto_retry_anthropic_oauth_signal;

    // Uniform rate-limit / overload handling. A retryable error (429, 529,
    // overloaded, 5xx, "too many requests", …) should auto-retry with backoff
    // regardless of whether the provider tagged it with an `auto-retry-*`
    // sentinel. The retry cap is shared across sentinel-tagged and bare
    // transients; otherwise a persistent provider-side rate limit can restart
    // forever and never surface a hard error.
    let retryable_stream_error = jfc_provider::retry::retryable_stream_error(&e).is_some();
    let auto_retry_signal = retryable_stream_error
        && state.network_recovery_attempts < crate::app::MAX_NETWORK_RECOVERY_ATTEMPTS
        && state.streaming_assistant_idx.is_some();

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
    if auto_retry_signal && auto_retry_openwebui_signal {
        record_network_recovery(
            state,
            NetworkRecoveryProvider::OpenWebUI,
            e.trim_start_matches(crate::providers::openwebui::AUTO_RETRY_SENTINEL),
        );
    } else if auto_retry_signal && auto_retry_anthropic_signal {
        record_network_recovery(
            state,
            NetworkRecoveryProvider::Anthropic,
            e.trim_start_matches(crate::providers::anthropic::AUTO_RETRY_SENTINEL),
        );
    } else if auto_retry_signal && auto_retry_anthropic_oauth_signal {
        record_network_recovery(
            state,
            NetworkRecoveryProvider::AnthropicOAuth,
            e.trim_start_matches(crate::providers::anthropic_oauth::AUTO_RETRY_SENTINEL),
        );
    } else if auto_retry_signal && !sentinel_signal {
        // No provider sentinel, but the bare text is a recognized transient
        // (429/529/overloaded/5xx). Record it under the generic provider so the
        // recovery banner + backoff behave identically to the sentinel path.
        record_network_recovery(state, NetworkRecoveryProvider::Provider, &e);
    } else {
        state.network_recovery_status = None;
        state.network_recovery_attempts = 0;
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
        if let Some(detected) = parse_detected_context_limit(visible_error) {
            let changed = state.record_detected_context_limit(detected);
            if let Some(session_id) = state.current_session_id.clone()
                && let Err(error) = persist_session_detected_context_limit(
                    session_id.as_str(),
                    state.model.as_str(),
                    detected,
                )
                .await
            {
                tracing::warn!(
                    target: "jfc::stream::budget",
                    session_id = %session_id,
                    error = %error,
                    "failed to persist detected context limit"
                );
            }
            tracing::warn!(
                target: "jfc::stream::budget",
                actual_tokens = ?detected.actual_tokens,
                limit_tokens = detected.limit_tokens,
                old_context_window = changed.map(|(old, _)| old),
                new_context_window = state.max_context_tokens,
                model = %state.model,
                "provider overflow reported context limit"
            );
            if let Some((old, new)) = changed {
                toast::push_with_cap(
                    &mut state.toasts,
                    toast::Toast::new(
                        toast::ToastKind::Warning,
                        format!("Detected provider context limit: {new} tokens (was {old})"),
                    ),
                );
            }
        }
        state.force_compact_pending = true;
        toast::push_with_cap(
            &mut state.toasts,
            toast::Toast::new(
                toast::ToastKind::Warning,
                "Auto-compacting (prompt exceeded model window)…",
            ),
        );
        // Try to recover the last *genuine* user prompt so we can
        // re-queue it after compaction.
        //
        // Skip compact-boundary messages: `ChatMessage::compact_boundary`
        // is `Role::User` (it has to be, so the summary lands as user
        // context), so a naive `rfind(Role::User)` on a transcript that
        // already ends on a boundary would grab the summary's "This session
        // is being continued…" prose and replay *that* as the user's prompt.
        // Join every non-empty text part rather than just the first, so a
        // structured multi-text user message isn't silently truncated to its
        // opening block. Binary attachments can't ride along
        // `ControlEvent::SubmitPrompt(String)`, but they are not lost from context: the
        // original user message (with its attachments) stays in `state.messages`
        // and survives into the preserved tail of the compacted transcript —
        // the re-queue only re-drives the turn, it does not re-upload.
        let last_user = state
            .messages
            .iter()
            .rfind(|m| matches!(m.role, types::Role::User) && !m.is_compact_boundary());
        if let Some(att_count) = last_user.map(|m| m.attachments.len()).filter(|n| *n > 0) {
            tracing::debug!(
                target: "jfc::stream",
                attachments = att_count,
                "auto-compact re-queue: attachments remain in the preserved transcript; \
                 the re-driven turn re-sends text only"
            );
        }
        let last_user_text = last_user.and_then(recoverable_requeue_text);
        if let Some(text) = last_user_text {
            let tx_compact = tx.clone();
            tokio::spawn(async move {
                tokio::time::sleep(std::time::Duration::from_millis(150)).await;
                let _ = tx_compact
                    .send(EngineEvent::Control(ControlEvent::SubmitPrompt(text)))
                    .await;
            });
        }
    }
    let retry_assistant_idx = state.streaming_assistant_idx;
    let retry_turn_started_at = state.turn_started_at;

    state.is_streaming = false;
    state.last_stream_event_at = None;
    state.streaming_started_at = None;
    state.streaming_last_token_at = None;
    state.thinking_started_at = None;
    state.thinking_ended_at = None;
    state.streaming_text = String::new();
    state.streaming_reasoning = String::new();
    state.push_effect(crate::app::EngineEffect::StreamingFinalized);
    state.streaming_response_bytes = 0;
    state.streaming_response_baseline = 0;
    state.streaming_thinking_tokens = 0;
    state.token_rate_samples.clear();
    state.token_rate_sample_thinking = None;
    state.streaming_assistant_idx = None;
    state.active_stream_handle = None;
    state.clear_active_stream_scope();
    state.current_stream_request = None;
    state.stream_lifecycle = None;
    // Clear the turn clock and any pending tool calls so the
    // spinner row stops rendering. Without this, the
    // `show_spinner` condition stays true (it checks
    // `turn_started_at.is_some()` and `!pending_tool_calls.is_empty()`)
    // and the spinner/counter keeps animating after an
    // interrupt or network error.
    if !auto_retry_signal {
        state.turn_started_at = None;
    }
    state.pending_tool_calls.clear();
    // A question modal only exists as a turn's terminal act; if the turn really
    // died (error/cancel) the answer can no longer feed anywhere, so close the
    // modal rather than leave it capturing all key input. For recoverable
    // auto-retry signals, keep it visible so a transient stream restart does
    // not make an AskUserQuestion prompt disappear under the user.
    if !auto_retry_signal {
        state.pending_question = None;
    }
    state.pre_dispatched_tool_ids.clear();
    state.deferred_tool_uses.clear();
    state.in_progress_tool_use_ids.clear();
    state.active_tool_calls.clear();
    state.in_flight_eager_dispatches = 0;
    state.in_flight_tool_batches = 0;
    // Reset the interrupt flag so background tasks or the
    // next auto-retry don't see a stale `true`. Also mint
    // a fresh cancel token — the previous one may already
    // be cancelled, and we don't want to poison the next
    // spawn.
    state
        .interrupt_flag
        .store(false, std::sync::atomic::Ordering::SeqCst);
    state.cancel_token = tokio_util::sync::CancellationToken::new();
    let mut auto_retry_restarted = false;
    if auto_retry_signal {
        state
            .exploration_state
            .bump_for_signal(crate::exploration::ExplorationSignal::StreamRetry);
        if let Some(idx) = retry_assistant_idx {
            restart_stream_in_place(state, tx, idx, retry_turn_started_at);
            auto_retry_restarted = true;
        } else {
            tracing::warn!(
                target: "jfc::stream",
                error = %visible_error,
                "auto-retry stream error had no assistant slot; surfacing as hard error"
            );
            state.network_recovery_status = None;
            state.network_recovery_attempts = 0;
            state.turn_started_at = None;
            push_hard_stream_error(state, visible_error);
            let mut preview_cap = visible_error.len().min(120);
            while preview_cap > 0 && !visible_error.is_char_boundary(preview_cap) {
                preview_cap -= 1;
            }
            let preview = &visible_error[..preview_cap];
            toast::push_with_cap(
                &mut state.toasts,
                toast::Toast::new(toast::ToastKind::Error, format!("Stream error: {preview}")),
            );
        }
    } else if !auto_compact_signal && !interrupted_by_user {
        let hard_error = if sentinel_signal {
            visible_error
        } else {
            e.as_str()
        };
        push_hard_stream_error(state, hard_error);
        // Surface as a toast too so the user sees the failure
        // even if they aren't looking at the bottom of the
        // transcript when it lands. Cap to 120 chars so a
        // multi-paragraph error stays readable in the strip.
        let mut preview_cap = hard_error.len().min(120);
        while preview_cap > 0 && !hard_error.is_char_boundary(preview_cap) {
            preview_cap -= 1;
        }
        let preview = &hard_error[..preview_cap];
        toast::push_with_cap(
            &mut state.toasts,
            toast::Toast::new(toast::ToastKind::Error, format!("Stream error: {preview}")),
        );
    }
    state.push_effect(crate::app::EngineEffect::ScrollToBottom);
    // v137 VC4 (cli.2.1.137.deob.js:580338) auto-fires queued
    // commands once the queryGuard goes idle. jfc had no
    // equivalent: after ESC×2 abort or a network error the
    // queue would sit visible-but-stranded until the user
    // submitted again. Drain here so queued prompts run on
    // the next opportunity. Skipped on auto-compact since
    // that path already re-queues the last user prompt.
    if !auto_compact_signal && !auto_retry_restarted && !state.queued_prompts.is_empty() {
        tracing::info!(
            target: "jfc::ui::queue",
            count = state.queued_prompts.len(),
            "draining queued prompts after StreamError"
        );
        drain_queued_prompts(state, tx).await;
    }
}

/// Handle `StreamEvent::FallbackTriggered` — the provider switched from the
/// requested model to a fallback (e.g. 529 overload triggered Opus→Sonnet).
/// Surfaces a toast so the user knows which model is actually responding.
pub fn handle_fallback_triggered(
    state: &mut EngineState,
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
        FallbackReason::ModelNotFound => {
            format!("{original_model} unavailable (not found) — using {fallback_model}")
        }
        FallbackReason::PermissionDenied => {
            format!("{original_model} access denied — using {fallback_model}")
        }
        FallbackReason::Overloaded => {
            format!("{original_model} overloaded — using {fallback_model}")
        }
        FallbackReason::ServerError => {
            format!("{original_model} server error — last-resort fallback to {fallback_model}")
        }
    };
    toast::push_with_cap(
        &mut state.toasts,
        toast::Toast::new(toast::ToastKind::Warning, message),
    );
}

/// Join every non-empty text part of a user message into the single string the
/// auto-compact re-queue replays via `ControlEvent::SubmitPrompt`. Returns `None` when the
/// message carries no usable prompt text (e.g. an attachment-only turn), so the
/// caller skips the re-queue rather than submitting an empty prompt.
///
/// Joining *all* text parts (not just the first) keeps a structured multi-text
/// user message from being silently truncated to its opening block. Caller is
/// responsible for excluding compact-boundary messages before calling this —
/// see the `rfind` filter at the call site.
fn recoverable_requeue_text(m: &ChatMessage) -> Option<String> {
    let joined = m
        .parts
        .iter()
        .filter_map(|p| match p {
            types::MessagePart::Text(t) if !t.trim().is_empty() => Some(t.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n");
    (!joined.trim().is_empty()).then_some(joined)
}

fn hard_stream_error_text(error: &str) -> String {
    format!("**Error:** {error}\n\n_Press Ctrl+R to retry the last prompt._")
}

fn push_hard_stream_error(state: &mut EngineState, error: &str) -> bool {
    let body = hard_stream_error_text(error);
    let duplicate_last = state.messages.last().is_some_and(|msg| {
        msg.role == Role::Assistant
            && msg
                .parts
                .iter()
                .filter_map(|part| match part {
                    types::MessagePart::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>()
                == body
    });
    if duplicate_last {
        tracing::debug!(
            target: "jfc::stream",
            error = %error,
            "deduping repeated hard stream error"
        );
        false
    } else {
        state.messages.push(ChatMessage::assistant(body));
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::app::EngineState;
    use std::ffi::OsString;
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
            vec![
                ModelInfo::new("claude-opus-4-8", "Claude Opus", "test")
                    .with_context_window_tokens(1_000_000usize),
            ]
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for TestProvider {}

    struct KnowledgeDbEnvGuard {
        previous: Option<OsString>,
        _dir: tempfile::TempDir,
    }

    impl KnowledgeDbEnvGuard {
        fn new() -> Self {
            let dir = tempfile::tempdir().expect("temp knowledge db dir");
            let path = dir.path().join("knowledge.db");
            let previous = std::env::var_os("JFC_KNOWLEDGE_DB");
            unsafe { std::env::set_var("JFC_KNOWLEDGE_DB", &path) };
            Self {
                previous,
                _dir: dir,
            }
        }
    }

    impl Drop for KnowledgeDbEnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.previous.take() {
                    Some(previous) => std::env::set_var("JFC_KNOWLEDGE_DB", previous),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
            }
        }
    }

    fn test_app() -> EngineState {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state
    }

    fn test_app_with_model(model: &str) -> EngineState {
        let mut state = EngineState::new(Arc::new(TestProvider), model);
        state.task_store = jfc_session::TaskStore::in_memory();
        state
    }

    #[test]
    fn stream_cancelled_by_user_is_expected_lifecycle_log_regression() {
        // Given: a user interrupt cancelled an active stream task.
        let mut state = test_app();
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        state.cancel_token.cancel();
        state
            .interrupt_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);

        // When: the stream supervisor reports the task JoinError.
        let disposition =
            stream_error_log_disposition(&state, "stream task cancelled: task 17 was cancelled");

        // Then: it is expected lifecycle noise, not an unexpected stream error.
        assert_eq!(disposition, StreamErrorLogDisposition::ExpectedLifecycle);
    }

    #[test]
    fn provider_error_is_unexpected_log_normal() {
        // Given: an active stream receives a provider-side failure.
        let mut state = test_app();
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);

        // When: the error is not a cancellation/supersession lifecycle event.
        let disposition = stream_error_log_disposition(&state, "provider 500 overloaded");

        // Then: it still logs as an unexpected stream error.
        assert_eq!(disposition, StreamErrorLogDisposition::Unexpected);
    }

    #[tokio::test]
    async fn stale_interrupt_from_superseded_stream_is_ignored() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("new prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(&mut state, &tx, "Interrupted by user".to_owned()).await;

        assert!(state.is_streaming);
        assert_eq!(state.streaming_assistant_idx, Some(1));
        assert_eq!(state.messages.len(), 2);
    }

    #[tokio::test]
    #[serial_test::serial]
    async fn auto_compact_records_provider_detected_context_limit_regression() {
        let _db = KnowledgeDbEnvGuard::new();
        let mut state = test_app_with_model("claude-opus-4-8");
        state.max_context_tokens = 1_000_000;
        state.messages.push(ChatMessage::user("huge prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            r#"auto-compact: Anthropic API error 413 Payload Too Large: raw: {"error":{"type":"request_too_large","message":"Request exceeds the maximum size"},"actualTokens":900000,"limitTokens":200000}"#.to_owned(),
        )
        .await;

        assert_eq!(state.max_context_tokens, 200_000);
        assert_eq!(state.detected_context_limit_tokens, Some(200_000));
        assert_eq!(
            state.detected_context_limit_model.as_deref(),
            Some("claude-opus-4-8")
        );
        assert!(state.force_compact_pending);
        assert!(!state.is_streaming);
        let session_id = state.current_session_id.as_ref().expect("session id");
        let persisted = crate::context_accounting::load_session_detected_context_limit(
            session_id.as_str(),
            state.model.as_str(),
        )
        .await;
        assert_eq!(
            persisted,
            Some(crate::context_accounting::DetectedContextLimit {
                actual_tokens: Some(900_000),
                limit_tokens: 200_000,
            })
        );
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
        let mut state = test_app();
        state.messages.push(ChatMessage::user("new prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        // Fresh turn's token is healthy; interrupt flag clear (submit, not ESC).
        state.cancel_token = tokio_util::sync::CancellationToken::new();
        state
            .interrupt_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            "Stream cancelled before connection opened".to_owned(),
        )
        .await;

        // The new turn must be untouched: still streaming, same slot, no
        // **Error:** message appended.
        assert!(state.is_streaming, "fresh turn must keep streaming");
        assert_eq!(state.streaming_assistant_idx, Some(1));
        assert_eq!(state.messages.len(), 2, "no error message appended");
    }

    // Robust: the open-timeout sibling string ("Stream open timed out…")
    // takes the same superseded path — it also doesn't start with "Stream
    // timed out", so it would otherwise slip through to the hard-error push.
    #[tokio::test]
    async fn superseded_open_timeout_is_dropped_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("new prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            "Stream open timed out after 45s before first provider response".to_owned(),
        )
        .await;

        assert!(state.is_streaming);
        assert_eq!(state.messages.len(), 2);
    }

    #[tokio::test]
    async fn superseded_aborted_stream_join_error_is_dropped_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("new prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        state.cancel_token = tokio_util::sync::CancellationToken::new();
        state
            .interrupt_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            "stream task cancelled: task 17 was cancelled".to_owned(),
        )
        .await;

        assert!(state.is_streaming);
        assert_eq!(state.streaming_assistant_idx, Some(1));
        assert_eq!(state.messages.len(), 2, "no stale join error appended");
    }

    #[tokio::test]
    async fn aborted_stream_join_error_after_user_interrupt_is_clean_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("only prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        state.cancel_token.cancel();
        state
            .interrupt_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            "stream task cancelled: task 17 was cancelled".to_owned(),
        )
        .await;

        assert!(!state.is_streaming);
        assert_eq!(
            state.messages.len(),
            2,
            "user interrupt should not add hard error"
        );
        assert!(
            !state.cancel_token.is_cancelled(),
            "next turn gets a fresh token"
        );
        assert!(
            !state
                .interrupt_flag
                .load(std::sync::atomic::Ordering::SeqCst),
            "interrupt flag should be cleared after cleanup"
        );
    }

    #[tokio::test]
    async fn interrupted_partial_assistant_keeps_transcript_clean_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("only prompt".into()));
        state
            .messages
            .push(ChatMessage::assistant("partial answer".into()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        state.cancel_token.cancel();
        state
            .interrupt_flag
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(&mut state, &tx, "Interrupted by user".to_owned()).await;

        assert_eq!(state.messages.len(), 2, "interrupt should not append error");
        let text: String = state.messages[1]
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "partial answer");
        assert!(
            !text.contains("Response truncated"),
            "interrupt must not persist a truncation banner"
        );
    }

    // Robust: a late JoinError from an aborted stream task landing AFTER the
    // turn was already fully cleaned up (watchdog or interrupt path reset
    // everything, minted a fresh token, cleared the flag) must be dropped —
    // not surfaced as a hard "Stream error: stream task cancelled" toast.
    #[tokio::test]
    async fn late_join_error_after_cleanup_is_dropped_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("prompt".into()));
        // Already cleaned up: nothing streaming, no slot, no handle.
        state.is_streaming = false;
        state.streaming_assistant_idx = None;
        state.active_stream_handle = None;
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            "stream task cancelled: task 919 was cancelled".to_owned(),
        )
        .await;

        assert_eq!(state.messages.len(), 1, "no hard error appended");
        assert!(state.toasts.is_empty(), "no error toast pushed");
    }

    #[tokio::test]
    async fn stale_scoped_provider_error_is_dropped_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("new prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        let old_stream_id = state.begin_stream_scope();
        let current_stream_id = state.begin_stream_scope();
        assert_ne!(old_stream_id, current_stream_id);
        let (tx, _rx) = mpsc::channel(8);

        crate::runtime::handle_engine_event(
            &mut state,
            &tx,
            crate::runtime::EngineEvent::ScopedStream {
                stream_id: old_stream_id,
                event: crate::runtime::StreamEvent::Error("Rate limited".to_owned()),
            },
        )
        .await
        .unwrap();

        assert!(state.is_streaming, "current stream must keep running");
        assert_eq!(state.streaming_assistant_idx, Some(1));
        assert_eq!(state.messages.len(), 2, "stale error must not append");
        assert_eq!(state.active_stream_id, Some(current_stream_id));
    }

    #[tokio::test]
    async fn duplicate_hard_stream_error_is_deduped_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        state.network_recovery_attempts = crate::app::MAX_NETWORK_RECOVERY_ATTEMPTS;
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(&mut state, &tx, "Rate limited".to_owned()).await;
        handle_stream_error(&mut state, &tx, "Rate limited".to_owned()).await;

        let hard_errors = state
            .messages
            .iter()
            .filter(|msg| {
                msg.role == Role::Assistant
                    && msg.parts.iter().any(|part| {
                        matches!(part, MessagePart::Text(text) if text.contains("**Error:** Rate limited"))
                    })
            })
            .count();
        assert_eq!(hard_errors, 1, "same hard error should render once");
    }

    #[tokio::test]
    async fn stream_error_clears_active_stream_handle_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("only prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        let handle = tokio::spawn(async {
            std::future::pending::<()>().await;
        });
        state.active_stream_handle = Some(handle.abort_handle());
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(&mut state, &tx, "provider failed".to_owned()).await;

        assert!(state.active_stream_handle.is_none());
        assert!(!state.has_interruptible_work());
        handle.abort();
    }

    #[tokio::test]
    async fn sentinel_retry_under_cap_restarts_without_hard_error_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("only prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            format!(
                "{}Rate limited",
                crate::providers::anthropic::AUTO_RETRY_SENTINEL
            ),
        )
        .await;

        assert!(state.is_streaming, "retry under cap should restart stream");
        assert_eq!(state.streaming_assistant_idx, Some(1));
        assert_eq!(state.network_recovery_attempts, 1);
        assert!(
            state.network_recovery_status.is_some(),
            "retry banner should stay armed"
        );
        assert_eq!(state.messages.len(), 2, "no hard error should append");
        assert!(
            state.active_stream_id.is_some(),
            "new stream should be scoped"
        );
    }

    #[tokio::test]
    async fn sentinel_retry_respects_max_attempts_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("only prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        state.is_streaming = true;
        state.streaming_assistant_idx = Some(1);
        state.network_recovery_attempts = crate::app::MAX_NETWORK_RECOVERY_ATTEMPTS;
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            format!(
                "{}Rate limited",
                crate::providers::anthropic::AUTO_RETRY_SENTINEL
            ),
        )
        .await;

        assert!(!state.is_streaming, "cap exhaustion should stop streaming");
        assert_eq!(
            state.messages.len(),
            3,
            "cap exhaustion should surface a hard error"
        );
        let text: String = state
            .messages
            .last()
            .expect("hard error appended")
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(text.contains("**Error:** Rate limited"));
        assert!(
            !text.contains(crate::providers::anthropic::AUTO_RETRY_SENTINEL),
            "provider retry sentinel must not leak into the user-visible error"
        );
    }

    // Robust: a GENUINE pre-open cancel (no fresh turn took over —
    // is_streaming already false because the lifecycle path reset it) must
    // still surface as a hard error so the user can Ctrl+R. The supersession
    // guard must not swallow it.
    #[tokio::test]
    async fn genuine_pre_open_cancel_surfaces_robust() {
        let mut state = test_app();
        state.messages.push(ChatMessage::user("only prompt".into()));
        state.messages.push(ChatMessage::assistant(String::new()));
        // No fresh turn: is_streaming was already cleared by the lifecycle.
        state.is_streaming = false;
        state.streaming_assistant_idx = Some(1);
        let (tx, _rx) = mpsc::channel(8);

        handle_stream_error(
            &mut state,
            &tx,
            "Stream cancelled before connection opened".to_owned(),
        )
        .await;

        // Hard-error path ran: an **Error:** assistant message was appended.
        assert_eq!(state.messages.len(), 3, "error message must be appended");
        let last = state.messages.last().expect("appended message");
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

    // ─── auto-compact re-queue selection (regression: fix #6) ───────────

    /// Mirror the call-site selector: most-recent genuine user message that is
    /// NOT a compact boundary, joined into its replayable prompt text.
    fn select_requeue_text(messages: &[ChatMessage]) -> Option<String> {
        messages
            .iter()
            .rfind(|m| matches!(m.role, types::Role::User) && !m.is_compact_boundary())
            .and_then(recoverable_requeue_text)
    }

    // Normal: a plain user prompt is recovered verbatim for re-queue.
    #[test]
    fn requeue_recovers_plain_user_prompt_normal() {
        let messages = vec![
            ChatMessage::user("first".into()),
            ChatMessage::assistant("reply".into()),
            ChatMessage::user("the real prompt".into()),
        ];
        assert_eq!(
            select_requeue_text(&messages),
            Some("the real prompt".to_owned())
        );
    }

    // Regression: when the transcript already ends on a compact boundary, the
    // selector must SKIP the boundary's "This session is being continued…"
    // summary prose and recover the genuine user prompt before it — replaying
    // the summary as the user's prompt was the bug.
    #[test]
    fn requeue_skips_compact_boundary_robust() {
        let messages = vec![
            ChatMessage::user("genuine prompt".into()),
            ChatMessage::assistant("reply".into()),
            ChatMessage::compact_boundary("a long summary of the session", 120_000),
        ];
        let recovered = select_requeue_text(&messages).expect("must recover the genuine prompt");
        assert_eq!(recovered, "genuine prompt");
        assert!(
            !recovered.contains("This session is being continued"),
            "must not replay the compact-boundary summary as the user's prompt"
        );
    }

    // Regression: a structured multi-text user message must be joined in full,
    // not truncated to its opening block.
    #[test]
    fn requeue_joins_multi_text_parts_robust() {
        let msg = ChatMessage {
            role: Role::User,
            parts: vec![
                MessagePart::Text("part one".into()),
                MessagePart::Text("   ".into()), // blank → dropped
                MessagePart::Text("part two".into()),
            ],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
            created_at: 0,
        };
        assert_eq!(
            recoverable_requeue_text(&msg),
            Some("part one\n\npart two".to_owned())
        );
    }

    // Edge: an attachment-only / whitespace-only user message yields no
    // replayable text, so the re-queue is skipped (None) rather than
    // submitting an empty prompt.
    #[test]
    fn requeue_skips_textless_message_edge() {
        let msg = ChatMessage {
            role: Role::User,
            parts: vec![MessagePart::Text("   \n  ".into())],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
            created_at: 0,
        };
        assert_eq!(recoverable_requeue_text(&msg), None);
    }
}
