//! Message compaction helpers: coalescing consecutive same-role messages and
//! filtering out runtime-only placeholders before disk persistence.

use crate::types::{ChatMessage, MessagePart, Role};

/// Extract the first meaningful user prompt from messages for display in session list
pub(super) fn extract_first_prompt(messages: &[ChatMessage]) -> Option<String> {
    messages
        .iter()
        .find(|m| m.role == Role::User)
        .and_then(|m| {
            m.parts.iter().find_map(|p| match p {
                MessagePart::Text(t) if !t.trim().is_empty() => {
                    let trimmed = t.trim();
                    // Truncate long prompts for display (floor to char boundary)
                    if trimmed.len() > 100 {
                        let boundary = trimmed.floor_char_boundary(100);
                        Some(format!("{}…", &trimmed[..boundary]))
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                _ => None,
            })
        })
}

/// Merge consecutive same-role `ChatMessage`s into one logical turn
/// for persistence. Agentic loops push a fresh empty assistant slot
/// per sub-stream (see `setup_new_substream_slot`), so a 5-step
/// agentic turn ends up as `[user, A1, A2, A3, A4, A5, user]` on
/// disk. That:
///
///   * makes the file unreadable (one prompt → 5+ "assistant:" headers);
///   * makes resume rebuild the per-sub-stream split, with every
///     subsequent provider request `validate_turn_invariants`-warning
///     ConsecutiveAssistant in the log;
///   * confuses LLM-based summarizers that key off speaker alternation.
///
/// This helper does NOT touch the in-memory `app.messages` (sub-stream
/// boundaries are still needed at runtime for streaming-slot tracking
/// and the "this sub-stream completed at T" timestamps); it only runs
/// on the path **into** the session JSON.
///
/// Merging rules:
///   * adjacent same-role messages → one message with all parts
///     concatenated in order;
///   * `is_compact_boundary` messages stay on their own — they're a
///     semantic separator the renderer keys off;
///   * scalar fields (`agent_name`, `model_name`, `cost_tier`,
///     `elapsed`, `usage`) prefer the LAST non-None value — the most
///     recent sub-stream's metadata is the cumulative-correct one
///     (matches v126's per-message usage semantics: every assistant
///     message carries the END-of-turn cumulative count).
///   * `attachments` concatenate.
///   * `queued` messages bypass merging entirely (they're filtered
///     out before serialize anyway, but the dedup walk respects them
///     in case the filter ever moves).
pub(super) fn coalesce_consecutive_same_role(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut out: Vec<ChatMessage> = Vec::with_capacity(messages.len());
    for msg in messages {
        let can_merge = out.last().is_some_and(|prev| {
            prev.role == msg.role
                && !prev.is_compact_boundary()
                && !msg.is_compact_boundary()
                && !prev.queued
                && !msg.queued
        });
        if can_merge {
            let prev = out.last_mut().expect("can_merge guarantees a tail");
            // Extend parts in order — preserves the per-sub-stream
            // interleaving (text from sub-stream 1, tool from sub-stream
            // 1, text from sub-stream 2, tool from sub-stream 2, ...)
            // so the renderer can still walk through the conversation
            // chronologically.
            prev.parts.extend(msg.parts.iter().cloned());
            // Merge consecutive Text parts created by the extend so we don't
            // produce the 156-fragment-per-message bug seen in long sessions.
            crate::types::merge_consecutive_text_parts(&mut prev.parts);
            prev.attachments.extend(msg.attachments.iter().cloned());
            // Scalar fields: prefer the LAST non-None — the latest
            // sub-stream's view is the cumulative-correct one.
            if msg.agent_name.is_some() {
                prev.agent_name = msg.agent_name.clone();
            }
            if msg.model_name.is_some() {
                prev.model_name = msg.model_name.clone();
            }
            if msg.cost_tier.is_some() {
                prev.cost_tier = msg.cost_tier.clone();
            }
            if msg.elapsed.is_some() {
                prev.elapsed = msg.elapsed.clone();
            }
            if msg.usage.is_some() {
                prev.usage = msg.usage.clone();
            }
        } else {
            out.push(msg.clone());
        }
    }
    out
}

/// An assistant message is a discardable placeholder when it carries **no
/// renderable or sendable content** — every part is blank text and there are
/// no attachments. Metadata fields (`usage`, `model_name`, `cost_tier`,
/// `elapsed`, `agent_name`) are deliberately NOT part of this test: they are
/// pure bookkeeping, not content.
///
/// This matters for the `stop_reason=refusal` shape. When a model refuses and
/// emits zero content but the API still bills a `message_delta`, the runtime
/// stamps `usage` onto an otherwise-empty assistant slot (one `Text("")`
/// part). The earlier predicate required `usage.is_none()`, so this slot
/// survived into the persisted transcript — sandwiched between two user turns
/// (the original prompt's continuation reminders), it tripped
/// `validate_turn_invariants` (`EmptyMessage` / `ConsecutiveUser`) on every
/// subsequent save, spamming the session-save invariant warnings. Stripping it
/// regardless of metadata lets `coalesce_consecutive_same_role` merge the two
/// neighbouring user turns into one valid alternating turn.
pub(super) fn is_empty_assistant_placeholder(msg: &ChatMessage) -> bool {
    msg.role == Role::Assistant
        && !msg.queued
        && msg.attachments.is_empty()
        && msg
            .parts
            .iter()
            .all(|part| matches!(part, MessagePart::Text(text) if text.trim().is_empty()))
}

pub(super) fn persistent_session_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let filtered: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| !m.queued && !is_empty_assistant_placeholder(m))
        .cloned()
        .collect();
    coalesce_consecutive_same_role(&filtered)
}

