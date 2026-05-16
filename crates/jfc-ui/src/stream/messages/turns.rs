use crate::types::{ChatMessage, MessagePart, Role};
use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole};

pub(super) fn provider_role(role: Role) -> ProviderRole {
    match role {
        Role::User => ProviderRole::User,
        Role::Assistant => ProviderRole::Assistant,
    }
}

pub(super) fn chat_message_text(m: &ChatMessage) -> String {
    m.parts
        .iter()
        .filter_map(|p| match p {
            MessagePart::Text(t) if !t.is_empty() => Some(t.to_owned()),
            // Serialize TaskStatus parts as inline text so the model sees
            // completed background agent summaries. Without this, detached
            // agents could finish and update the UI, but the model never knew.
            MessagePart::TaskStatus(ts) if ts.summary.is_some() || ts.error.is_some() => {
                let status_label = format!("{:?}", ts.status);
                let body = ts
                    .summary
                    .as_deref()
                    .or(ts.error.as_deref())
                    .unwrap_or("(no output)");
                // Cap at 2000 chars to prevent unbounded prompt growth —
                // TaskStatus summaries replay every turn and accumulate fast.
                let body = if body.len() > 2000 {
                    format!(
                        "{}… [truncated {} chars]",
                        &body[..body.floor_char_boundary(2000)],
                        body.len()
                    )
                } else {
                    body.to_string()
                };
                Some(format!(
                    "[Background agent: {} ({status_label})] {body}",
                    ts.description
                ))
            }
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub(super) fn turns_ago_by_message(msgs: &[ChatMessage]) -> Vec<usize> {
    let mut turns_ago_map: Vec<usize> = vec![0; msgs.len()];
    let mut user_turns_seen = 0usize;
    for i in (0..msgs.len()).rev() {
        if msgs[i].role == Role::User && !msgs[i].queued {
            user_turns_seen += 1;
        }
        turns_ago_map[i] = user_turns_seen;
    }
    turns_ago_map
}

/// Ensure the message list ends with a user-role message before sending to
/// the provider.
///
/// ## Why this is needed
///
/// Opus 4.6+ rejects any trailing assistant message with:
///     `"This model does not support assistant message prefill.
///      The conversation must end with a user message."`
///
/// Bedrock-via-LiteLLM returns the same error for any Anthropic model.
///
/// The native pre-4.6 API *silently* treats a trailing assistant as prefill,
/// which is also wrong for the agentic continuation use case (we want a
/// fresh assistant turn, not a continuation of an old one).
///
/// ## What we do
///
/// 1. Strip trailing assistant messages that are empty (only blank text).
/// 2. If the last message is still an assistant with real content (e.g. a
///    compact boundary summary, or a text-only end_turn that ended up last
///    due to filtering), **keep it but append a synthetic empty user turn**
///    so the API sees user-last ordering. This matches v126's behavior:
///    `normalizeMessagesForAPI` never produces a conversation ending in
///    assistant — tool_result blocks always follow tool_use blocks in a
///    trailing user message.
pub(super) fn ensure_user_last(msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    let msgs = strip_trailing_empty_assistants(msgs);
    let msgs = append_synthetic_user_if_trailing_assistant(msgs);
    merge_consecutive_same_role(msgs)
}

/// Resume-mode message preparation for `pause_turn` continuations.
///
/// Anthropic's `pause_turn` resume protocol (cli.js v142:622686, :623776):
///
/// > To continue, re-send the user message and assistant response and make
/// > another API request — the server will resume where it left off.
/// > **Do NOT add an extra user message like "Continue."** — the API detects
/// > the trailing `server_tool_use` block and knows to resume automatically.
///
/// We still strip empty trailing assistant placeholders (those are
/// `continue_agentic_loop` staging artifacts, not model output), and we
/// still merge consecutive same-role messages, but we deliberately skip
/// the synthetic-user injection that `ensure_user_last` does. The trailing
/// assistant with its `server_tool_use` block IS the resume signal.
///
/// Used only by `build_provider_messages_for_pause_turn_resume`.
pub(super) fn prepare_for_pause_turn_resume(msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    let msgs = strip_trailing_empty_assistants(msgs);
    // Note: NO synthetic-user step here. The trailing assistant is the
    // resume cue per Anthropic spec.
    merge_consecutive_same_role(msgs)
}

fn strip_trailing_empty_assistants(mut msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    while msgs
        .last()
        .map(|m| {
            m.role == ProviderRole::Assistant
                && m.content.iter().all(|c| match c {
                    ProviderContent::Text(s) => s.trim().is_empty(),
                    _ => false,
                })
        })
        .unwrap_or(false)
    {
        tracing::info!(
            target: "jfc::stream",
            "stripped trailing empty assistant before send"
        );
        msgs.pop();
    }
    msgs
}

fn append_synthetic_user_if_trailing_assistant(
    mut msgs: Vec<ProviderMessage>,
) -> Vec<ProviderMessage> {
    // If the conversation still ends with an assistant (real content),
    // append a minimal user turn. The Anthropic API requires alternating
    // user/assistant roles and user-last ordering.
    if msgs
        .last()
        .map(|m| m.role == ProviderRole::Assistant)
        .unwrap_or(false)
    {
        tracing::info!(
            target: "jfc::stream",
            "appending synthetic user turn to satisfy user-last ordering"
        );
        msgs.push(ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(
                "Continue from where you left off.".to_owned(),
            )],
        });
    }
    msgs
}

fn merge_consecutive_same_role(msgs: Vec<ProviderMessage>) -> Vec<ProviderMessage> {
    // Merge consecutive same-role messages. The Anthropic API requires
    // strictly alternating user/assistant turns. Consecutive same-role
    // messages happen when: (a) a compact_boundary (assistant) is followed
    // by a text-only assistant, (b) queued prompts produce adjacent user
    // messages, (c) filtering removes messages and collapses the alternation.
    //
    // CRITICAL: do NOT merge across a `tool_result` boundary. Anthropic's
    // Messages API validates that any user message containing a
    // `tool_result` block contains ONLY tool_result blocks (and that the
    // immediately-preceding assistant message contained matching
    // `tool_use` IDs). Merging a tool_result user message with a normal
    // text user message produces a mixed-content user turn that either
    // gets rejected with `invalid_request_error` or — worse on lenient
    // gateways — teaches the model that tool results are interchangeable
    // with chat text. Same rule for the inverse direction.
    let mut merged: Vec<ProviderMessage> = Vec::with_capacity(msgs.len());
    for msg in msgs {
        if let Some(last) = merged.last_mut() {
            if last.role == msg.role && !contains_tool_result(last) && !contains_tool_result(&msg) {
                last.content.extend(msg.content);
                continue;
            }
        }
        merged.push(msg);
    }
    merged
}

/// True iff the message carries any `ProviderContent::ToolResult` block.
/// Used by `ensure_user_last` to enforce Anthropic's tool_result purity
/// rule: a user message that contains tool_result MUST contain only
/// tool_result, and must not be merged with adjacent normal-text user
/// messages.
pub(super) fn contains_tool_result(msg: &ProviderMessage) -> bool {
    msg.content
        .iter()
        .any(|c| matches!(c, ProviderContent::ToolResult { .. }))
}

/// Debug-only validator for the post-merge provider message stream.
///
/// Anthropic's Messages API enforces several invariants that JFC's
/// `build_provider_messages*` mutations could violate:
///
/// 1. A user message containing `tool_result` MUST contain ONLY
///    tool_result blocks (no text, no images, no other types).
/// 2. A `tool_result` user message MUST be immediately preceded by an
///    assistant message that contains the matching `tool_use` IDs.
/// 3. Every `tool_use` ID in an assistant message MUST have a matching
///    `tool_result` in the next user message.
///
/// In debug builds we log loud warnings when any of these are violated
/// so the regression is visible in trace logs before the provider 400s
/// the request. In release builds this is a no-op. We don't BLOCK the
/// send — the gateway may be lenient, and a forensic log is more useful
/// than a hard panic.
#[cfg(debug_assertions)]
pub(super) fn validate_provider_messages(msgs: &[ProviderMessage]) {
    use std::collections::HashSet;
    for (i, msg) in msgs.iter().enumerate() {
        if matches!(msg.role, ProviderRole::User) && contains_tool_result(msg) {
            // Invariant 1: tool_result purity.
            let non_tool_result = msg
                .content
                .iter()
                .any(|c| !matches!(c, ProviderContent::ToolResult { .. }));
            if non_tool_result {
                tracing::warn!(
                    target: "jfc::stream::invariants",
                    msg_index = i,
                    content_kinds = ?msg.content.iter().map(|c| match c {
                        ProviderContent::Text(_) => "text",
                        ProviderContent::ToolUse { .. } => "tool_use",
                        ProviderContent::ToolResult { .. } => "tool_result",
                        ProviderContent::Attachment(_) => "attachment",
                    }).collect::<Vec<_>>(),
                    "provider message invariant violation: user message contains tool_result mixed with other content"
                );
            }
            // Invariant 2: must follow an assistant with matching tool_use IDs.
            let tool_result_ids: HashSet<&str> = msg
                .content
                .iter()
                .filter_map(|c| match c {
                    ProviderContent::ToolResult { tool_use_id, .. } => Some(tool_use_id.as_str()),
                    _ => None,
                })
                .collect();
            if let Some(prev) = i.checked_sub(1).and_then(|j| msgs.get(j)) {
                if !matches!(prev.role, ProviderRole::Assistant) {
                    tracing::warn!(
                        target: "jfc::stream::invariants",
                        msg_index = i,
                        prev_role = ?prev.role,
                        "provider message invariant violation: tool_result user message not preceded by assistant"
                    );
                } else {
                    let tool_use_ids: HashSet<&str> = prev
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            ProviderContent::ToolUse { id, .. } => Some(id.as_str()),
                            _ => None,
                        })
                        .collect();
                    let missing: Vec<&str> =
                        tool_result_ids.difference(&tool_use_ids).copied().collect();
                    let unmatched: Vec<&str> =
                        tool_use_ids.difference(&tool_result_ids).copied().collect();
                    if !missing.is_empty() || !unmatched.is_empty() {
                        tracing::warn!(
                            target: "jfc::stream::invariants",
                            msg_index = i,
                            tool_result_without_use = ?missing,
                            tool_use_without_result = ?unmatched,
                            "provider message invariant violation: tool_use ↔ tool_result IDs do not match"
                        );
                    }
                }
            } else {
                tracing::warn!(
                    target: "jfc::stream::invariants",
                    msg_index = i,
                    "provider message invariant violation: leading tool_result user message"
                );
            }
        }
    }
}

