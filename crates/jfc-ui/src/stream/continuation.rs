use tokio::sync::mpsc;

use crate::app::App;
use crate::runtime::AppEvent;
use crate::types::*;

use super::{
    messages::{
        build_provider_messages_for_pause_turn_resume, build_provider_messages_with_tool_results,
    },
    stream_response,
};

pub(crate) fn should_continue_loop(messages: &[ChatMessage]) -> bool {
    let last = match messages.iter().rev().find(|m| m.role == Role::Assistant) {
        Some(m) => m,
        None => {
            tracing::trace!(target: "jfc::stream", "should_continue_loop: no assistant message found");
            return false;
        }
    };
    let has_tools = last.parts.iter().any(|p| matches!(p, MessagePart::Tool(_)));
    if !has_tools {
        tracing::trace!(target: "jfc::stream", "should_continue_loop: last assistant has no tools");
        return false;
    }
    let all_done = last.parts.iter().all(|p| match p {
        MessagePart::Tool(tc) => {
            tc.status == ToolStatus::Completed || tc.status == ToolStatus::Failed
        }
        _ => true,
    });
    tracing::debug!(
        target: "jfc::stream",
        has_tools, all_done,
        tool_count = last.parts.iter().filter(|p| matches!(p, MessagePart::Tool(_))).count(),
        "should_continue_loop"
    );
    all_done
}

/// Resume an Anthropic server-side sampling loop after `stop_reason: "pause_turn"`.
///
/// Mirrors [`continue_agentic_loop`]'s setup (new empty assistant slot,
/// fresh sub-stream clocks, cleared thinking timestamps), but builds the
/// provider request via [`build_provider_messages_for_pause_turn_resume`]
/// so we do NOT inject a synthetic `"Continue from where you left off."`
/// user message. Anthropic's pause_turn protocol (cli.js v142:622686)
/// requires re-sending the conversation as-is — the trailing assistant's
/// `server_tool_use` block IS the resume signal. Adding a fake user turn
/// would tell the server "the human typed something" and break the
/// server-side loop's resumption logic.
///
/// Wire shape difference from `continue_agentic_loop`:
///   - continue_agentic_loop (post-tool):  ..., user_with_tool_results, [new assistant slot]
///   - continue_after_pause_turn:          ..., assistant_with_server_tool_use, [new assistant slot]
///
/// The synthetic-user-injection step is skipped specifically for the
/// trailing-assistant case; consecutive same-role merging and
/// empty-assistant stripping still apply.
pub(crate) async fn continue_after_pause_turn(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let assistant_idx = app.messages.len();
    tracing::info!(
        target: "jfc::stream",
        assistant_idx,
        model = %app.model,
        total_messages = app.messages.len(),
        "continue_after_pause_turn: resuming server-side sampling loop"
    );
    #[cfg(debug_assertions)]
    if let Err(err) = crate::types::validate_turn_invariants_inner(
        &app.messages,
        /* allow_streaming_tail = */ true,
    ) {
        tracing::warn!(
            target: "jfc::stream::invariants",
            error = %err,
            assistant_idx,
            "continue_after_pause_turn: turn-invariant violation BEFORE staging new assistant slot"
        );
    }
    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.last_stream_event_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);
    app.thinking_started_at = None;
    app.thinking_ended_at = None;

    let provider = app.provider.clone();
    let messages = build_provider_messages_for_pause_turn_resume(&app.messages[..assistant_idx]);
    let model = app.model.clone();
    let tx = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    let cancel = app.cancel_token.clone();

    tokio::spawn(async move {
        stream_response(provider, messages, model, tx, interrupt, cancel).await;
    });
}