pub(super) fn repair_loaded_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let stripped: Vec<ChatMessage> = messages
        .into_iter()
        .filter(|m| !is_empty_assistant_placeholder(m))
        .collect();
    let mut repaired = coalesce_consecutive_same_role(&stripped);
    for m in &mut repaired {
        crate::types::merge_consecutive_text_parts(&mut m.parts);
    }
    repaired
}

#[cfg(test)]
mod coalesce_tests {
    //! Pins the on-disk shape of agentic-loop transcripts. Sub-stream
    //! splits (one `ChatMessage::assistant("")` per sub-stream from
    //! `setup_new_substream_slot`) must collapse into a single
    //! assistant message on save so the file is human-readable and the
    //! resume path doesn't get 50+ assistant rows for a single user
    //! turn (the original `ses_20260515_175208.json` symptom).
    use super::coalesce_consecutive_same_role;
    use crate::ids::ToolId;
    use crate::types::{
        ChatMessage, MessagePart, ModelUsage, Role, ToolCall, ToolDisplayState, ToolInput,
        ToolKind, ToolOutput, ToolStatus, validate_turn_invariants,
    };

    fn user_text(s: &str) -> ChatMessage {
        ChatMessage::user(s.to_owned())
    }

    fn assistant_text(s: &str) -> ChatMessage {
        ChatMessage::assistant(s.to_owned())
    }

    fn tool_part(id: &str) -> MessagePart {
        MessagePart::Tool(ToolCall {
            id: ToolId::from(id),
            kind: ToolKind::Bash,
            status: ToolStatus::Completed,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output: ToolOutput::Text("ok".into()),
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })
    }

    // Normal: a 5-step agentic loop (user → A,A,A,A,A → user) collapses
    // to user → A → user. The persisted JSON shape matches the
    // alternating-role invariant `validate_turn_invariants` enforces.
    #[test]
    fn coalesces_five_assistant_substreams_to_one_normal() {
        let input = vec![
            user_text("do the thing"),
            ChatMessage::assistant_parts(vec![MessagePart::Text("step 1".into()), tool_part("t1")]),
            ChatMessage::assistant_parts(vec![MessagePart::Text("step 2".into()), tool_part("t2")]),
            ChatMessage::assistant_parts(vec![MessagePart::Text("step 3".into()), tool_part("t3")]),
            ChatMessage::assistant_parts(vec![MessagePart::Text("step 4".into()), tool_part("t4")]),
            assistant_text("done"),
            user_text("next prompt"),
        ];
        let out = coalesce_consecutive_same_role(&input);
        assert_eq!(out.len(), 3, "must collapse the 5 sub-streams into one");
        assert_eq!(out[0].role, Role::User);
        assert_eq!(out[1].role, Role::Assistant);
        assert_eq!(out[2].role, Role::User);
        // Parts preserved in order across all 5 sub-streams:
        // 4 (text+tool) + 1 (text) = 9.
        assert_eq!(out[1].parts.len(), 9);
        // Validate that the alternating-role invariant holds on the
        // coalesced output (this is what the on-disk file should
        // satisfy).
        validate_turn_invariants(&out).expect("coalesced session must satisfy invariants");
    }

