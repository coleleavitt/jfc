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
            // Merging two consecutive USER turns is suspicious: it means an
            // assistant turn between them was filtered out (empty-but-billed
            // placeholder), and the two user prompts will be concatenated into
            // one — the save-side face of the prompt-doubling report. Flag it
            // so the log shows when history got structurally rewritten.
            if msg.role == crate::types::Role::User {
                tracing::warn!(
                    target: "jfc::session",
                    "coalesce merging two consecutive USER turns — an assistant turn was \
                     stripped between them; their text will be concatenated"
                );
            }
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
        && !msg.parts.iter().any(part_has_meaningful_content)
}

/// Whether a part carries content worth keeping. Mirrors the `EmptyMessage`
/// branch of `validate_turn_invariants_inner` so the repair pass strips
/// *exactly* what the invariant flags. The earlier version only recognised
/// non-empty `Text`, so an assistant turn whose sole part was an empty
/// `Reasoning("")` (or `Advisor("")`) survived repair, leaving the
/// transcript permanently invalid — every save re-logged "empty assistant
/// message at index N" and load reported "still violates after repair".
fn part_has_meaningful_content(part: &MessagePart) -> bool {
    match part {
        MessagePart::Text(s) | MessagePart::Reasoning(s) | MessagePart::Advisor(s) => {
            !s.trim().is_empty()
        }
        MessagePart::RedactedThinking(_)
        | MessagePart::Tool(_)
        | MessagePart::TaskStatus(_)
        | MessagePart::CompactBoundary { .. } => true,
    }
}