#[cfg(not(debug_assertions))]
pub(super) fn validate_provider_messages(_msgs: &[ProviderMessage]) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn user_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }

    fn assistant_text(s: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(s.to_owned())],
        }
    }

    // Normal: the exact bug from the screenshot — `continue_agentic_loop`
    // pushes an empty assistant placeholder, the builder echoes it, Bedrock
    // explodes. After the strip, the conversation ends on the user turn.
    #[test]
    fn strip_drops_trailing_empty_assistant_normal() {
        let input = vec![user_text("hi"), assistant_text("")];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ProviderRole::User);
    }

    // Normal: whitespace-only text counts as empty — a streamed turn that
    // only emitted a newline before being interrupted is still no content.
    #[test]
    fn strip_drops_trailing_whitespace_only_assistant_normal() {
        let input = vec![user_text("hi"), assistant_text("   \n")];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ProviderRole::User);
    }

    // Normal: real assistant text at the end gets a synthetic user turn
    // appended so the API sees user-last ordering. Opus 4.6 rejects trailing
    // assistant even with content.
    #[test]
    fn appends_user_when_assistant_has_real_content() {
        let input = vec![user_text("hi"), assistant_text("hello")];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 3);
        assert_eq!(out[1].role, ProviderRole::Assistant);
        assert_eq!(out[2].role, ProviderRole::User);
    }

    // Normal: an assistant turn with a tool_use gets a synthetic user turn
    // appended (the tool_result would normally follow, but ensure_user_last
    // acts as a safety net).
    #[test]
    fn appends_user_when_assistant_has_only_toolcall() {
        let assistant_with_tool = ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: "toolu_1".to_owned(),
                name: "Bash".to_owned(),
                input: serde_json::json!({"command": "ls"}),
            }],
        };
        let input = vec![user_text("hi"), assistant_with_tool];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 3);
        assert_eq!(out[2].role, ProviderRole::User);
    }

    // Normal: if the conversation already ends with a user message (the
    // common tool_result-injection case), the function is a no-op.
    #[test]
    fn no_op_on_user_last_normal() {
        let input = vec![assistant_text("hi"), user_text("ok")];
        let out = ensure_user_last(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].role, ProviderRole::User);
    }

    // Robust: empty input must round-trip — no panic.
    #[test]
    fn no_op_on_empty_input_robust() {
        let out = ensure_user_last(Vec::<ProviderMessage>::new());
        assert!(out.is_empty());
    }

    // Normal: multiple trailing empty assistants are ALL stripped.
    #[test]
    fn strips_multiple_trailing_empty_assistants() {
        let input = vec![user_text("hi"), assistant_text(""), assistant_text("")];
        let out = ensure_user_last(input);
        // Both empties stripped, "hi" remains. User-last already satisfied.
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].role, ProviderRole::User);
    }

    // Normal: pause_turn resume DOES strip trailing empty assistants
    // (placeholder leak from `continue_after_pause_turn`'s staging) but
    // does NOT inject the `"Continue from where you left off."` user.
    // Per Anthropic spec (cli.js v142:622686), the trailing assistant's
    // `server_tool_use` block is itself the resume cue.
    #[test]
    fn pause_turn_resume_keeps_trailing_assistant_normal() {
        let input = vec![user_text("hi"), assistant_text("partial reply")];
        let out = prepare_for_pause_turn_resume(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out.last().unwrap().role, ProviderRole::Assistant);
    }

    // Robust: pause_turn resume still strips empty trailing assistants
    // (placeholder slots staged by `continue_after_pause_turn`) — only
    // the synthetic-user injection is skipped.
    #[test]
    fn pause_turn_resume_strips_empty_assistant_robust() {
        let input = vec![
            user_text("hi"),
            assistant_text("partial"),
            assistant_text(""),
        ];
        let out = prepare_for_pause_turn_resume(input);
        assert_eq!(out.len(), 2);
        assert_eq!(out.last().unwrap().role, ProviderRole::Assistant);
        if let ProviderContent::Text(t) = &out.last().unwrap().content[0] {
            assert_eq!(t, "partial");
        } else {
            panic!("expected Text");
        }
    }

    // Robust: pause_turn resume must NEVER inject the "Continue from
    // where you left off." filler — the Anthropic API detects that as
    // a real user turn and breaks the server-side loop resumption.
    #[test]
    fn pause_turn_resume_never_injects_continue_filler_robust() {
        let input = vec![user_text("search"), assistant_text("looking up rust docs")];
        let out = prepare_for_pause_turn_resume(input);
        for msg in &out {
            if msg.role == ProviderRole::User {
                for c in &msg.content {
                    if let ProviderContent::Text(t) = c {
                        assert!(
                            !t.contains("Continue from where you left off"),
                            "pause_turn resume injected forbidden filler: {t}"
                        );
                    }
                }
            }
        }
    }

    // Normal: consecutive same-role messages get merged.
    #[test]
    fn merges_consecutive_user_messages() {
        let input = vec![user_text("a"), user_text("b"), assistant_text("c")];
        let out = ensure_user_last(input);
        // Two users merged into one, then assistant, then synthetic user
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].role, ProviderRole::User);
        assert_eq!(out[0].content.len(), 2); // merged
        assert_eq!(out[1].role, ProviderRole::Assistant);
        assert_eq!(out[2].role, ProviderRole::User); // synthetic
    }
}
