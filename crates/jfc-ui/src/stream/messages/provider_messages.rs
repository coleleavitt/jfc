use crate::types::{ChatMessage, MessagePart};
use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};

use super::attachments::push_attachments;
use super::tool_wire::{
    ToolWireCounters, is_server_tool, server_tool_result_content, tool_result_content,
    tool_use_content,
};
use super::turns::{
    chat_message_text, ensure_user_last, prepare_for_pause_turn_resume, provider_role,
    repair_tool_result_pairing, turns_ago_by_message, validate_provider_messages,
};

pub(crate) fn build_provider_messages(msgs: &[ChatMessage]) -> Vec<ProviderMessage> {
    build_provider_messages_with_tool_results(msgs)
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
    let out = repair_tool_result_pairing(out);
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

        // Bucket each tool into one of:
        //   (a) regular tool_use → emitted on assistant; paired
        //       tool_result emitted on the trailing user message;
        //   (b) server_tool_use → emitted on assistant; paired
        //       server_tool_result emitted on the SAME assistant
        //       message (no trailing user pair). Per cli.js v142:7057
        //       and :441375 — the server pairs server_tool_use with
        //       server_tool_result inside the same assistant turn.
        //
        // NOTE: Tool parts should only exist in assistant messages.
        // If one ends up in a user message (e.g. due to a race between
        // interruption and tool completion), skip it here — emitting a
        // tool_use block in a user message causes API 400.
        let mut assistant_tool_blocks: Vec<ProviderContent> = Vec::new();
        let mut user_tool_results: Vec<ProviderContent> = Vec::new();
        // Successfully-answered AskUserQuestion tools are replayed as plain
        // user-authored text turns instead of tool_use/tool_result pairs (see
        // the trust rewrite below). Collected here, emitted after the assistant
        // turn alongside `user_tool_results`.
        let mut user_text_rewrites: Vec<String> = Vec::new();
        if role == ProviderRole::Assistant {
            for part in &m.parts {
                let MessagePart::Tool(tc) = part else {
                    continue;
                };
                if is_server_tool(&tc.kind) {
                    assistant_tool_blocks.push(tool_use_content(tc, &mut counters));
                    // The paired result block (web_search_tool_result, etc.)
                    // is captured on the ToolCall's output by
                    // event_loop's StreamEvent::ServerToolResult handler.
                    // When it's present, re-emit it byte-faithfully here;
                    // when absent (e.g. the stream paused before the
                    // result arrived), the trailing server_tool_use block
                    // alone IS the resume cue per cli.js v142:622686, so
                    // we deliberately do not synthesize a placeholder.
                    if let Some(result) = server_tool_result_content(tc, &mut counters) {
                        assistant_tool_blocks.push(result);
                    }
                } else if matches!(tc.kind, crate::types::ToolKind::AskUserQuestion)
                    && tc.status == crate::types::ToolStatus::Completed
                {
                    // Trust rewrite: a successfully-answered AskUserQuestion is
                    // replayed as a user-authored turn
                    // `[User answered AskUserQuestion]: …` rather than a
                    // tool_use/tool_result pair. Dropping BOTH blocks together
                    // keeps tool pairing valid (no orphaned tool_use). This is
                    // the prompt-injection-safety carve-out — the answer is
                    // *direct user intent*, not untrusted tool output. Mirrors
                    // cli.js 2.1.160 :294912 (the rewrite) + :294659 (the trust
                    // exception) + the s15 tool_use skip. Declines (Failed) fall
                    // through to the normal tool_result path below.
                    user_text_rewrites.push(format!(
                        "[User answered AskUserQuestion]: {}",
                        tc.output.text_only()
                    ));
                } else {
                    assistant_tool_blocks.push(tool_use_content(tc, &mut counters));
                    user_tool_results.push(tool_result_content(
                        tc,
                        turns_ago_map[msg_idx],
                        &mut counters,
                    ));
                }
            }
        } else if m.parts.iter().any(|p| matches!(p, MessagePart::Tool(_))) {
            tracing::warn!(
                target: "jfc::stream",
                role = ?m.role,
                msg_idx,
                "orphaned tool part in non-assistant message — skipping to avoid API 400"
            );
        }

        let mut assistant_content = Vec::new();
        // Redacted thinking blocks must be round-tripped before text/tools.
        for part in &m.parts {
            if let MessagePart::RedactedThinking(data) = part {
                assistant_content.push(ProviderContent::RedactedThinking { data: data.clone() });
            }
        }
        if !text.is_empty() {
            assistant_content.push(ProviderContent::Text(text.clone()));
        }
        assistant_content.extend(assistant_tool_blocks);
        push_attachments(&mut assistant_content, &m.attachments);

        if !assistant_content.is_empty() {
            out.push(ProviderMessage {
                role,
                content: assistant_content,
            });
        } else if !text.is_empty() {
            out.push(ProviderMessage {
                role,
                content: vec![ProviderContent::Text(text)],
            });
        }

        if !user_tool_results.is_empty() {
            out.push(ProviderMessage {
                role: ProviderRole::User,
                content: user_tool_results,
            });
        }

        // Emit any AskUserQuestion answers as a user turn. Kept separate from
        // `user_tool_results` so the rewrite carries no tool_result block —
        // `merge_consecutive_same_role` folds it into an adjacent user turn.
        if !user_text_rewrites.is_empty() {
            out.push(ProviderMessage {
                role: ProviderRole::User,
                content: user_text_rewrites
                    .into_iter()
                    .map(ProviderContent::Text)
                    .collect(),
            });
        }
    }

    tracing::debug!(
        target: "jfc::stream",
        input_messages = msgs.len(),
        output_messages = out.len(),
        tool_use_count = counters.tool_use_count,
        tool_result_count = counters.tool_result_count,
        server_tool_use_count = counters.server_tool_use_count,
        server_tool_result_count = counters.server_tool_result_count,
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
            thought_signature: None,
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
            assistant_with_parts(vec![MessagePart::tool(tool)]),
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

    // Trust rewrite: a *completed* AskUserQuestion is replayed as a plain user
    // text turn `[User answered AskUserQuestion]: …` with NO tool_use/tool_result
    // pair, so the answer reads as direct user intent. Pairing stays valid
    // because both blocks are dropped together.
    #[test]
    fn build_answered_ask_user_question_rewrites_to_user_text_regression() {
        let tool = make_tool_call(
            "toolu_q",
            ToolKind::AskUserQuestion,
            ToolStatus::Completed,
            ToolOutput::Text("User has answered your question: \"X?\"=\"Yes\".".into()),
        );
        let msgs = vec![
            user_msg("which one?"),
            assistant_with_parts(vec![MessagePart::tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // No ToolUse / ToolResult block survives for the question.
        for m in &out {
            for c in &m.content {
                assert!(
                    !matches!(
                        c,
                        ProviderContent::ToolUse { .. } | ProviderContent::ToolResult { .. }
                    ),
                    "AskUserQuestion must not emit tool_use/tool_result blocks"
                );
            }
        }
        // A user turn carries the trust-marked answer.
        let has_marker = out.iter().any(|m| {
            m.role == ProviderRole::User
                && m.content.iter().any(|c| {
                    matches!(c, ProviderContent::Text(t) if t.starts_with("[User answered AskUserQuestion]:"))
                })
        });
        assert!(
            has_marker,
            "expected a [User answered AskUserQuestion] user turn"
        );
    }

    // Robust: a *declined* (Failed) AskUserQuestion is NOT rewritten — it keeps
    // the normal tool_use/tool_result pair (is_error=true) so the model sees the
    // decline, and pairing is preserved.
    #[test]
    fn build_declined_ask_user_question_keeps_tool_result_robust() {
        let tool = make_tool_call(
            "toolu_q2",
            ToolKind::AskUserQuestion,
            ToolStatus::Failed,
            ToolOutput::Text("User declined to answer the question.".into()),
        );
        let msgs = vec![
            user_msg("pick one"),
            assistant_with_parts(vec![MessagePart::tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let has_tool_use = out
            .iter()
            .flat_map(|m| &m.content)
            .any(|c| matches!(c, ProviderContent::ToolUse { id, .. } if id == "toolu_q2"));
        let has_tool_result = out.iter().flat_map(|m| &m.content).any(
            |c| matches!(c, ProviderContent::ToolResult { tool_use_id, is_error, .. } if tool_use_id == "toolu_q2" && *is_error),
        );
        assert!(has_tool_use, "declined question keeps its tool_use");
        assert!(
            has_tool_result,
            "declined question keeps its error tool_result"
        );
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
            assistant_with_parts(vec![MessagePart::tool(tool)]),
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
            assistant_with_parts(vec![MessagePart::tool(tool)]),
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
            assistant_with_parts(vec![MessagePart::tool(tool)]),
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
            assistant_with_parts(vec![MessagePart::tool(tool)]),
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
            assistant_with_parts(vec![MessagePart::tool(tool)]),
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
                MessagePart::tool(tool),
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
                MessagePart::tool(tool),
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
            assistant_with_parts(vec![MessagePart::tool(t1), MessagePart::tool(t2)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // Tool-result message is the last (no synthetic-user trailer needed).
        let last = out.last().unwrap();
        assert_eq!(last.role, ProviderRole::User);
        assert_eq!(last.content.len(), 2);
    }

    // ───── server_tool_use round-trip pins ────────────────────────────────
    // Per cli.js v142:7057 and :441375 a `server_tool_use` block emitted
    // on an assistant turn must round-trip as `type: "server_tool_use"`
    // (not plain `tool_use`) and must NOT spawn a paired user
    // `tool_result` message. The server pairs it with its own
    // `web_search_tool_result` block which is also written onto the
    // assistant turn.

    // Normal: a completed server_tool_use ToolCall round-trips as a
    // `ProviderContent::ServerToolUse` on the assistant message and
    // produces NO synthetic user `tool_result` block. (Note: this
    // tests the NON-pause-turn builder — which still appends the
    // standard "Continue from where you left off." trailer because
    // an assistant-tail server_tool_use isn't a pause_turn cue in
    // this path. The pause_turn builder's tests pin the resume
    // suppression separately.)
    #[test]
    fn server_tool_use_emits_server_tool_use_not_plain_tool_use_normal() {
        use jfc_provider::ServerToolResultKind;
        use serde_json::json;
        let tool = make_tool_call(
            "srvtoolu_1",
            ToolKind::ServerWebSearch,
            ToolStatus::Completed,
            ToolOutput::ServerToolResult {
                tool_kind: ServerToolResultKind::WebSearch,
                content: json!([]),
            },
        );
        let msgs = vec![
            user_msg("search rust"),
            assistant_with_parts(vec![MessagePart::tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        // The KEY assertion: NO fabricated `ProviderContent::ToolResult`
        // user block anywhere in the payload. That fabrication is what
        // used to break server-side sampling loop resumption.
        let has_tool_result = out.iter().any(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, ProviderContent::ToolResult { .. }))
        });
        assert!(
            !has_tool_result,
            "server tools must NOT round-trip as ProviderContent::ToolResult — that breaks the server-side sampling loop"
        );
        // And the assistant turn carries a `ProviderContent::ServerToolUse`
        // with the bare wire name `web_search` (NOT the JFC-internal
        // `server_tool_use:web_search` prefix). cli.js v142:441090.
        let assistant_msg = out
            .iter()
            .find(|m| m.role == ProviderRole::Assistant)
            .expect("expected an assistant message");
        let has_server_tool_use = assistant_msg.content.iter().any(
            |c| matches!(c, ProviderContent::ServerToolUse { name, .. } if name == "web_search"),
        );
        assert!(
            has_server_tool_use,
            "assistant message must carry a ProviderContent::ServerToolUse{{name='web_search'}}"
        );
    }

    // Normal: when the runtime captured a paired server_tool_result on
    // the ToolCall's output (via the StreamEvent::ServerToolResult
    // event in event_loop), the builder re-emits it byte-faithfully
    // on the SAME assistant message — NOT a separate user message.
    #[test]
    fn server_tool_use_carries_paired_result_on_same_assistant_msg_normal() {
        use jfc_provider::ServerToolResultKind;
        use serde_json::json;
        let mut tool = make_tool_call(
            "srvtoolu_2",
            ToolKind::ServerWebSearch,
            ToolStatus::Completed,
            ToolOutput::ServerToolResult {
                tool_kind: ServerToolResultKind::WebSearch,
                content: json!([
                    { "title": "Rust Programming Language",
                      "url": "https://www.rust-lang.org/" }
                ]),
            },
        );
        let _ = tool.mark_running();
        let _ = tool.mark_completed();
        let msgs = vec![
            user_msg("search rust"),
            assistant_with_parts(vec![MessagePart::tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let assistant = out
            .iter()
            .find(|m| m.role == ProviderRole::Assistant)
            .expect("expected an assistant message");
        // Two blocks: ServerToolUse first, then ServerToolResult — both
        // on the SAME assistant message. cli.js v142:441375 emits them
        // in this order on resend.
        let server_tool_use_count = assistant
            .content
            .iter()
            .filter(|c| matches!(c, ProviderContent::ServerToolUse { .. }))
            .count();
        let server_tool_result_count = assistant
            .content
            .iter()
            .filter(|c| matches!(c, ProviderContent::ServerToolResult { .. }))
            .count();
        assert_eq!(
            server_tool_use_count, 1,
            "assistant must carry exactly one server_tool_use block"
        );
        assert_eq!(
            server_tool_result_count, 1,
            "assistant must carry exactly one server_tool_result block (paired on the same turn)"
        );
        // The result block round-trips its raw JSON content
        // unchanged — Anthropic uses identity comparison on resend.
        let result = assistant
            .content
            .iter()
            .find_map(|c| match c {
                ProviderContent::ServerToolResult {
                    content, tool_kind, ..
                } => Some((content, tool_kind)),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            result.1.wire_type(),
            "web_search_tool_result",
            "kind must round-trip as web_search_tool_result"
        );
        let arr = result.0.as_array().expect("content must be an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["title"], "Rust Programming Language");
    }

    #[test]
    fn advisor_server_tool_round_trips_with_advisor_result_normal() {
        use jfc_provider::ServerToolResultKind;
        use serde_json::json;
        let tool = make_tool_call(
            "srvtoolu_advisor",
            ToolKind::ServerAdvisor,
            ToolStatus::Completed,
            ToolOutput::ServerToolResult {
                tool_kind: ServerToolResultKind::Advisor,
                content: json!({"type":"advisor_result","text":"check edge cases"}),
            },
        );
        let msgs = vec![
            user_msg("review this"),
            assistant_with_parts(vec![MessagePart::tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let assistant = out
            .iter()
            .find(|m| m.role == ProviderRole::Assistant)
            .expect("expected assistant message");
        assert!(assistant.content.iter().any(
            |c| matches!(c, ProviderContent::ServerToolUse { name, .. } if name == "advisor")
        ));
        assert!(assistant.content.iter().any(
            |c| matches!(c, ProviderContent::ServerToolResult { tool_kind, .. }
                if *tool_kind == ServerToolResultKind::Advisor)
        ));
    }

    // Robust: in the normal send path, an unpaired server_tool_use is
    // stripped by the upstream-style pairing repair. pause_turn resume has
    // its own builder that deliberately skips the repair because there the
    // trailing server_tool_use is the resume cue.
    #[test]
    fn normal_send_strips_server_tool_use_without_result_robust() {
        let tool = make_tool_call(
            "srvtoolu_3",
            ToolKind::ServerWebSearch,
            ToolStatus::Running, // result not arrived yet
            ToolOutput::Empty,
        );
        let msgs = vec![
            user_msg("search rust"),
            assistant_with_parts(vec![MessagePart::tool(tool)]),
        ];
        let out = build_provider_messages_with_tool_results(&msgs);
        let has_server_tool_use = out.iter().any(|m| {
            m.content
                .iter()
                .any(|c| matches!(c, ProviderContent::ServerToolUse { .. }))
        });
        assert!(
            !has_server_tool_use,
            "normal send must strip unpaired server_tool_use blocks"
        );
        assert!(!out.iter().any(|m| m.content.iter().any(|c| matches!(
            c,
            ProviderContent::ToolResult { .. } | ProviderContent::ServerToolResult { .. }
        ))));
    }
}
