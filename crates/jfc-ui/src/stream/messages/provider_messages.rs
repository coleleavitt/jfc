use crate::types::{ChatMessage, MessagePart};
use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};

use super::attachments::push_attachments;
use super::tool_wire::{ToolWireCounters, tool_result_content, tool_use_content};
use super::turns::{
    chat_message_text, ensure_user_last, prepare_for_pause_turn_resume, provider_role,
    turns_ago_by_message, validate_provider_messages,
};

pub(crate) fn build_provider_messages(msgs: &[ChatMessage]) -> Vec<ProviderMessage> {
    let out: Vec<ProviderMessage> = msgs
        .iter()
        .filter_map(|m| {
            // Same guard as `build_provider_messages_with_tool_results`:
            // skip queued placeholders. See the longer rationale there.
            if m.queued {
                return None;
            }
            let text = chat_message_text(m);
            if text.is_empty() && m.attachments.is_empty() {
                return None;
            }
            let mut content: Vec<ProviderContent> = Vec::new();
            if !text.is_empty() {
                content.push(ProviderContent::Text(text));
            }
            push_attachments(&mut content, &m.attachments);
            Some(ProviderMessage {
                role: provider_role(m.role),
                content,
            })
        })
        .collect();
    tracing::debug!(
        target: "jfc::stream",
        input_messages = msgs.len(),
        output_messages = out.len(),
        "build_provider_messages (text-only)"
    );
    let out = ensure_user_last(out);
    validate_provider_messages(&out);
    out
}

/// Build provider messages for a `pause_turn` resume request.
///
/// Identical to [`build_provider_messages_with_tool_results`] EXCEPT that
/// it skips the synthetic-user-message injection that `ensure_user_last`
/// performs when the conversation ends on an assistant turn. Anthropic's
/// pause_turn resume protocol explicitly forbids appending a `"Continue."`
/// user message — the trailing assistant's `server_tool_use` block IS the
/// resume signal, and the server pairs it with its own
/// `web_search_tool_result` / equivalent server-side completion blocks
/// once the loop resumes.
///
/// See `StopReason::PauseTurn` docs and cli.js v142:622686, :623776.
pub(crate) fn build_provider_messages_for_pause_turn_resume(
    msgs: &[ChatMessage],
) -> Vec<ProviderMessage> {
    let out = build_assistant_and_tool_result_messages(msgs);
    let out = prepare_for_pause_turn_resume(out);
    validate_provider_messages(&out);
    out
}

pub(crate) fn build_provider_messages_with_tool_results(
    msgs: &[ChatMessage],
) -> Vec<ProviderMessage> {
    let out = build_assistant_and_tool_result_messages(msgs);
    let out = ensure_user_last(out);
    validate_provider_messages(&out);
    out
}

