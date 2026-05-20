use crate::app::App;
use crate::runtime::{
    EventSender, StreamRequestOverrides, StreamToolChoice, restart_stream_in_place_with_overrides,
};
use crate::types::*;

pub(super) fn assistant_count_since_last_user(messages: &[ChatMessage]) -> usize {
    messages
        .iter()
        .rev()
        .take_while(|m| m.role != Role::User)
        .filter(|m| m.role == Role::Assistant)
        .count()
}

pub(super) fn has_tool_since_last_user(messages: &[ChatMessage]) -> bool {
    messages
        .iter()
        .rev()
        .take_while(|m| m.role != Role::User)
        .any(|m| {
            m.role == Role::Assistant && m.parts.iter().any(|p| matches!(p, MessagePart::Tool(_)))
        })
}

/// Byte threshold above which an assistant text-only response is
/// considered "narration" rather than a trivial ack ("ok", "done").
/// Architecturally a structural fence, not a heuristic about
/// phrasing: below this, the model genuinely had nothing to say and
/// is probably bowing out cleanly; above it, the model wrote prose
/// instead of acting and we want to nudge it back to tools.
pub(super) const NARRATION_BYTES_FLOOR: usize = 40;

pub(super) fn assistant_text_bytes_since_last_user(messages: &[ChatMessage]) -> usize {
    let mut bytes = 0usize;
    for msg in messages.iter().rev().take_while(|m| m.role != Role::User) {
        if msg.role != Role::Assistant {
            continue;
        }
        for part in &msg.parts {
            if let MessagePart::Text(text) = part {
                bytes = bytes.saturating_add(text.len());
            }
        }
    }
    bytes
}

/// Turn-termination contract: when the user requested an action and
/// tools were available, an `EndTurn` is only legal if the assistant
/// either emitted a tool_use or kept the response trivially short.
/// Returns `true` when the contract was violated and a retry is
/// warranted.
///
/// Structural fences (each one is an architectural invariant, not a
/// phrasing heuristic):
///   * `action_expected` — the user's prompt asked for work.
///   * `tool_choice == Auto` — the model chose, of its own accord, to
///     skip tool use; we did not constrain it.
///   * `advertised_tool_count > 0` — tools were on offer.
///   * `!narration_retry` — break the loop after one retry.
///   * first assistant turn since the user message — model hasn't
///     done anything else this turn.
///   * no tool_use emitted since the user message — the contract was
///     violated.
///   * substantial narration — the model actually wrote prose
///     (filters out trivial acks).
pub(super) fn should_retry_narration_only_end_turn(
    app: &App,
    stop_reason: &jfc_provider::StopReason,
) -> bool {
    if *stop_reason != jfc_provider::StopReason::EndTurn
        || app.pending_approval.is_some()
        || !app.approval_queue.is_empty()
        || !app.pending_tool_calls.is_empty()
    {
        return false;
    }

    let Some(meta) = app.current_stream_request.as_ref() else {
        return false;
    };
    if !(meta.advertised_tool_count > 0
        && meta.action_expected
        && meta.tool_choice == StreamToolChoice::Auto
        && !meta.narration_retry)
    {
        return false;
    }

    if assistant_count_since_last_user(&app.messages) != 1
        || has_tool_since_last_user(&app.messages)
    {
        return false;
    }

    assistant_text_bytes_since_last_user(&app.messages) >= NARRATION_BYTES_FLOOR
}

pub(super) fn retry_narration_only_end_turn(
    app: &mut App,
    tx: &EventSender,
    stop_reason: &jfc_provider::StopReason,
) -> bool {
    if !should_retry_narration_only_end_turn(app, stop_reason) {
        return false;
    }
    let Some(assistant_idx) = app.streaming_assistant_idx else {
        return false;
    };
    let Some(meta) = app.current_stream_request.clone() else {
        return false;
    };

    tracing::warn!(
        target: "jfc::stream::guard",
        advertised_tool_count = meta.advertised_tool_count,
        output_tokens = app.last_usage_output,
        assistant_text_bytes = assistant_text_bytes_since_last_user(&app.messages),
        "narration-only end_turn detected — retrying once with required tool use"
    );
    crate::toast::push_with_cap(
        &mut app.toasts,
        crate::toast::Toast::new(
            crate::toast::ToastKind::Warning,
            "Model narrated instead of using tools; retrying with tool use required.",
        ),
    );
    crate::system_reminder::append_to_last_user(
        &mut app.messages,
        "Your previous response described the work but did not call any tools. \
         Retry this turn now. Call at least one appropriate tool before giving \
         the user a prose answer.",
    );
    let turn_started_at = app.turn_started_at;
    restart_stream_in_place_with_overrides(
        app,
        tx,
        assistant_idx,
        turn_started_at,
        StreamRequestOverrides {
            tool_choice: StreamToolChoice::Any,
            narration_retry: true,
            background_reminders: Vec::new(),
        },
    );
    true
}