pub(super) fn persistent_session_messages(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let mut filtered: Vec<ChatMessage> = messages
        .iter()
        .filter(|m| !m.queued && !is_empty_assistant_placeholder(m))
        .cloned()
        .collect();
    terminalize_stranded_tools(&mut filtered, TerminalizeTail::Preserve);
    coalesce_consecutive_same_role(&filtered)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalizeTail {
    Preserve,
    Include,
}

/// Coerce any non-terminal (Pending / Running / Idle) tool that is stranded
/// *before the final message* into a terminal `Failed` state with an
/// abandoned-stub output. Tools in the **last** message are left untouched —
/// those are legitimately in-flight at save time (the live streaming tail).
///
/// ## Why
///
/// A tool whose terminal status was lost (a dropped `ToolEvent`, an
/// interrupt mid-batch, the abandoned-tool race) persisted to disk as
/// `pending` *mid-history*, with completed turns after it (observed in
/// `ses_20260528_184520` msg 53, `ses_20260528_143541` msg 49). On reload
/// the provider-message builder already synthesizes an "abandoned" stub for
/// such tools (`stream/messages/provider_messages.rs`), so it doesn't 400 —
/// but the on-disk transcript stays misleading and the renderer shows a
/// frozen spinner glyph forever. Terminalizing at save time makes the
/// persisted history honest and matches what the wire builder does anyway.
fn terminalize_stranded_tools(messages: &mut [ChatMessage], tail: TerminalizeTail) {
    let len = messages.len();
    if len == 0 {
        return;
    }
    if len < 2 && tail == TerminalizeTail::Preserve {
        return; // only message is the (possibly in-flight) tail — leave it.
    }
    let last_idx = len - 1;
    for (idx, msg) in messages.iter_mut().enumerate() {
        if tail == TerminalizeTail::Preserve && idx == last_idx {
            continue; // live streaming tail — genuine in-flight tools.
        }
        for part in &mut msg.parts {
            if let MessagePart::Tool(tc) = part
                && !tc.status.is_terminal()
            {
                tracing::debug!(
                    target: "jfc::session",
                    tool = %tc.kind.label(),
                    prior_status = tc.status.label(),
                    msg_idx = idx,
                    "terminalizing stranded non-terminal tool (lost completion signal)"
                );
                tc.status = crate::types::ToolStatus::Failed;
                if matches!(tc.output, crate::types::ToolOutput::Empty) {
                    tc.output = crate::types::ToolOutput::Text(
                        "Tool did not report completion before the session was saved \
                         (status reset to failed on persist)."
                            .to_owned(),
                    );
                }
            }
        }
    }
}

/// Strip orphan tool parts that ended up on a `User` message.
///
/// In jfc's transcript model `MessagePart::Tool` belongs only on assistant
/// turns; a tool part on a user message is malformed and trips
/// `validate_turn_invariants`' `OrphanToolResult` check, which the load path
/// then logs as "still violates after repair". The wire builder already drops
/// these (`stream/messages/provider_messages.rs`: "orphaned tool part in
/// non-assistant message — skipping to avoid API 400"), so the repair path
/// mirrors that: remove the offending parts, then drop any user message left
/// with no content. Runs before `coalesce_consecutive_same_role` so a dropped
/// message lets its neighbours merge into one valid alternating turn.
fn strip_orphan_user_tool_parts(messages: &mut Vec<ChatMessage>) {
    for m in messages.iter_mut() {
        if m.role == Role::User {
            m.parts.retain(|p| !matches!(p, MessagePart::Tool(_)));
        }
    }
    // An empty user message is itself an invariant violation, so drop it.
    messages.retain(|m| m.role != Role::User || !m.parts.is_empty());
}

pub(super) fn repair_loaded_messages(messages: Vec<ChatMessage>) -> Vec<ChatMessage> {
    let mut stripped: Vec<ChatMessage> = messages
        .into_iter()
        .filter(|m| !is_empty_assistant_placeholder(m))
        .collect();
    strip_orphan_user_tool_parts(&mut stripped);
    let mut repaired = coalesce_consecutive_same_role(&stripped);
    terminalize_stranded_tools(&mut repaired, TerminalizeTail::Include);
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
        MessagePart::tool(ToolCall {
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

    // Robust: a USER turn mixing text + a stray tool part keeps the text but
    // loses the tool part (the `OrphanToolResult` shape the load path logged
    // as "still violates after repair"). The repaired transcript is valid.
    #[test]
    fn repair_strips_orphan_tool_part_keeps_user_text_robust() {
        let mut bad_user = ChatMessage::user("please continue".into());
        bad_user.parts.push(tool_part("toolu_orphan"));

        let out = super::repair_loaded_messages(vec![
            user_text("start"),
            assistant_text("ok"),
            bad_user,
            assistant_text("done"),
        ]);

        let user_tool_parts = out
            .iter()
            .filter(|m| m.role == Role::User)
            .flat_map(|m| &m.parts)
            .filter(|p| matches!(p, MessagePart::Tool(_)))
            .count();
        assert_eq!(user_tool_parts, 0, "user tool part must be stripped");
        validate_turn_invariants(&out).expect("repaired transcript must be valid");
    }

    // Robust: a USER turn that is *only* a tool part has no content left after
    // stripping, so the whole turn is dropped and its assistant neighbours
    // coalesce into one valid alternating turn.
    #[test]
    fn repair_drops_user_message_that_is_only_a_tool_part_robust() {
        let mut only_tool = ChatMessage::user(String::new());
        only_tool.parts = vec![tool_part("toolu_only")];

        let out = super::repair_loaded_messages(vec![
            user_text("start"),
            assistant_text("more"),
            only_tool,
            assistant_text("done"),
        ]);

        let user_count = out.iter().filter(|m| m.role == Role::User).count();
        assert_eq!(user_count, 1, "tool-only user turn must be dropped");
        validate_turn_invariants(&out).expect("repaired transcript must be valid");
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

    // Robust: an assistant turn whose only part is an empty `Reasoning("")`
    // (or `Advisor("")`) is empty per `validate_turn_invariants` but the old
    // Text-only placeholder check missed it, so it survived repair and the
    // transcript stayed permanently invalid (the recurring "empty assistant
    // message at index N · still violates after repair" in the logs). It must
    // now be recognized as a placeholder and stripped.
    #[test]
    fn empty_reasoning_only_assistant_is_placeholder_robust() {
        let mut msg = ChatMessage::assistant(String::new());
        msg.parts = vec![MessagePart::Reasoning(String::new())];
        assert!(
            is_empty_assistant_placeholder(&msg),
            "an empty-reasoning-only assistant must be stripped to satisfy the invariant"
        );

        // A *non-empty* reasoning-only turn is real content — keep it.
        let mut thinking = ChatMessage::assistant(String::new());
        thinking.parts = vec![MessagePart::Reasoning("let me think…".into())];
        assert!(!is_empty_assistant_placeholder(&thinking));

        // Mixed empties (empty text + empty reasoning) → still a placeholder.
        let mut mixed = ChatMessage::assistant(String::new());
        mixed.parts = vec![
            MessagePart::Text(String::new()),
            MessagePart::Reasoning("   ".into()),
        ];
        assert!(is_empty_assistant_placeholder(&mixed));
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

#[cfg(test)]
mod terminalize_tests {
    use super::{persistent_session_messages, repair_loaded_messages};
    use crate::types::{
        ChatMessage, MessagePart, ToolCall, ToolDisplayState, ToolInput, ToolKind, ToolOutput,
        ToolStatus,
    };

    fn tool(id: &str, status: ToolStatus, output: ToolOutput) -> MessagePart {
        MessagePart::tool(ToolCall {
            id: crate::ids::ToolId::from(id),
            kind: ToolKind::Bash,
            status,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })
    }

    fn tool_status(msg: &ChatMessage) -> ToolStatus {
        msg.parts
            .iter()
            .find_map(|p| match p {
                MessagePart::Tool(tc) => Some(tc.status),
                _ => None,
            })
            .expect("a tool part")
    }

    // Normal — REGRESSION: a tool left Pending *mid-history* (a completed turn
    // follows it) is coerced to Failed on persist, with an abandoned-stub
    // output so the on-disk transcript is honest. Mirrors ses_20260528_184520
    // msg 53 / ses_20260528_143541 msg 49.
    #[test]
    fn stranded_pending_tool_is_terminalized_normal_regression() {
        let input = vec![
            ChatMessage::user("do it".into()),
            ChatMessage::assistant_parts(vec![tool("t1", ToolStatus::Pending, ToolOutput::Empty)]),
            ChatMessage::user("next".into()),
            ChatMessage::assistant("done".into()), // tail — not the stranded tool
        ];
        let out = persistent_session_messages(&input);
        // The mid-history assistant (now out[1]) had its Pending tool coerced.
        let coerced = out
            .iter()
            .find(|m| m.parts.iter().any(|p| matches!(p, MessagePart::Tool(_))))
            .expect("tool message survives");
        assert_eq!(
            tool_status(coerced),
            ToolStatus::Failed,
            "a stranded Pending tool must terminalize to Failed on save"
        );
        // Empty output replaced with the abandoned-stub text.
        let has_stub = coerced.parts.iter().any(|p| matches!(
            p,
            MessagePart::Tool(tc) if matches!(&tc.output, ToolOutput::Text(t) if t.contains("did not report completion"))
        ));
        assert!(has_stub, "abandoned stub output must be written");
    }

    // Robust: a tool that's Running in the LAST message (the live streaming
    // tail) is left untouched — it's legitimately in-flight at save time.
    #[test]
    fn in_flight_tail_tool_is_left_untouched_robust() {
        let input = vec![
            ChatMessage::user("do it".into()),
            ChatMessage::assistant_parts(vec![tool("t1", ToolStatus::Running, ToolOutput::Empty)]),
        ];
        let out = persistent_session_messages(&input);
        let tail = out.last().expect("tail survives");
        assert_eq!(
            tool_status(tail),
            ToolStatus::Running,
            "an in-flight tool in the tail message must keep its live status"
        );
    }

    // Robust: an already-Completed mid-history tool is not disturbed (its
    // real output and status survive verbatim).
    #[test]
    fn completed_tool_is_preserved_robust() {
        let input = vec![
            ChatMessage::user("do it".into()),
            ChatMessage::assistant_parts(vec![tool(
                "t1",
                ToolStatus::Completed,
                ToolOutput::Text("real output".into()),
            )]),
            ChatMessage::user("next".into()),
            ChatMessage::assistant("done".into()),
        ];
        let out = persistent_session_messages(&input);
        let coerced = out
            .iter()
            .find(|m| m.parts.iter().any(|p| matches!(p, MessagePart::Tool(_))))
            .expect("tool message survives");
        assert_eq!(tool_status(coerced), ToolStatus::Completed);
        let preserved = coerced.parts.iter().any(|p| matches!(
            p,
            MessagePart::Tool(tc) if matches!(&tc.output, ToolOutput::Text(t) if t == "real output")
        ));
        assert!(preserved, "completed tool output must survive verbatim");
    }

    #[test]
    fn load_repair_terminalizes_final_pending_tool_robust() {
        let input = vec![
            ChatMessage::user("do it".into()),
            ChatMessage::assistant_parts(vec![tool("t1", ToolStatus::Pending, ToolOutput::Empty)]),
        ];
        let out = repair_loaded_messages(input);
        let tail = out.last().expect("tail survives");
        assert_eq!(
            tool_status(tail),
            ToolStatus::Failed,
            "a loaded transcript has no live in-flight tail, so pending tools must be failed"
        );
    }
}
