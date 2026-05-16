use jfc_provider::ProviderMessage;
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

/// Stage a fresh assistant message slot to stream the next sub-stream's
/// output into. Common setup shared by `continue_agentic_loop` (post-
/// tool-result continuation) and `continue_after_pause_turn` (Anthropic
/// server-side sampling loop resumption).
///
/// Returns the index of the newly-pushed assistant message so callers
/// can slice `&app.messages[..assistant_idx]` to build the resend
/// request without including the empty placeholder slot.
///
/// What this resets:
///   * `streaming_text` / `streaming_reasoning` — fresh per sub-stream.
///   * Sub-stream clocks (`streaming_started_at`, `last_stream_event_at`,
///     `streaming_last_token_at`) — Anthropic restarts `output_tokens`
///     per request, so the sub-stream clock has to restart too.
///   * `last_usage_output` / `usage_apply_baseline` — cumulative delta
///     accounting resets at the sub-stream boundary.
///   * Thinking timestamps — the next sub-stream may not emit thinking,
///     so cleared so a stale "thought for Ns" doesn't render.
///
/// What this preserves:
///   * `streaming_response_bytes` — accumulates across the WHOLE user
///     turn (all agentic loop iterations); the spinner's cumulative
///     token estimate depends on it persisting (v126:responseLengthRef).
///   * `turn_started_at` — wall clock for "Cooked for Nm Ns" footer,
///     owned by `handle_submit_text` and cleared only at turn end.
fn setup_new_substream_slot(app: &mut App, label: &'static str) -> usize {
    let assistant_idx = app.messages.len();
    tracing::info!(
        target: "jfc::stream",
        assistant_idx,
        model = %app.model,
        total_messages = app.messages.len(),
        sub_stream = label,
        "setup_new_substream_slot: staging new assistant slot"
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
            sub_stream = label,
            "setup_new_substream_slot: turn-invariant violation BEFORE staging new assistant slot"
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
    assistant_idx
}

/// Spawn the actual provider stream task with the cancel token, mirror
/// the wg-async pattern used everywhere else (atomic flag + token).
///
/// This is a separate function (rather than inlined at the call site)
/// so the two continuation entry points share an identical spawn shape
/// — including the cancel-token plumbing. A divergence here was how
/// the legacy code accidentally let one path skip the token and miss
/// ESCx2 unwinds.
fn spawn_substream(app: &App, messages: Vec<ProviderMessage>, tx: &mpsc::Sender<AppEvent>) {
    let provider = app.provider.clone();
    let model = app.model.clone();
    let tx = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    let cancel = app.cancel_token.clone();
    tokio::spawn(async move {
        stream_response(provider, messages, model, tx, interrupt, cancel, None).await;
    });
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
    let assistant_idx = setup_new_substream_slot(app, "pause_turn_resume");
    let messages = build_provider_messages_for_pause_turn_resume(&app.messages[..assistant_idx]);
    spawn_substream(app, messages, tx);
}

pub(crate) async fn continue_agentic_loop(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let assistant_idx = setup_new_substream_slot(app, "agentic_loop");
    let messages = build_provider_messages_with_tool_results(&app.messages[..assistant_idx]);
    spawn_substream(app, messages, tx);
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

#[cfg(test)]
mod setup_new_substream_slot_tests {
    //! Pins the contract `continue_agentic_loop` and
    //! `continue_after_pause_turn` share: every sub-stream gets a
    //! fresh assistant slot, fresh sub-stream clocks, cleared
    //! thinking timestamps, and the cumulative
    //! `streaming_response_bytes` counter SURVIVES across sub-streams
    //! so the spinner shows turn-total tokens, not per-sub-stream
    //! tokens (matches v126 responseLengthRef).
    use super::*;
    use std::sync::Arc;

    /// Tiny no-op provider — needed because `App::new` requires a
    /// concrete `Arc<dyn Provider>`. The continuation tests never
    /// dispatch real streams; the provider only has to exist. Mirrors
    /// the test-provider shape in `crate::app::tests`.
    struct NoopProvider;
    #[async_trait::async_trait]
    impl jfc_provider::Provider for NoopProvider {
        fn name(&self) -> &str {
            "test"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl jfc_provider::seal::Sealed for NoopProvider {}

    fn fresh_app_with(messages: Vec<ChatMessage>) -> App {
        let mut app = App::new(Arc::new(NoopProvider), "test-model");
        app.messages = messages;
        app
    }

    // Normal: a fresh sub-stream pushes a new empty assistant message
    // and returns its index. The previous trailing message stays
    // intact (the helper does NOT mutate or merge it).
    #[test]
    fn setup_pushes_empty_assistant_slot_and_returns_index_normal() {
        let mut app = fresh_app_with(vec![ChatMessage::user("hi".into())]);
        let idx = setup_new_substream_slot(&mut app, "test");
        assert_eq!(idx, 1, "assistant_idx must be the post-push index");
        assert_eq!(app.messages.len(), 2);
        assert_eq!(app.messages[idx].role, Role::Assistant);
        assert!(
            app.messages[idx]
                .parts
                .iter()
                .all(|p| matches!(p, MessagePart::Text(s) if s.is_empty())),
            "new assistant slot must be empty"
        );
        assert_eq!(app.streaming_assistant_idx, Some(idx));
        assert!(app.is_streaming);
    }

    // Normal: every reset field that the original two functions used
    // to set is set here too, so callers can drop their own copies.
    #[test]
    fn setup_resets_per_substream_state_normal() {
        let mut app = fresh_app_with(vec![ChatMessage::user("hi".into())]);
        // Pretend the previous sub-stream left state behind.
        app.streaming_text.push_str("leftover text");
        app.streaming_reasoning.push_str("leftover reasoning");
        app.last_usage_output = 999;
        app.usage_apply_baseline = (1, 2, 3, 4);
        app.thinking_started_at = Some(std::time::Instant::now());
        app.thinking_ended_at = Some(std::time::Instant::now());

        let _ = setup_new_substream_slot(&mut app, "test");

        assert!(
            app.streaming_text.is_empty(),
            "streaming_text must reset per sub-stream"
        );
        assert!(
            app.streaming_reasoning.is_empty(),
            "streaming_reasoning must reset per sub-stream"
        );
        assert_eq!(app.last_usage_output, 0);
        assert_eq!(app.usage_apply_baseline, (0, 0, 0, 0));
        assert!(app.thinking_started_at.is_none());
        assert!(app.thinking_ended_at.is_none());
        assert!(app.streaming_started_at.is_some());
        assert!(app.last_stream_event_at.is_some());
        assert!(app.streaming_last_token_at.is_some());
    }

    // Robust: streaming_response_bytes is the cumulative per-USER-TURN
    // counter and MUST survive a sub-stream restart. Regressing this
    // makes the spinner display "Brewed for 5s · ↓ 0k tokens" on
    // every agentic step instead of accumulating across the turn.
    #[test]
    fn setup_preserves_cumulative_response_bytes_robust() {
        let mut app = fresh_app_with(vec![ChatMessage::user("hi".into())]);
        app.streaming_response_bytes = 12_345;
        let _ = setup_new_substream_slot(&mut app, "test");
        assert_eq!(
            app.streaming_response_bytes, 12_345,
            "streaming_response_bytes must persist across sub-streams (v126 responseLengthRef)"
        );
    }
}

#[cfg(test)]
mod pause_turn_end_to_end_tests {
    //! Integration test for the full pause_turn dispatch path: stage a
    //! conversation that ends with a server_tool_use trailing assistant
    //! (the wire shape Anthropic produces when stop_reason=pause_turn
    //! fires), call the resume-mode builder, and pin the EXACT wire
    //! shape the next request will carry.
    //!
    //! We don't spin the entire `event_loop::run()` here — that requires
    //! a real provider, channel plumbing, and ratatui surface — but
    //! we do exercise every step from the dispatch ladder's
    //! `StopReason::PauseTurn` branch onward:
    //!
    //!   1. setup_new_substream_slot pushes a fresh assistant slot.
    //!   2. build_provider_messages_for_pause_turn_resume slices the
    //!      messages up to the new slot.
    //!   3. The resulting ProviderMessage Vec is what `stream_response`
    //!      will send to Anthropic.
    //!
    //! That third step is the one prior unit tests didn't pin in
    //! sequence — they tested the builder in isolation. Here we
    //! exercise the full pre-staged + slice + build chain so any
    //! regression in setup_new_substream_slot (e.g. it stops pushing
    //! the empty trailing assistant) immediately surfaces a wrong
    //! wire shape.
    use super::*;
    use crate::ids::ToolId;
    use crate::types::{
        ChatMessage, MessagePart, Role, ToolCall, ToolDisplayState, ToolInput, ToolKind,
        ToolOutput, ToolStatus,
    };
    use jfc_provider::{ProviderContent, ProviderRole, ServerToolResultKind};
    use std::sync::Arc;

    /// Mirror of `setup_new_substream_slot_tests::NoopProvider` — the
    /// integration test reuses the same dummy provider because
    /// `App::new` requires one even when no stream actually fires.
    struct NoopProvider;
    #[async_trait::async_trait]
    impl jfc_provider::Provider for NoopProvider {
        fn name(&self) -> &str {
            "test"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl jfc_provider::seal::Sealed for NoopProvider {}

    fn server_tool(id: &str, status: ToolStatus, output: ToolOutput) -> ToolCall {
        ToolCall {
            id: ToolId::from(id),
            kind: ToolKind::ServerWebSearch,
            status,
            input: ToolInput::Generic {
                summary: "rust language".into(),
            },
            output,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }
    }

    /// Normal: a turn that ended with stop_reason=pause_turn — assistant
    /// emitted a server_tool_use that hasn't received its paired
    /// result yet — produces a resend payload whose LAST provider
    /// message is the trailing assistant with the ServerToolUse
    /// block, NOT a synthetic "Continue from where you left off."
    /// user message.
    ///
    /// This is the exact wire shape Anthropic looks for in
    /// cli.js v142:622686: re-send the conversation, the trailing
    /// `server_tool_use` is the resume signal.
    #[tokio::test]
    async fn pause_turn_resend_wire_shape_pin_normal() {
        // Stage the conversation the way the runtime would have it
        // at the moment stop_reason=pause_turn fires:
        //   user → assistant(text + server_tool_use, Running)
        let mut app = App::new(Arc::new(NoopProvider), "test-model");
        app.messages = vec![
            ChatMessage::user("research rust".into()),
            ChatMessage::assistant_parts(vec![
                MessagePart::Text("Looking it up.".into()),
                MessagePart::Tool(server_tool(
                    "srvtoolu_1",
                    ToolStatus::Running,
                    ToolOutput::Empty,
                )),
            ]),
        ];

        // setup_new_substream_slot is what the event_loop's
        // PauseTurn branch ultimately invokes (via
        // continue_after_pause_turn). After this call, app.messages
        // has a trailing empty assistant placeholder; the resend
        // builder must slice it off.
        let assistant_idx = setup_new_substream_slot(&mut app, "pause_turn_resume");
        assert_eq!(
            assistant_idx, 2,
            "fresh slot index must be the post-push position"
        );
        assert_eq!(
            app.messages.len(),
            3,
            "trailing empty assistant must be staged"
        );

        // Build the resend payload — this is what stream_response
        // will hand to the provider.
        let payload = build_provider_messages_for_pause_turn_resume(&app.messages[..assistant_idx]);

        // Hard pins on the wire shape:
        //
        //   * Exactly 2 ProviderMessages: user + assistant.
        //   * NO synthetic "Continue from where you left off." user.
        //   * Last message is an assistant carrying a ServerToolUse
        //     block (the resume cue).
        assert_eq!(
            payload.len(),
            2,
            "resend payload must be [user, assistant] — got {:?}",
            payload.iter().map(|m| m.role).collect::<Vec<_>>()
        );
        assert_eq!(payload[0].role, ProviderRole::User);
        assert_eq!(payload[1].role, ProviderRole::Assistant);

        let has_synthetic_continue = payload.iter().any(|m| {
            m.role == ProviderRole::User
                && m.content.iter().any(
                    |c| matches!(c, ProviderContent::Text(t) if t.contains("Continue from where you left off")),
                )
        });
        assert!(
            !has_synthetic_continue,
            "pause_turn resume must not inject 'Continue from where you left off.' (cli.js v142:622686)"
        );

        // The trailing assistant carries the ServerToolUse — that's
        // the resume cue. Wire name is the bare "web_search", NOT
        // the JFC-internal "server_tool_use:web_search" prefix.
        let trailing = &payload[1];
        let has_server_tool_use = trailing.content.iter().any(
            |c| matches!(c, ProviderContent::ServerToolUse { name, .. } if name == "web_search"),
        );
        assert!(
            has_server_tool_use,
            "trailing assistant must carry a ProviderContent::ServerToolUse{{name='web_search'}} as the resume cue"
        );

        // No fabricated tool_result anywhere — that would tell the
        // server "the client already handled the tool" and break
        // server-side resumption.
        let has_tool_result = payload.iter().any(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, ProviderContent::ToolResult { .. }))
        });
        assert!(
            !has_tool_result,
            "pause_turn resend must not include a synthetic tool_result for the server_tool_use"
        );
    }

    /// Normal: when the runtime captured the paired server_tool_result
    /// before pause_turn fired (rare but possible: result block arrived,
    /// then loop hit iteration cap before the next text block), the
    /// resend payload re-emits the result on the SAME assistant message
    /// as the server_tool_use. Cli.js v142:441375 wire shape.
    #[tokio::test]
    async fn pause_turn_resend_carries_paired_result_when_present_normal() {
        let mut tool = server_tool(
            "srvtoolu_2",
            ToolStatus::Running,
            ToolOutput::ServerToolResult {
                tool_kind: ServerToolResultKind::WebSearch,
                content: serde_json::json!([
                    { "title": "Rust", "url": "https://rust-lang.org" }
                ]),
            },
        );
        let _ = tool.mark_completed();

        let mut app = App::new(Arc::new(NoopProvider), "test-model");
        app.messages = vec![
            ChatMessage::user("research rust".into()),
            ChatMessage::assistant_parts(vec![
                MessagePart::Text("Looking it up.".into()),
                MessagePart::Tool(tool),
            ]),
        ];

        let assistant_idx = setup_new_substream_slot(&mut app, "pause_turn_resume");
        let payload = build_provider_messages_for_pause_turn_resume(&app.messages[..assistant_idx]);

        // Trailing assistant carries BOTH the server_tool_use AND
        // the server_tool_result, in that order. Same wire shape
        // cli.js v142:441375 produces on resend.
        let trailing = &payload[1];
        assert_eq!(trailing.role, ProviderRole::Assistant);
        let server_tool_use_count = trailing
            .content
            .iter()
            .filter(|c| matches!(c, ProviderContent::ServerToolUse { .. }))
            .count();
        let server_tool_result_count = trailing
            .content
            .iter()
            .filter(|c| matches!(c, ProviderContent::ServerToolResult { .. }))
            .count();
        assert_eq!(server_tool_use_count, 1);
        assert_eq!(
            server_tool_result_count, 1,
            "paired ServerToolResult must round-trip on the SAME assistant message"
        );

        // The server_tool_use MUST come before the server_tool_result
        // in the content vec — cli.js v142 pairs them in that
        // specific order and the server matches on adjacency.
        let mut use_idx = None;
        let mut result_idx = None;
        for (i, c) in trailing.content.iter().enumerate() {
            match c {
                ProviderContent::ServerToolUse { .. } => use_idx = Some(i),
                ProviderContent::ServerToolResult { .. } => result_idx = Some(i),
                _ => {}
            }
        }
        let use_idx = use_idx.unwrap();
        let result_idx = result_idx.unwrap();
        assert!(
            use_idx < result_idx,
            "ServerToolUse must come before ServerToolResult on the same assistant message"
        );
    }

    /// Robust: a multi-sub-stream agentic turn that hits pause_turn
    /// on the FINAL sub-stream produces a resend payload that still
    /// has the user at index 0, the assistant in between, and NO
    /// synthetic-continue filler. Pins that
    /// `setup_new_substream_slot` interacts correctly with the
    /// resume-mode builder when the runtime has accumulated several
    /// per-sub-stream assistant placeholders.
    #[tokio::test]
    async fn pause_turn_after_multi_substream_agentic_turn_robust() {
        let mut app = App::new(Arc::new(NoopProvider), "test-model");
        // user → A1 (text) → A2 (server_tool_use, pause_turn fires here)
        // The actual app.messages would still have both assistants
        // separated until session save coalesces them.
        app.messages = vec![
            ChatMessage::user("research rust then summarize".into()),
            ChatMessage::assistant("Sure, searching now.".into()),
            ChatMessage::assistant_parts(vec![MessagePart::Tool(server_tool(
                "srvtoolu_3",
                ToolStatus::Running,
                ToolOutput::Empty,
            ))]),
        ];

        let assistant_idx = setup_new_substream_slot(&mut app, "pause_turn_resume");
        let payload = build_provider_messages_for_pause_turn_resume(&app.messages[..assistant_idx]);

        // The resume-mode builder's merge step collapses the two
        // adjacent assistants into one provider message, so the
        // payload should be [user, assistant(merged)].
        assert_eq!(
            payload.len(),
            2,
            "multi-sub-stream assistants merge into one on resend — got {:?}",
            payload.iter().map(|m| m.role).collect::<Vec<_>>()
        );
        assert_eq!(payload[1].role, ProviderRole::Assistant);

        // The merged assistant carries BOTH the text from A1 AND
        // the server_tool_use from A2.
        let has_text = payload[1]
            .content
            .iter()
            .any(|c| matches!(c, ProviderContent::Text(t) if t.contains("searching now")));
        let has_server_tool_use = payload[1]
            .content
            .iter()
            .any(|c| matches!(c, ProviderContent::ServerToolUse { .. }));
        assert!(has_text, "merged assistant must preserve A1's text");
        assert!(
            has_server_tool_use,
            "merged assistant must preserve A2's server_tool_use as the resume cue"
        );

        // And still no synthetic-continue filler.
        let has_synthetic_continue = payload.iter().any(|m| {
            m.role == ProviderRole::User
                && m.content.iter().any(
                    |c| matches!(c, ProviderContent::Text(t) if t.contains("Continue from where you left off")),
                )
        });
        assert!(
            !has_synthetic_continue,
            "pause_turn resume must not inject 'Continue from where you left off.'"
        );
    }

    /// Robust: mixed-mode pause_turn — the response carried BOTH a
    /// local tool (Bash) AND server_tool_use, and stop_reason was
    /// pause_turn. After local tools complete, the event_loop's
    /// AllToolsComplete handler routes via the pause-turn-resume
    /// builder (when `pending_pause_turn_resume` is latched). The
    /// resulting wire shape:
    ///
    ///   1. user: original prompt
    ///   2. assistant: [text, server_tool_use, local_tool_use]
    ///   3. user: [tool_result for local_tool_use]
    ///   4. (no synthetic-Continue trailer — pause_turn-resume mode)
    ///
    /// The `server_tool_use` block in (2) is still in the conversation
    /// and Anthropic's server-side loop matches on its presence
    /// (cli.js v142:7057 — discriminator is `type === "server_tool_use"`,
    /// not adjacency). The KEY contract: NO synthetic Continue.
    #[tokio::test]
    async fn mixed_mode_pause_turn_resend_omits_synthetic_continue_robust() {
        let mut app = App::new(Arc::new(NoopProvider), "test-model");
        let local_tool = ToolCall {
            id: ToolId::from("toolu_local_1"),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Command {
                stdout: "file.txt\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        };
        app.messages = vec![
            ChatMessage::user("research rust and run ls".into()),
            ChatMessage::assistant_parts(vec![
                MessagePart::Text("Searching and listing.".into()),
                MessagePart::Tool(server_tool(
                    "srvtoolu_mix",
                    ToolStatus::Running,
                    ToolOutput::Empty,
                )),
                MessagePart::Tool(local_tool),
            ]),
        ];

        let assistant_idx = setup_new_substream_slot(&mut app, "pause_turn_mixed_resume");
        let payload = build_provider_messages_for_pause_turn_resume(&app.messages[..assistant_idx]);

        // The assistant turn carries text + server_tool_use + local tool_use,
        // and the local tool's tool_result follows as a trailing user
        // message. NO synthetic-Continue is appended.
        let has_synthetic_continue = payload.iter().any(|m| {
            m.role == ProviderRole::User
                && m.content.iter().any(
                    |c| matches!(c, ProviderContent::Text(t) if t.contains("Continue from where you left off")),
                )
        });
        assert!(
            !has_synthetic_continue,
            "mixed-mode pause_turn resume must not inject 'Continue from where you left off.'"
        );

        // The server_tool_use is preserved as the resume cue. cli.js
        // v142:7057 matches on the `type` field, not adjacency.
        let assistant = payload
            .iter()
            .find(|m| m.role == ProviderRole::Assistant)
            .expect("expected an assistant message");
        let has_server_tool_use = assistant.content.iter().any(
            |c| matches!(c, ProviderContent::ServerToolUse { name, .. } if name == "web_search"),
        );
        assert!(
            has_server_tool_use,
            "assistant must still carry the server_tool_use block as the resume cue"
        );

        // The local Bash tool_use round-trips as a regular tool_use
        // (NOT server_tool_use) — only the server-side tool gets the
        // server_tool_use wire type.
        let has_local_tool_use = assistant.content.iter().any(|c| {
            matches!(c, ProviderContent::ToolUse { name, .. } if name.eq_ignore_ascii_case("bash"))
        });
        assert!(
            has_local_tool_use,
            "local Bash must round-trip as plain ProviderContent::ToolUse"
        );

        // The local tool's tool_result is on the trailing user message.
        let trailing = payload
            .last()
            .expect("payload must have at least one message");
        assert_eq!(
            trailing.role,
            ProviderRole::User,
            "mixed-mode trailing message must be the local tool_result user turn"
        );
        let has_tool_result = trailing
            .content
            .iter()
            .any(|c| matches!(c, ProviderContent::ToolResult { .. }));
        assert!(
            has_tool_result,
            "trailing user message must carry the local tool_result"
        );
    }

    /// Normal: the `pending_pause_turn_resume` flag on `App` defaults
    /// to false on a fresh App. Pins the default so a later regression
    /// (e.g. flag flipped to true by accident in the constructor)
    /// doesn't silently re-route every turn through pause-turn-resume
    /// — which would break the "continue from where you left off"
    /// behavior on the normal agentic continuation path.
    #[test]
    fn fresh_app_has_pause_turn_resume_unlatched_normal() {
        let app = App::new(Arc::new(NoopProvider), "test-model");
        assert!(
            !app.pending_pause_turn_resume,
            "pending_pause_turn_resume must default to false on a fresh App"
        );
    }
}
