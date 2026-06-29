use jfc_provider::ProviderMessage;
use tokio::sync::mpsc;

use crate::app::EngineState;
use crate::runtime::{EngineEvent, StreamRequestOverrides};
use crate::types::*;

use super::{
    compaction::{
        SUBAGENT_HISTORY_BUDGET_BYTES, cap_messages_for_budget, estimate_provider_message_bytes,
    },
    messages::{
        build_provider_messages_for_pause_turn_resume, build_provider_messages_with_tool_results,
    },
};

pub fn should_continue_loop(messages: &[ChatMessage]) -> bool {
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
    if last.parts.iter().any(|p| match p {
        MessagePart::Tool(tc) => is_incomplete_provider_tool_input(tc),
        _ => false,
    }) {
        tracing::warn!(
            target: "jfc::stream",
            "should_continue_loop: not continuing after incomplete provider tool input"
        );
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

fn is_incomplete_provider_tool_input(tc: &ToolCall) -> bool {
    if tc.status != ToolStatus::Failed {
        return false;
    }
    let ToolOutput::Text(text) = &tc.output else {
        return false;
    };
    text.contains("The provider stream finished before sending a complete `input` object")
}

/// Detect a "permission-asking stall": the assistant finished a chunk of work
/// and ended the turn by *asking whether to do the next obvious step* instead
/// of doing it. Corpus analysis of 133 turns where the user had to type
/// "continue" showed ~41% ended this way ("Want me to …?", a trailing
/// question, "shall I", "let me know", "next steps:"). In factory /
/// auto-continue mode this is the signal to self-continue rather than wait.
///
/// Operates on the tail of the last assistant message's plain text. Returns
/// `false` for genuine completions ("Done. Pushed `abc123`.") so we never
/// loop on finished work.
pub fn assistant_text_stalls(messages: &[ChatMessage]) -> bool {
    let Some(last) = messages.iter().rev().find(|m| m.role == Role::Assistant) else {
        return false;
    };
    // Tool-bearing turns are handled by `should_continue_loop`; this guard is
    // only for text-only conversational stalls.
    if last.parts.iter().any(|p| matches!(p, MessagePart::Tool(_))) {
        return false;
    }
    let text = last
        .parts
        .iter()
        .filter_map(|p| match p {
            MessagePart::Text(s) => Some(s.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return false;
    }

    // Inspect the closing window (last ~240 bytes) — stalls live at the end.
    // The window start is a raw byte offset, so snap it forward to the next
    // char boundary; otherwise a multi-byte char (e.g. an em-dash `—`) straddling
    // the cut would panic the slice.
    let mut tail_start = trimmed.len().saturating_sub(240);
    while tail_start < trimmed.len() && !trimmed.is_char_boundary(tail_start) {
        tail_start += 1;
    }
    let tail = trimmed[tail_start..].to_lowercase();

    // Strong phrase signals anywhere in the tail.
    const STALL_PHRASES: &[&str] = &[
        "want me to",
        "shall i ",
        "should i ",
        "would you like",
        "do you want",
        "let me know",
        "if you want",
        "if you'd like",
        "ready to proceed",
        "ready to implement",
        "want to pick",
        "any changes before",
        "or call this a stable checkpoint",
    ];
    if STALL_PHRASES.iter().any(|p| tail.contains(p)) {
        return true;
    }

    // A trailing question mark is a weaker but real signal: the turn ended on
    // a question, which in a factory context is a request for direction.
    trimmed.ends_with('?')
}

/// Whether self-continuation (auto-driving the next in-scope step without a
/// user "continue") is enabled. Sources, in order: the `JFC_AUTO_CONTINUE`
/// env var, then `[continuation] auto_continue` in config, then the autonomous
/// default/factory mode. Plan mode disables it unconditionally — the caller is
/// responsible for that check since config has no view of permission mode.
pub fn auto_continue_enabled() -> bool {
    if let Ok(v) = std::env::var("JFC_AUTO_CONTINUE") {
        let v = v.trim().to_ascii_lowercase();
        if matches!(v.as_str(), "1" | "true" | "yes" | "on") {
            return true;
        }
        if matches!(v.as_str(), "0" | "false" | "no" | "off") {
            return false;
        }
    }
    let cfg = crate::config::load_arc();
    if let Some(c) = cfg.continuation.as_ref()
        && !c.auto_continue
    {
        return false;
    }
    true
}

/// Max consecutive self-continuations before we stop and wait for the user —
/// prevents a runaway loop if the model keeps stalling. Configurable via
/// `[continuation] max_self_continuations` (default 25).
pub fn max_self_continuations() -> u32 {
    crate::config::load_arc()
        .continuation
        .as_ref()
        .map(|c| c.max_self_continuations)
        .unwrap_or(25)
}

/// Stage a fresh assistant message slot to stream the next sub-stream's
/// output into. Common setup shared by `continue_agentic_loop` (post-
/// tool-result continuation) and `continue_after_pause_turn` (Anthropic
/// server-side sampling loop resumption).
///
/// Returns the index of the newly-pushed assistant message so callers
/// can slice `&state.messages[..assistant_idx]` to build the resend
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
fn setup_new_substream_slot(state: &mut EngineState, label: &'static str) -> usize {
    let assistant_idx = state.messages.len();
    tracing::info!(
        target: "jfc::stream",
        assistant_idx,
        model = %state.model,
        total_messages = state.messages.len(),
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
        &state.messages,
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
    state.messages.push(ChatMessage::assistant(String::new()));
    let identity = crate::cache_lineage::current_identity(state);
    crate::cache_lineage::stamp_assistant(&mut state.messages, assistant_idx, &identity);
    state.streaming_text.clear();
    state.streaming_reasoning.clear();
    state.streaming_assistant_idx = Some(assistant_idx);
    state.is_streaming = true;
    let now = std::time::Instant::now();
    state.streaming_started_at = Some(now);
    state.last_stream_event_at = Some(now);
    state.streaming_last_token_at = Some(now);
    state.last_usage_output = 0;
    state.usage_apply_baseline = (0, 0, 0, 0);
    state.thinking_started_at = None;
    state.thinking_ended_at = None;
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
fn spawn_substream(
    state: &mut EngineState,
    messages: Vec<ProviderMessage>,
    tx: &mpsc::Sender<EngineEvent>,
) {
    let provider = state.provider.clone();
    let model = state.model.clone();
    let interrupt = state.interrupt_flag.clone();
    let cancel = state.cancel_token.clone();
    let overrides = StreamRequestOverrides {
        session_id: state
            .current_session_id
            .as_ref()
            .map(|s| s.as_str().to_owned()),
        provider_history_archive_seen: state.provider_history_archive_seen(),
        background_reminders: state.take_background_reminders(),
        disallowed_tools: state.effective_disallowed_tools(),
        allowed_tools: state.allowed_tools.clone(),
        custom_betas: state.custom_betas.clone(),
        fine_grained_tool_streaming: state.fine_grained_tool_streaming,
        strict_tool_schemas: state.strict_tool_schemas,
        task_budget: state.cli_task_budget,
        max_thinking_tokens: state.cli_max_thinking_tokens,
        thinking_display: state.cli_thinking_display.clone(),
        brief_mode: state.brief_mode,
        // Copy (do NOT reclassify): the active mode was resolved once when the
        // user submitted this turn and is held across its continuations so a
        // tool loop can't re-mode itself.
        interaction_mode: state.active_interaction_mode,
        context_hint_tokens_saved: state.take_context_hint_tokens_saved(),
        last_usage_input_tokens: Some(state.last_usage_input as u64),
        context_window_tokens: Some(state.max_context_tokens as u64),
        ..Default::default()
    };
    crate::runtime::spawn_stream_response_scoped(
        state, tx, provider, messages, model, interrupt, cancel, None, overrides,
    );
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
pub async fn continue_after_pause_turn(state: &mut EngineState, tx: &mpsc::Sender<EngineEvent>) {
    let assistant_idx = setup_new_substream_slot(state, "pause_turn_resume");
    let messages = build_provider_messages_for_pause_turn_resume(&state.messages[..assistant_idx]);
    spawn_substream(state, messages, tx);
}

/// Maximum API round-trips per user turn before hard-stop.
/// Default: unlimited (u32::MAX). Configurable via `JFC_MAX_AGENTIC_TURNS`
/// env var if a safety cap is desired.
const MAX_AGENTIC_TURNS: u32 = u32::MAX;

fn cap_main_continuation_history(provider_name: &str, messages: &mut Vec<ProviderMessage>) -> bool {
    if provider_name != "gemini" {
        return false;
    }

    let before_count = messages.len();
    let before_bytes: usize = messages.iter().map(estimate_provider_message_bytes).sum();
    let capped = cap_messages_for_budget(messages, SUBAGENT_HISTORY_BUDGET_BYTES);
    if capped {
        let after_bytes: usize = messages.iter().map(estimate_provider_message_bytes).sum();
        tracing::info!(
            target: "jfc::stream::budget",
            provider = provider_name,
            before_messages = before_count,
            after_messages = messages.len(),
            before_bytes,
            after_bytes,
            budget_bytes = SUBAGENT_HISTORY_BUDGET_BYTES,
            "capped Gemini continuation history before provider request"
        );
    }
    capped
}

pub async fn continue_agentic_loop(state: &mut EngineState, tx: &mpsc::Sender<EngineEvent>) {
    // Enforce max-turns safety limit. Without this a model stuck in a
    // retry loop (e.g. repeatedly failing Edit calls) runs indefinitely.
    state.agentic_turn_count += 1;
    let max = std::env::var("JFC_MAX_AGENTIC_TURNS")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(MAX_AGENTIC_TURNS);
    if state.agentic_turn_count >= max {
        tracing::error!(
            target: "jfc::stream",
            turn_count = state.agentic_turn_count,
            max,
            "max agentic turns exceeded — hard-stopping loop"
        );
        crate::toast::push_with_cap(
            &mut state.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Error,
                format!(
                    "Agentic loop hard-stopped at {max} turns. Use /clear or submit a new prompt."
                ),
            ),
        );
        return;
    }
    if state.agentic_turn_count == max.saturating_sub(10) {
        crate::toast::push_with_cap(
            &mut state.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                format!(
                    "Approaching turn limit ({}/{max}). The loop will stop at {max}.",
                    state.agentic_turn_count
                ),
            ),
        );
    }

    let assistant_idx = setup_new_substream_slot(state, "agentic_loop");
    let mut messages = build_provider_messages_with_tool_results(&state.messages[..assistant_idx]);
    cap_main_continuation_history(state.provider.name(), &mut messages);
    spawn_substream(state, messages, tx);
}

#[cfg(test)]
mod should_continue_loop_tests {
    use super::*;
    use jfc_provider::{ProviderContent, ProviderRole};

    fn assistant_with_tool(status: ToolStatus) -> ChatMessage {
        ChatMessage::assistant_parts(vec![MessagePart::tool(ToolCall {
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
            thought_signature: None,
        })])
    }

    fn assistant_with_failed_tool_output(output: ToolOutput) -> ChatMessage {
        ChatMessage::assistant_parts(vec![MessagePart::tool(ToolCall {
            id: "toolu_x".into(),
            kind: ToolKind::Write,
            status: ToolStatus::Failed,
            input: ToolInput::Generic {
                summary: "{\"file_path\":\"/tmp/context.rs\"".into(),
            },
            output,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
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

    // Robust: if the provider/gateway truncated a tool call before the JSON
    // arguments finished, do not immediately continue the agentic loop. The
    // retry tends to reproduce the same partial Write/Edit call and render a
    // duplicate failed tool block.
    #[test]
    fn does_not_continue_after_incomplete_provider_input_robust() {
        let msgs = vec![assistant_with_failed_tool_output(ToolOutput::Text(
            "Tool input was not valid JSON (82 bytes received): EOF while parsing an object at line 1 column 82\n\n\
             The provider stream finished before sending a complete `input` object. Retry the tool call with a properly-formed JSON input."
                .into(),
        ))];
        assert!(!should_continue_loop(&msgs));
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

    fn provider_user_text(s: impl Into<String>) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.into())],
        }
    }

    fn provider_assistant_text(s: impl Into<String>) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(s.into())],
        }
    }

    fn provider_tool_use(id: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: id.to_owned(),
                name: "Read".to_owned(),
                input: serde_json::json!({ "path": "x" }),
                thought_signature: None,
            }],
        }
    }

    fn provider_tool_result(id: &str, content: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: id.to_owned(),
                content: content.to_owned(),
                is_error: false,
            }],
        }
    }

    #[test]
    fn caps_gemini_continuation_history_normal() {
        let big = "x".repeat(80_000);
        let mut msgs = vec![provider_user_text("PROMPT")];
        for idx in 0..12 {
            let id = format!("toolu_{idx}");
            msgs.push(provider_tool_use(&id));
            msgs.push(provider_tool_result(&id, &big));
        }
        msgs.push(provider_assistant_text("recent tail"));

        let before_len = msgs.len();
        assert!(cap_main_continuation_history("gemini", &mut msgs));
        assert!(msgs.len() < before_len);
        let bytes: usize = msgs.iter().map(estimate_provider_message_bytes).sum();
        assert!(bytes <= SUBAGENT_HISTORY_BUDGET_BYTES + 256);
        match &msgs[1].content[0] {
            ProviderContent::Text(t) => assert!(t.contains("elided")),
            _ => panic!("expected budget marker"),
        }
        match &msgs.last().unwrap().content[0] {
            ProviderContent::Text(t) => assert_eq!(t, "recent tail"),
            _ => panic!("expected recent tail"),
        }
    }

    #[test]
    fn leaves_non_gemini_continuation_history_alone_normal() {
        let big = "x".repeat(80_000);
        let mut msgs = vec![
            provider_user_text("PROMPT"),
            provider_tool_use("toolu_1"),
            provider_tool_result("toolu_1", &big),
            provider_assistant_text("recent tail"),
        ];
        let before_len = msgs.len();
        let before_bytes: usize = msgs.iter().map(estimate_provider_message_bytes).sum();
        assert!(!cap_main_continuation_history("anthropic", &mut msgs));
        assert_eq!(msgs.len(), before_len);
        let after_bytes: usize = msgs.iter().map(estimate_provider_message_bytes).sum();
        assert_eq!(after_bytes, before_bytes);
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

        // `EngineState::handle_submit_text` and the StreamError handler both do
        // `state.cancel_token = CancellationToken::new();` after a cancel.
        let fresh = CancellationToken::new();
        assert!(!fresh.is_cancelled());

        // And cloning the fresh token doesn't observe the prior one's
        // cancelled state -- they're independent.
        let fresh_clone = fresh;
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
    use crate::app::EngineState;
    use std::sync::Arc;

    /// Tiny no-op provider — needed because `EngineState::new` requires a
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

    fn fresh_app_with(messages: Vec<ChatMessage>) -> EngineState {
        let mut state = EngineState::new(Arc::new(NoopProvider), "test-model");
        state.messages = messages;
        state
    }

    // Normal: a fresh sub-stream pushes a new empty assistant message
    // and returns its index. The previous trailing message stays
    // intact (the helper does NOT mutate or merge it).
    #[test]
    fn setup_pushes_empty_assistant_slot_and_returns_index_normal() {
        let mut state = fresh_app_with(vec![ChatMessage::user("hi".into())]);
        let idx = setup_new_substream_slot(&mut state, "test");
        assert_eq!(idx, 1, "assistant_idx must be the post-push index");
        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[idx].role, Role::Assistant);
        assert!(
            state.messages[idx]
                .parts
                .iter()
                .all(|p| matches!(p, MessagePart::Text(s) if s.is_empty())),
            "new assistant slot must be empty"
        );
        assert_eq!(state.streaming_assistant_idx, Some(idx));
        assert!(state.is_streaming);
    }

    // Normal: every reset field that the original two functions used
    // to set is set here too, so callers can drop their own copies.
    #[test]
    fn setup_resets_per_substream_state_normal() {
        let mut state = fresh_app_with(vec![ChatMessage::user("hi".into())]);
        // Pretend the previous sub-stream left state behind.
        state.streaming_text.push_str("leftover text");
        state.streaming_reasoning.push_str("leftover reasoning");
        state.last_usage_output = 999;
        state.usage_apply_baseline = (1, 2, 3, 4);
        state.thinking_started_at = Some(std::time::Instant::now());
        state.thinking_ended_at = Some(std::time::Instant::now());

        let _ = setup_new_substream_slot(&mut state, "test");

        assert!(
            state.streaming_text.is_empty(),
            "streaming_text must reset per sub-stream"
        );
        assert!(
            state.streaming_reasoning.is_empty(),
            "streaming_reasoning must reset per sub-stream"
        );
        assert_eq!(state.last_usage_output, 0);
        assert_eq!(state.usage_apply_baseline, (0, 0, 0, 0));
        assert!(state.thinking_started_at.is_none());
        assert!(state.thinking_ended_at.is_none());
        assert!(state.streaming_started_at.is_some());
        assert!(state.last_stream_event_at.is_some());
        assert!(state.streaming_last_token_at.is_some());
    }

    // Robust: streaming_response_bytes is the cumulative per-USER-TURN
    // counter and MUST survive a sub-stream restart. Regressing this
    // makes the spinner display "Brewed for 5s · ↓ 0k tokens" on
    // every agentic step instead of accumulating across the turn.
    #[test]
    fn setup_preserves_cumulative_response_bytes_robust() {
        let mut state = fresh_app_with(vec![ChatMessage::user("hi".into())]);
        state.streaming_response_bytes = 12_345;
        let _ = setup_new_substream_slot(&mut state, "test");
        assert_eq!(
            state.streaming_response_bytes, 12_345,
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
    use crate::app::EngineState;
    use crate::ids::ToolId;
    use crate::types::{
        ChatMessage, MessagePart, ToolCall, ToolDisplayState, ToolInput, ToolKind, ToolOutput,
        ToolStatus,
    };
    use jfc_provider::{ProviderContent, ProviderRole, ServerToolResultKind};
    use std::sync::Arc;

    /// Mirror of `setup_new_substream_slot_tests::NoopProvider` — the
    /// integration test reuses the same dummy provider because
    /// `EngineState::new` requires one even when no stream actually fires.
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
            thought_signature: None,
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
        let mut state = EngineState::new(Arc::new(NoopProvider), "test-model");
        state.messages = vec![
            ChatMessage::user("research rust".into()),
            ChatMessage::assistant_parts(vec![
                MessagePart::Text("Looking it up.".into()),
                MessagePart::tool(server_tool(
                    "srvtoolu_1",
                    ToolStatus::Running,
                    ToolOutput::Empty,
                )),
            ]),
        ];

        // setup_new_substream_slot is what the event_loop's
        // PauseTurn branch ultimately invokes (via
        // continue_after_pause_turn). After this call, state.messages
        // has a trailing empty assistant placeholder; the resend
        // builder must slice it off.
        let assistant_idx = setup_new_substream_slot(&mut state, "pause_turn_resume");
        assert_eq!(
            assistant_idx, 2,
            "fresh slot index must be the post-push position"
        );
        assert_eq!(
            state.messages.len(),
            3,
            "trailing empty assistant must be staged"
        );

        // Build the resend payload — this is what stream_response
        // will hand to the provider.
        let payload =
            build_provider_messages_for_pause_turn_resume(&state.messages[..assistant_idx]);

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

        let mut state = EngineState::new(Arc::new(NoopProvider), "test-model");
        state.messages = vec![
            ChatMessage::user("research rust".into()),
            ChatMessage::assistant_parts(vec![
                MessagePart::Text("Looking it up.".into()),
                MessagePart::tool(tool),
            ]),
        ];

        let assistant_idx = setup_new_substream_slot(&mut state, "pause_turn_resume");
        let payload =
            build_provider_messages_for_pause_turn_resume(&state.messages[..assistant_idx]);

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
        let mut state = EngineState::new(Arc::new(NoopProvider), "test-model");
        // user → A1 (text) → A2 (server_tool_use, pause_turn fires here)
        // The actual state.messages would still have both assistants
        // separated until session save coalesces them.
        state.messages = vec![
            ChatMessage::user("research rust then summarize".into()),
            ChatMessage::assistant("Sure, searching now.".into()),
            ChatMessage::assistant_parts(vec![MessagePart::tool(server_tool(
                "srvtoolu_3",
                ToolStatus::Running,
                ToolOutput::Empty,
            ))]),
        ];

        let assistant_idx = setup_new_substream_slot(&mut state, "pause_turn_resume");
        let payload =
            build_provider_messages_for_pause_turn_resume(&state.messages[..assistant_idx]);

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
        let mut state = EngineState::new(Arc::new(NoopProvider), "test-model");
        let local_tool = ToolCall {
            id: ToolId::from("toolu_local_1"),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
                run_in_background: None,
                suppress_output: None,
            },
            output: ToolOutput::Command {
                stdout: "file.txt\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        };
        state.messages = vec![
            ChatMessage::user("research rust and run ls".into()),
            ChatMessage::assistant_parts(vec![
                MessagePart::Text("Searching and listing.".into()),
                MessagePart::tool(server_tool(
                    "srvtoolu_mix",
                    ToolStatus::Running,
                    ToolOutput::Empty,
                )),
                MessagePart::tool(local_tool),
            ]),
        ];

        let assistant_idx = setup_new_substream_slot(&mut state, "pause_turn_mixed_resume");
        let payload =
            build_provider_messages_for_pause_turn_resume(&state.messages[..assistant_idx]);

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

    /// Normal: the `pending_pause_turn_resume` flag on `EngineState` defaults
    /// to false on a fresh EngineState. Pins the default so a later regression
    /// (e.g. flag flipped to true by accident in the constructor)
    /// doesn't silently re-route every turn through pause-turn-resume
    /// — which would break the "continue from where you left off"
    /// behavior on the normal agentic continuation path.
    #[test]
    fn fresh_app_has_pause_turn_resume_unlatched_normal() {
        let state = EngineState::new(Arc::new(NoopProvider), "test-model");
        assert!(
            !state.pending_pause_turn_resume,
            "pending_pause_turn_resume must default to false on a fresh EngineState"
        );
    }
}

#[cfg(test)]
mod stall_detection_tests {
    use super::*;

    fn assistant_text(s: &str) -> ChatMessage {
        ChatMessage::assistant(s.to_string())
    }

    // Verbatim stalls from the session corpus — all must be detected.
    #[test]
    fn detects_corpus_stall_phrases() {
        let stalls = [
            "Want me to start implementing any of these?",
            "Want me to take a closer look and propose an exact patch?",
            "Want me to fire another round of 30 agents covering the remaining 165 tasks?",
            "Does this plan look right? Any changes before I start coding?",
            "Want me to dive deeper into a specific section of either file?",
            "Shall I proceed with the refactor?",
            "Should I wire that in next?",
            "Let me know which direction you'd prefer.",
            "I can do that next if you want.",
        ];
        for s in stalls {
            assert!(
                assistant_text_stalls(&[assistant_text(s)]),
                "should detect stall: {s:?}"
            );
        }
    }

    // Genuine completions must NOT be flagged — otherwise we'd loop forever.
    #[test]
    fn ignores_genuine_completions() {
        let done = [
            "Done. Pushed `abc1234`.",
            "All 6 gaps shipped and the release binary is rebuilt.",
            "Fixed the bug and committed. Tests pass.",
            "The cache now serves 5.5x faster on warm reads.",
        ];
        for s in done {
            assert!(
                !assistant_text_stalls(&[assistant_text(s)]),
                "should NOT flag completion: {s:?}"
            );
        }
    }

    // A trailing question is a weak-but-real stall signal.
    #[test]
    fn detects_trailing_question() {
        assert!(assistant_text_stalls(&[assistant_text(
            "I found three candidates. Which one should we tackle first?"
        )]));
    }

    // A turn whose last assistant carries tool calls is handled by
    // should_continue_loop, not this guard.
    #[test]
    fn skips_tool_bearing_turns() {
        let msg = ChatMessage::assistant_parts(vec![MessagePart::tool(ToolCall {
            id: "toolu_x".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })]);
        assert!(!assistant_text_stalls(&[msg]));
    }

    #[test]
    fn empty_or_no_assistant_does_not_stall() {
        assert!(!assistant_text_stalls(&[]));
        assert!(!assistant_text_stalls(&[assistant_text("")]));
        assert!(!assistant_text_stalls(&[ChatMessage::user("hi".into())]));
    }

    // The stall phrase must be near the END — a "want me to" buried in the
    // middle of a long completion report shouldn't trip it.
    #[test]
    fn only_checks_the_tail() {
        let long = format!(
            "Earlier I asked want me to do X — you said yes, so I did it.\n\n{}",
            "Done. All tasks complete, committed, and pushed. ".repeat(8)
        );
        assert!(
            !assistant_text_stalls(&[assistant_text(&long)]),
            "a stall phrase far from the tail should not trip"
        );
    }

    // Regression: a multi-byte char (em-dash `—`, 3 bytes) straddling the
    // 240-byte tail window must not panic the byte slice. The crash had
    // start byte index inside '—'. Build a string so the cut lands mid-char.
    #[test]
    fn multibyte_char_on_tail_boundary_does_not_panic() {
        for pad in 230..250usize {
            let s = format!("{}— and that wraps it up.", "x".repeat(pad));
            // Must not panic.
            let _ = assistant_text_stalls(&[assistant_text(&s)]);
        }
    }
}