pub(crate) async fn continue_agentic_loop(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let assistant_idx = app.messages.len();
    tracing::info!(
        target: "jfc::stream",
        assistant_idx,
        model = %app.model,
        total_messages = app.messages.len(),
        "continue_agentic_loop: starting new sub-stream"
    );
    // Debug-only invariant check BEFORE we stage the next assistant
    // slot. If the caller handed us a broken slice (e.g. trailing
    // assistant from the prior round wasn't merged), surface it in
    // the log instead of silently doubling down. Behind cfg() so
    // release builds skip the walk.
    #[cfg(debug_assertions)]
    if let Err(err) = crate::types::validate_turn_invariants_inner(
        &app.messages,
        /* allow_streaming_tail = */ true,
    ) {
        tracing::warn!(
            target: "jfc::stream::invariants",
            error = %err,
            assistant_idx,
            "continue_agentic_loop: turn-invariant violation BEFORE staging new assistant slot"
        );
    }
    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    // NOTE: do NOT reset streaming_response_bytes here -- it accumulates
    // across the entire user turn (all agentic loop iterations). The spinner
    // shows the cumulative token estimate for the full turn, matching v126's
    // responseLengthRef which persists across sub-streams.
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    // The sub-stream clock restarts (Anthropic restarts `output_tokens`
    // per request) but the *user-turn* clock keeps running -- set in
    // `handle_submit_text` and only cleared when the loop concludes.
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.last_stream_event_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);
    // Clear the thinking timestamps so the next sub-stream's spinner doesn't
    // render a stale "thought for Ns · almost done thinking" while the new
    // request is still in-flight. The next ThinkingDelta event will re-stamp
    // `thinking_started_at`; if the new turn isn't an extended-thinking one,
    // the spinner correctly shows the composing state instead.
    app.thinking_started_at = None;
    app.thinking_ended_at = None;

    let provider = app.provider.clone();
    let messages = build_provider_messages_with_tool_results(&app.messages[..assistant_idx]);
    let model = app.model.clone();
    let tx = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    let cancel = app.cancel_token.clone();

    // wg-async: the agentic continuation IS critical state -- it produces
    // the next sub-stream's events. Hand it the cancel token so a mid-loop
    // ESC unwinds it the same way it unwinds the original turn.
    tokio::spawn(async move {
        stream_response(provider, messages, model, tx, interrupt, cancel).await;
    });
}

#[cfg(test)]
mod should_continue_loop_tests {
    use super::*;

    fn assistant_with_tool(status: ToolStatus) -> ChatMessage {
        ChatMessage::assistant_parts(vec![MessagePart::Tool(ToolCall {
            id: "toolu_x".into(),
            kind: ToolKind::Bash,
            status,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        })])
    }

    // Normal: when the last assistant has all tools Complete, the loop
    // continues so the agent gets a chance to react to the tool results.
    #[test]
    fn continues_when_all_tools_complete_normal() {
        let msgs = vec![assistant_with_tool(ToolStatus::Completed)];
        assert!(should_continue_loop(&msgs));
    }

    // Normal: a Failed tool also signals "go again" -- the agent might try
    // a different approach.
    #[test]
    fn continues_when_tool_failed_normal() {
        let msgs = vec![assistant_with_tool(ToolStatus::Failed)];
        assert!(should_continue_loop(&msgs));
    }

    // Robust: a Pending tool means the user hasn't approved yet, so the loop
    // does NOT continue (we'd send a half-assembled state to the model).
    #[test]
    fn does_not_continue_when_tool_pending_robust() {
        let msgs = vec![assistant_with_tool(ToolStatus::Pending)];
        assert!(!should_continue_loop(&msgs));
    }

    // Robust: a Running tool is still in flight -- the loop must wait.
    #[test]
    fn does_not_continue_when_tool_running_robust() {
        let msgs = vec![assistant_with_tool(ToolStatus::Running)];
        assert!(!should_continue_loop(&msgs));
    }

    // Normal: assistant turn with no tools (pure prose) -> loop terminates.
    // The agent finished its turn cleanly.
    #[test]
    fn does_not_continue_for_text_only_assistant_normal() {
        let msgs = vec![ChatMessage::assistant("done".into())];
        assert!(!should_continue_loop(&msgs));
    }

    // Robust: empty conversation -> no continuation. Used when the session is
    // freshly resumed and there's nothing to react to.
    #[test]
    fn does_not_continue_when_no_assistant_robust() {
        let msgs = vec![ChatMessage::user("hi".into())];
        assert!(!should_continue_loop(&msgs));
    }