#[cfg(test)]
mod tests {
    //! Pins the structural contract enforced by
    //! [`should_retry_narration_only_end_turn`]. Each test isolates
    //! one fence — flipping it should disable the retry, regardless
    //! of the others. Together they describe the architectural rule:
    //!
    //!   "Action expected, tools advertised, model chose Auto, no
    //!    tool emitted, model wrote prose → retry."

    use super::*;
    use crate::runtime::{StreamRequestMetadata, StreamToolChoice};
    use crate::types::{
        ChatMessage, MessagePart, ToolCall, ToolInput, ToolKind, ToolOutput, ToolStatus,
    };
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use std::sync::Arc;

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
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl jfc_provider::seal::Sealed for TestProvider {}

    fn base_app(assistant_text: &str) -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.messages.push(ChatMessage::user("fix the bug".into()));
        app.messages
            .push(ChatMessage::assistant(assistant_text.into()));
        app.current_stream_request = Some(StreamRequestMetadata {
            advertised_tool_count: 4,
            action_expected: true,
            tool_choice: StreamToolChoice::Auto,
            narration_retry: false,
        });
        app
    }

    fn long_narration() -> String {
        // Long enough to clear NARRATION_BYTES_FLOOR (40 bytes).
        "Let me investigate the issue by reading the file first.".into()
    }

    /// Normal: action requested + tools available + Auto choice +
    /// substantial narration + no tool use → retry fires.
    #[test]
    fn fires_on_narration_only_normal() {
        let app = base_app(&long_narration());
        assert!(should_retry_narration_only_end_turn(
            &app,
            &jfc_provider::StopReason::EndTurn,
        ));
    }

    /// Robust: a non-EndTurn stop reason (ToolUse, MaxTokens, etc.)
    /// never triggers — that's a different lifecycle event handled
    /// elsewhere.
    #[test]
    fn skips_when_stop_reason_not_end_turn_robust() {
        let app = base_app(&long_narration());
        assert!(!should_retry_narration_only_end_turn(
            &app,
            &jfc_provider::StopReason::ToolUse,
        ));
    }

    /// Robust: a tiny ack ("ok") is below NARRATION_BYTES_FLOOR and
    /// must NOT retry — the model genuinely had nothing to say and
    /// nudging it would produce a noisy tool call.
    #[test]
    fn skips_when_assistant_text_below_floor_robust() {
        let app = base_app("ok");
        assert!(!should_retry_narration_only_end_turn(
            &app,
            &jfc_provider::StopReason::EndTurn,
        ));
    }

    /// Robust: when the model already emitted a tool this turn, the
    /// turn-termination contract is satisfied — no retry.
    #[test]
    fn skips_when_tool_already_emitted_robust() {
        let mut app = base_app(&long_narration());
        let assistant = app.messages.last_mut().unwrap();
        assistant.parts.push(MessagePart::Tool(ToolCall {
            id: crate::ids::ToolId::from("toolu_x"),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output: ToolOutput::Empty,
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }));
        assert!(!should_retry_narration_only_end_turn(
            &app,
            &jfc_provider::StopReason::EndTurn,
        ));
    }

    /// Robust: user prompt was informational ("explain X"), so
    /// `action_expected` is false in the live path. We simulate that
    /// here by flipping the bit — prose-only EndTurn is legitimate.
    #[test]
    fn skips_when_action_not_expected_robust() {
        let mut app = base_app(&long_narration());
        let meta = app.current_stream_request.as_mut().unwrap();
        meta.action_expected = false;
        assert!(!should_retry_narration_only_end_turn(
            &app,
            &jfc_provider::StopReason::EndTurn,
        ));
    }

    /// Robust: we already retried once. Looping forever would burn
    /// tokens on a model that's clearly not going to call a tool.
    #[test]
    fn skips_when_already_retried_robust() {
        let mut app = base_app(&long_narration());
        let meta = app.current_stream_request.as_mut().unwrap();
        meta.narration_retry = true;
        assert!(!should_retry_narration_only_end_turn(
            &app,
            &jfc_provider::StopReason::EndTurn,
        ));
    }

    /// Robust: provider advertised zero tools (sandboxed model, all
    /// tools denied by permissions, etc.). Forcing tool_choice: Any
    /// would 400 on every provider — never retry.
    #[test]
    fn skips_when_no_tools_advertised_robust() {
        let mut app = base_app(&long_narration());
        let meta = app.current_stream_request.as_mut().unwrap();
        meta.advertised_tool_count = 0;
        assert!(!should_retry_narration_only_end_turn(
            &app,
            &jfc_provider::StopReason::EndTurn,
        ));
    }

    /// Robust: tool_choice was already Any (e.g. the user explicitly
    /// forced it), so this is a deliberate decision by the caller —
    /// not a model misstep. No retry.
    #[test]
    fn skips_when_tool_choice_already_any_robust() {
        let mut app = base_app(&long_narration());
        let meta = app.current_stream_request.as_mut().unwrap();
        meta.tool_choice = StreamToolChoice::Any;
        assert!(!should_retry_narration_only_end_turn(
            &app,
            &jfc_provider::StopReason::EndTurn,
        ));
    }
}