    // Normal: an empty input produces an empty output (no synthetic
    // injection, no panic on the no-tail branch).
    #[test]
    fn coalesce_empty_input_normal() {
        let out = coalesce_consecutive_same_role(&[]);
        assert!(out.is_empty());
    }

    // Robust: an already-alternating transcript is a fixed point.
    // Coalescing twice produces the same shape.
    #[test]
    fn coalesce_already_alternating_is_fixed_point_robust() {
        let input = vec![
            user_text("a"),
            assistant_text("b"),
            user_text("c"),
            assistant_text("d"),
        ];
        let first_pass = coalesce_consecutive_same_role(&input);
        let second_pass = coalesce_consecutive_same_role(&first_pass);
        assert_eq!(first_pass.len(), 4);
        assert_eq!(first_pass.len(), second_pass.len());
        for (a, b) in first_pass.iter().zip(second_pass.iter()) {
            assert_eq!(a.role, b.role);
            assert_eq!(a.parts.len(), b.parts.len());
        }
    }

    // Robust: queued-prompt placeholders never participate in
    // merging. They're filtered out of save_session before coalesce
    // runs, but if a future caller hands them in directly the
    // dedup walk must respect them so user-typed text isn't
    // accidentally promoted into a sent prompt.
    #[test]
    fn coalesce_skips_queued_messages_robust() {
        let mut queued = user_text("queued");
        queued.queued = true;
        let input = vec![user_text("first"), queued, user_text("second")];
        let out = coalesce_consecutive_same_role(&input);
        // Queued is preserved as its own entry — never merged into a
        // sibling user message.
        assert_eq!(out.len(), 3);
        assert!(out[1].queued);
    }

    // Robust: usage from the LAST sub-stream wins on merge. v126
    // semantics: each assistant message carries the END-of-turn
    // cumulative usage, so the final sub-stream's usage IS the
    // post-merge correct value. If we picked the first or summed
    // them, the Context gauge would over- or under-count.
    #[test]
    fn coalesce_picks_last_usage_robust() {
        let mut first = ChatMessage::assistant("step 1".into());
        first.usage = Some(ModelUsage {
            input_tokens: 100,
            output_tokens: 10,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        });
        let mut last = ChatMessage::assistant("step 2".into());
        last.usage = Some(ModelUsage {
            input_tokens: 100,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        });
        let input = vec![user_text("hi"), first, last];
        let out = coalesce_consecutive_same_role(&input);
        assert_eq!(out.len(), 2);
        let usage = out[1].usage.as_ref().expect("usage must survive merge");
        assert_eq!(
            usage.output_tokens, 200,
            "merged usage must be the LAST sub-stream's value (cumulative end-of-turn count)"
        );
    }

    // Robust: a compact_boundary message stays on its own — it's a
    // semantic separator the renderer keys off, and merging it into a
    // sibling assistant would teach the model that the summary IS the
    // assistant's reply.
    #[test]
    fn coalesce_preserves_compact_boundary_robust() {
        let boundary =
            ChatMessage::assistant_parts(vec![MessagePart::CompactBoundary { pre_tokens: 100 }]);
        let input = vec![
            user_text("first"),
            assistant_text("step 1"),
            boundary,
            assistant_text("step 2"),
        ];
        let out = coalesce_consecutive_same_role(&input);
        // The boundary stays on its own message; "step 1" and
        // "step 2" do NOT merge across it because the boundary
        // breaks the same-role-merge chain.
        assert_eq!(out.len(), 4);
        assert!(
            out[2].is_compact_boundary(),
            "boundary must survive on its own message"
        );
    }
}