fn build_assistant_and_tool_result_messages(msgs: &[ChatMessage]) -> Vec<ProviderMessage> {
    let turns_ago_map = turns_ago_by_message(msgs);
    let mut out = Vec::new();
    let mut counters = ToolWireCounters::default();

    for (msg_idx, m) in msgs.iter().enumerate() {
        // Skip queued-prompt placeholders. They're real ChatMessages in
        // `app.messages` (so the user can see them rendered with the
        // pending/running glyph) but they MUST NOT be sent to the provider
        // until `drain_queued_prompts` promotes them.
        if m.queued {
            continue;
        }

        let role = provider_role(m.role);
        let text = chat_message_text(m);
        let tool_uses: Vec<ProviderContent> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool(tc) => Some(tool_use_content(tc, &mut counters)),
                _ => None,
            })
            .collect();

        let tool_results: Vec<ProviderContent> = m
            .parts
            .iter()
            .filter_map(|p| match p {
                MessagePart::Tool(tc) => Some(tool_result_content(
                    tc,
                    turns_ago_map[msg_idx],
                    &mut counters,
                )),
                _ => None,
            })
            .collect();

        let mut assistant_content = Vec::new();
        if !text.is_empty() {
            assistant_content.push(ProviderContent::Text(text.clone()));
        }
        assistant_content.extend(tool_uses);
        push_attachments(&mut assistant_content, &m.attachments);

        if !assistant_content.is_empty() {
            out.push(ProviderMessage {
                role: role.clone(),
                content: assistant_content,
            });
        } else if !text.is_empty() {
            out.push(ProviderMessage {
                role: role.clone(),
                content: vec![ProviderContent::Text(text)],
            });
        }

        if !tool_results.is_empty() {
            out.push(ProviderMessage {
                role: ProviderRole::User,
                content: tool_results,
            });
        }
    }

    tracing::debug!(
        target: "jfc::stream",
        input_messages = msgs.len(),
        output_messages = out.len(),
        tool_use_count = counters.tool_use_count,
        tool_result_count = counters.tool_result_count,
        abandoned_count = counters.abandoned_count,
        "build_assistant_and_tool_result_messages"
    );
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ToolId;
    use crate::types::{
        MessagePart, Role, ToolCall, ToolDisplayState, ToolInput, ToolKind, ToolOutput, ToolStatus,
    };

    fn user_msg(text: &str) -> ChatMessage {
        let mut m = ChatMessage::user(text.to_owned());
        m.parts = vec![MessagePart::Text(text.to_owned())];
        m
    }

    fn assistant_msg(text: &str) -> ChatMessage {
        let mut m = ChatMessage::assistant(text.to_owned());
        m.parts = vec![MessagePart::Text(text.to_owned())];
        m
    }

    fn assistant_with_parts(parts: Vec<MessagePart>) -> ChatMessage {
        ChatMessage::assistant_parts(parts)
    }

    fn make_tool_call(
        id: &str,
        kind: ToolKind,
        status: ToolStatus,
        output: ToolOutput,
    ) -> ToolCall {
        ToolCall {
            id: ToolId::from(id),
            kind,
            status,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
        }
    }

    // Normal: text-only conversation maps 1:1 to ProviderMessage::Text. The
    // ensure_user_last invariant kicks in if the conversation ended with the
    // assistant turn — we exercise that elsewhere.
    #[test]
    fn build_text_only_normal() {
        let msgs = vec![user_msg("hi"), assistant_msg("hello")];
        let out = build_provider_messages(&msgs);
        // Three messages: user, assistant, synthetic-user-trailer.
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, ProviderRole::User);
        assert_eq!(out[1].role, ProviderRole::Assistant);
        assert_eq!(out[2].role, ProviderRole::User);
    }

    // Normal: a message with multiple text parts joins them with newlines so
    // the model sees a single coherent block per turn.
    #[test]
    fn build_multi_text_part_joins_with_newlines_normal() {
        let m = ChatMessage {
            role: Role::User,
            parts: vec![
                MessagePart::Text("first".into()),
                MessagePart::Text("second".into()),
            ],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        };
        let out = build_provider_messages(&[m]);
        assert_eq!(out.len(), 1);
        match &out[0].content[0] {
            ProviderContent::Text(t) => assert_eq!(t, "first\nsecond"),
            _ => panic!("expected text content"),
        }
    }

    // Robust: empty / whitespace-only messages drop out entirely so the API
    // doesn't see a degenerate user turn (which Bedrock rejects with 400).
    #[test]
    fn build_drops_empty_text_messages_robust() {
        let m = ChatMessage {
            role: Role::User,
            parts: vec![MessagePart::Text(String::new())],
            agent_name: None,
            model_name: None,
            cost_tier: None,
            elapsed: None,
            usage: None,
            queued: false,
            attachments: Vec::new(),
        };
        let out = build_provider_messages(&[m]);
        // Empty input -> nothing emitted, ensure_user_last leaves the result
        // empty too because there's no trailing assistant to fix up.
        assert!(out.is_empty());
    }

    // Robust: empty input produces empty output (no synthetic injection on
    // a fully-empty conversation).
    #[test]
    fn build_empty_input_robust() {
        let out = build_provider_messages(&[]);
        assert!(out.is_empty());
    }

    // Normal: assistant turn with a completed tool produces a 2-message pair
    // — the assistant's tool_use, then the user's tool_result.
    #[test]
    fn build_with_tool_results_completed_pair_normal() {
        let tool = make_tool_call(
            "toolu_a",
            ToolKind::Bash,
            ToolStatus::Completed,
            ToolOutput::Text("hello world".into()),
        );
        let msgs = vec![
            user_msg("run ls"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // user, assistant(tool_use), user(tool_result)
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].role, ProviderRole::Assistant);
        match &out[1].content[0] {
            ProviderContent::ToolUse { id, .. } => assert_eq!(id, "toolu_a"),
            _ => panic!("expected ToolUse"),
        }
        assert_eq!(out[2].role, ProviderRole::User);
        match &out[2].content[0] {
            ProviderContent::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                assert_eq!(tool_use_id, "toolu_a");
                assert_eq!(content, "hello world");
                assert!(!is_error);
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: a Failed tool surfaces as is_error=true so the model can react
    // to the failure on its next turn.
    #[test]
    fn build_with_tool_results_failed_marks_is_error_normal() {
        let tool = make_tool_call(
            "toolu_b",
            ToolKind::Bash,
            ToolStatus::Failed,
            ToolOutput::Text("permission denied".into()),
        );
        let msgs = vec![
            user_msg("run rm -rf /"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let last = out.last().unwrap();
        match &last.content[0] {
            ProviderContent::ToolResult { is_error, .. } => {
                assert!(*is_error, "Failed tool must be flagged is_error");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Robust: a Pending / Running tool was abandoned (the user moved on
    // without approving). The builder synthesizes a stub error result so the
    // API sees a well-formed tool_result for every tool_use — Anthropic 400s
    // on orphaned tool_use blocks.
    #[test]
    fn build_with_tool_results_pending_synthesizes_abandoned_stub_robust() {
        let tool = make_tool_call(
            "toolu_orphan",
            ToolKind::Bash,
            ToolStatus::Pending,
            ToolOutput::Empty,
        );
        let msgs = vec![
            user_msg("hi"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let last = out.last().unwrap();
        match &last.content[0] {
            ProviderContent::ToolResult {
                content, is_error, ..
            } => {
                assert!(*is_error, "abandoned tool must be flagged is_error");
                assert!(
                    content.contains("abandoned"),
                    "abandoned-tool stub must mention abandonment, got: {content}"
                );
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: Command output formats to "exit/stdout/stderr" tri-line.
    #[test]
    fn build_with_tool_results_command_output_formats_normal() {
        let tool = make_tool_call(
            "toolu_c",
            ToolKind::Bash,
            ToolStatus::Completed,
            ToolOutput::Command {
                stdout: "ok\n".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
        );
        let msgs = vec![
            user_msg("run"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let last = out.last().unwrap();
        match &last.content[0] {
            ProviderContent::ToolResult { content, .. } => {
                assert!(content.contains("exit: 0"));
                assert!(content.contains("stdout: ok"));
                assert!(content.contains("stderr:"));
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: FileContent -> content string passes through untouched.
    #[test]
    fn build_with_tool_results_file_content_normal() {
        let tool = make_tool_call(
            "toolu_d",
            ToolKind::Read,
            ToolStatus::Completed,
            ToolOutput::FileContent {
                path: "/tmp/x.rs".into(),
                content: "fn main() {}".into(),
                language: "rust".into(),
            },
        );
        let msgs = vec![
            user_msg("read x.rs"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        match &out.last().unwrap().content[0] {
            ProviderContent::ToolResult { content, .. } => {
                assert_eq!(content, "fn main() {}");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: FileList output -> joined-with-newlines.
    #[test]
    fn build_with_tool_results_file_list_normal() {
        let tool = make_tool_call(
            "toolu_e",
            ToolKind::Glob,
            ToolStatus::Completed,
            ToolOutput::FileList(vec!["/a".into(), "/b".into(), "/c".into()]),
        );
        let msgs = vec![
            user_msg("glob"),
            assistant_with_parts(vec![MessagePart::Tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        match &out.last().unwrap().content[0] {
            ProviderContent::ToolResult { content, .. } => {
                assert_eq!(content, "/a\n/b\n/c");
            }
            _ => panic!("expected ToolResult"),
        }
    }

    // Normal: an assistant turn that has both prose AND a tool emits both
    // content blocks in order — text first, then tool_use. Anthropic relies
    // on this ordering to render the chain-of-thought.
    #[test]
    fn build_with_tool_results_text_and_tool_in_order_normal() {
        let tool = make_tool_call(
            "toolu_f",
            ToolKind::Bash,
            ToolStatus::Completed,
            ToolOutput::Text("ok".into()),
        );
        let msgs = vec![
            user_msg("hi"),
            assistant_with_parts(vec![
                MessagePart::Text("I'll run it.".into()),
                MessagePart::Tool(tool),
            ]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // out[1] is the assistant turn — content[0]=text, content[1]=tool_use.
        assert_eq!(out[1].content.len(), 2);
        assert!(matches!(out[1].content[0], ProviderContent::Text(_)));
        assert!(matches!(out[1].content[1], ProviderContent::ToolUse { .. }));
    }

    // Normal: pause_turn resume builder leaves the conversation ending on
    // the trailing assistant — NO synthetic `"Continue from where you left
    // off."` user message gets appended. Per Anthropic's pause_turn
    // protocol (cli.js v142:622686): "Do NOT add an extra user message
    // like 'Continue.' — the API detects the trailing server_tool_use
    // block and knows to resume automatically."
    #[test]
    fn build_for_pause_turn_resume_omits_synthetic_user_normal() {
        let msgs = vec![user_msg("search for X"), assistant_msg("searching…")];
        let out = build_provider_messages_for_pause_turn_resume(&msgs);
        assert_eq!(
            out.len(),
            2,
            "pause_turn resume must NOT append synthetic user — got {} msgs",
            out.len()
        );
        assert_eq!(out[0].role, ProviderRole::User);
        assert_eq!(out[1].role, ProviderRole::Assistant);
    }

    // Normal: the normal builder (non-resume) DOES inject the synthetic
    // user trailer. Pins that the resume path is an explicit
    // deviation, not the default.
    #[test]
    fn build_with_tool_results_appends_synthetic_user_normal() {
        let msgs = vec![user_msg("search for X"), assistant_msg("searching…")];
        let out = build_provider_messages_with_tool_results(&msgs);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2].role, ProviderRole::User);
        match &out[2].content[0] {
            ProviderContent::Text(t) => {
                assert!(
                    t.contains("Continue"),
                    "expected synthetic Continue user message, got: {t}"
                );
            }
            _ => panic!("expected Text continuation"),
        }
    }

    // Robust: pause_turn resume still strips trailing empty assistant
    // placeholders (those are `continue_*_loop` staging artifacts, not
    // model output, and Anthropic 400s on assistant prefill).
    #[test]
    fn build_for_pause_turn_resume_strips_empty_assistant_robust() {
        let msgs = vec![
            user_msg("search for X"),
            assistant_msg("searching…"),
            assistant_msg(""),
        ];
        let out = build_provider_messages_for_pause_turn_resume(&msgs);
        // The empty trailing assistant is stripped; the real assistant
        // remains at the end (no synthetic user appended).
        assert_eq!(out.len(), 2);
        assert_eq!(out.last().unwrap().role, ProviderRole::Assistant);
        match &out.last().unwrap().content[0] {
            ProviderContent::Text(t) => assert_eq!(t, "searching…"),
            _ => panic!("expected Text"),
        }
    }

    // Normal: pause_turn resume preserves the trailing assistant's
    // server-side tool_use block — that block IS the resume signal per
    // the spec. Wire shape: assistant content = [Text, ToolUse].
    #[test]
    fn build_for_pause_turn_resume_preserves_server_tool_use_normal() {
        let tool = make_tool_call(
            "srvtool_1",
            ToolKind::ServerWebSearch,
            ToolStatus::Completed,
            ToolOutput::Text("🔍 Executed server-side by Anthropic (q: rust)".into()),
        );
        let msgs = vec![
            user_msg("search rust"),
            assistant_with_parts(vec![
                MessagePart::Text("Looking it up.".into()),
                MessagePart::Tool(tool),
            ]),
        ];
        let out = build_provider_messages_for_pause_turn_resume(&msgs);
        // Two provider messages: user + assistant with text+tool_use.
        // The tool_result for the server tool stays attached in our
        // current wire shape (separate user msg after) — this test pins
        // that pause_turn resume does NOT inject the synthetic
        // "Continue" user, even when tool_results are present.
        let trailing_synthetic_continue = out.iter().any(|m| {
            m.role == ProviderRole::User
                && m.content.iter().any(|c| matches!(c, ProviderContent::Text(t) if t.contains("Continue from where you left off")))
        });
        assert!(
            !trailing_synthetic_continue,
            "pause_turn resume must not append the 'Continue from where you left off.' user filler"
        );
    }

    // Robust: multiple tools in one assistant turn produce one tool_result
    // per tool, all in the same trailing user message — Anthropic requires
    // batched tool_result blocks to share a single message.
    #[test]
    fn build_with_tool_results_batched_results_robust() {
        let t1 = make_tool_call(
            "a",
            ToolKind::Bash,
            ToolStatus::Completed,
            ToolOutput::Text("1".into()),
        );
        let t2 = make_tool_call(
            "b",
            ToolKind::Read,
            ToolStatus::Completed,
            ToolOutput::Text("2".into()),
        );
        let msgs = vec![
            user_msg("hi"),
            assistant_with_parts(vec![MessagePart::Tool(t1), MessagePart::Tool(t2)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // Tool-result message is the last (no synthetic-user trailer needed).
        let last = out.last().unwrap();
        assert_eq!(last.role, ProviderRole::User);
        assert_eq!(last.content.len(), 2);
    }
}