    // Robust: completely empty message list -- defensive check for the
    // resume-from-disk code path.
    #[test]
    fn does_not_continue_on_empty_messages_robust() {
        let msgs: Vec<ChatMessage> = vec![];
        assert!(!should_continue_loop(&msgs));
    }
}

#[cfg(test)]
mod cancellation_token_tests {
    //! Regression tests for the wg-async cancellation pattern.
    //!
    //! Background: spawn sites in `stream.rs` and `event_loop.rs` used
    //! to take an `Arc<AtomicBool>` that the spawned task polled between
    //! iterations. A blocking provider call mid-tick could miss the flag
    //! for seconds. We migrated the long-running spawn sites to also
    //! receive a `tokio_util::sync::CancellationToken` so they can race
    //! their work against `.cancelled()` via `tokio::select!`. These
    //! tests pin that contract: cancelling the token must unwind the
    //! spawned task within a single tokio scheduler tick.
    use tokio_util::sync::CancellationToken;

    /// Normal: a task that races a long sleep against `.cancelled()`
    /// returns immediately when the token is cancelled, instead of
    /// waiting for the sleep to finish. This is the core latency win
    /// over the AtomicBool-poll pattern.
    #[tokio::test]
    async fn cancel_during_spawn_unwinds_within_one_tick_normal() {
        let cancel = CancellationToken::new();
        let cancel_for_task = cancel.clone();

        // Spawn a task that mirrors the migrated stream_response select!
        // shape: a long fake "stream read" raced against cancellation.
        let handle = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel_for_task.cancelled() => "cancelled",
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => "completed",
            }
        });

        // Cancel after a microsleep so the task has actually started
        // polling its select! arms before the signal lands.
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        cancel.cancel();

        // The whole join must finish well under the 60-second sleep --
        // give it 500ms of headroom for slow CI runners.
        let outcome = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("spawn must unwind within 500ms after cancel()")
            .expect("spawned task must not panic");

        assert_eq!(outcome, "cancelled");
    }

    /// Robust: cancelling the token BEFORE the task gets to its first
    /// poll still unwinds it -- `cancelled()` returns immediately when
    /// the token is already in the cancelled state. Without this, a
    /// task spawned between the user's ESCx2 and the runtime actually
    /// scheduling it could miss the cancel and run to completion.
    #[tokio::test]
    async fn cancel_before_task_starts_still_short_circuits_robust() {
        let cancel = CancellationToken::new();
        // Cancel BEFORE the spawn so the cloned token enters the task
        // already in the cancelled state.
        cancel.cancel();
        let cancel_for_task = cancel.clone();

        let handle = tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel_for_task.cancelled() => "cancelled",
                _ = tokio::time::sleep(std::time::Duration::from_secs(60)) => "completed",
            }
        });

        let outcome = tokio::time::timeout(std::time::Duration::from_millis(500), handle)
            .await
            .expect("pre-cancelled token must short-circuit the spawn")
            .expect("spawned task must not panic");

        assert_eq!(outcome, "cancelled");
    }

    /// Robust: a fresh token is not poisoned by a previously-cancelled
    /// sibling. The migration mints a new token on every user submit;
    /// if that mint were a no-op the next turn would be DOA. This pins
    /// `CancellationToken::new()` semantics for the post-interrupt
    /// new-turn path.
    #[tokio::test]
    async fn fresh_token_is_not_cancelled_robust() {
        let prior = CancellationToken::new();
        prior.cancel();
        assert!(prior.is_cancelled());

        // `App::handle_submit_text` and the StreamError handler both do
        // `app.cancel_token = CancellationToken::new();` after a cancel.
        let fresh = CancellationToken::new();
        assert!(!fresh.is_cancelled());

        // And cloning the fresh token doesn't observe the prior one's
        // cancelled state -- they're independent.
        let fresh_clone = fresh.clone();
        assert!(!fresh_clone.is_cancelled());
    }
}