#[cfg(test)]
mod placeholder_tests {
    //! Regression tests for `is_empty_assistant_placeholder` and the
    //! `persistent_session_messages` save pipeline. These pin the fix for
    //! the `stop_reason=refusal` artefact reproduced in
    //! `ses_20260528_200646.json` (message index 77): a usage-stamped,
    //! content-empty assistant that broke `validate_turn_invariants`.
    use super::{is_empty_assistant_placeholder, persistent_session_messages};
    use crate::types::{ChatMessage, MessagePart, ModelUsage, Role, validate_turn_invariants};

    fn user_text(s: &str) -> ChatMessage {
        ChatMessage::user(s.to_owned())
    }

    fn usage_only_refusal() -> ChatMessage {
        // Mirrors the on-disk shape: parts=[Text("")], usage populated,
        // every other metadata field None.
        let mut msg = ChatMessage::assistant(String::new());
        msg.usage = Some(ModelUsage {
            input_tokens: 6,
            output_tokens: 2,
            cache_read_tokens: 22107,
            cache_write_tokens: 90833,
            cost_usd: None,
        });
        msg
    }

    // Normal: the refusal-shaped assistant is recognized as a placeholder
    // even though `usage` is populated. Pre-fix this returned false.
    #[test]
    fn refusal_shape_is_placeholder_normal() {
        let msg = usage_only_refusal();
        assert!(
            is_empty_assistant_placeholder(&msg),
            "usage-only empty assistant must be treated as a placeholder so it's stripped on save"
        );
    }

    // Normal: a non-empty assistant is never a placeholder, even if its
    // metadata happens to match.
    #[test]
    fn non_empty_assistant_is_not_placeholder_normal() {
        let mut msg = ChatMessage::assistant("hello".into());
        msg.usage = Some(ModelUsage {
            input_tokens: 1,
            output_tokens: 1,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        });
        assert!(!is_empty_assistant_placeholder(&msg));
    }

    // Robust: the exact ses_20260528_200646.json shape — user (76) →
    // refusal-shape assistant (77) → user (78) — survives the save
    // pipeline with the placeholder dropped, the two neighbouring user
    // turns coalesced into one, and `validate_turn_invariants` happy.
    #[test]
    fn refusal_artefact_survives_save_pipeline_robust() {
        let input = vec![
            user_text("first continuation reminder"),
            usage_only_refusal(),
            user_text("second continuation reminder"),
        ];
        let out = persistent_session_messages(&input);
        assert_eq!(
            out.len(),
            1,
            "the two user turns must coalesce after the placeholder is stripped"
        );
        assert_eq!(out[0].role, Role::User);
        // Both prompts survive merged into the single coalesced user turn.
        let text: String = out[0]
            .parts
            .iter()
            .map(|p| match p {
                MessagePart::Text(t) => t.as_str(),
                _ => "",
            })
            .collect();
        assert!(text.contains("first continuation reminder"));
        assert!(text.contains("second continuation reminder"));
        validate_turn_invariants(&out)
            .expect("post-save transcript must satisfy alternating-role invariants");
    }

    // Robust: a queued user message is never mistaken for a placeholder
    // even if its text is empty. The `!msg.queued` guard remains.
    #[test]
    fn queued_assistant_never_placeholder_robust() {
        let mut msg = ChatMessage::assistant(String::new());
        msg.queued = true;
        assert!(
            !is_empty_assistant_placeholder(&msg),
            "queued messages must never be filtered as placeholders"
        );
    }

    // Robust: an assistant with attachments is real content even if its
    // text parts are blank — must NOT be stripped.
    #[test]
    fn assistant_with_attachments_is_not_placeholder_robust() {
        use jfc_core::{Attachment, AttachmentKind};
        let mut msg = ChatMessage::assistant(String::new());
        msg.attachments = vec![Attachment {
            id: 1,
            kind: AttachmentKind::ImagePng,
            bytes: vec![0u8; 4],
        }];
        assert!(
            !is_empty_assistant_placeholder(&msg),
            "assistant with attachments carries content and must survive save"
        );
    }
}
