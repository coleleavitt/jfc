//! The approval/question state machine — engine side. Tool approvals park
//! in `EngineState::pending_approval` / `approval_queue`; every frontend
//! resolves them through these functions (TUI modal keys, remote control,
//! headless permission prompts). Moved out of `input/` in stage 5 of the
//! jfc-engine extraction so the engine never depends on key handling.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::app::{EngineEvent, EngineState, PendingQuestion, QuestionItem, QuestionOption};
use crate::runtime::ToolEvent;
use crate::stream;
use crate::types::{MessagePart, ToolCall, ToolInput, ToolOutput, ToolStatus};

/// No-op: approvable tools are already inserted into the assistant message at
/// `StreamTool` time (see `main.rs` handler) so the user can see what's queued.
/// Kept as a stub for the call sites in the approval handlers; the real
/// status update happens via `ToolResult` when the dispatched tool finishes.
pub fn insert_tool_into_message(_state: &mut EngineState, _tool: &ToolCall) {
    // intentionally empty — the tool is already in `messages` from StreamTool.
}

fn send_set_in_progress(tx: &mpsc::Sender<EngineEvent>, action: &str, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    let _ = tx.try_send(EngineEvent::Tool(ToolEvent::SetInProgressToolUseIds {
        action: action.to_owned(),
        ids,
    }));
}

fn tool_ids(tools: &[ToolCall]) -> Vec<String> {
    tools
        .iter()
        .map(|tool| tool.id.as_str().to_owned())
        .collect()
}

pub fn dispatch_approved_tools(
    state: &mut EngineState,
    tools: Vec<ToolCall>,
    tx: &mpsc::Sender<EngineEvent>,
) {
    if tools.is_empty() {
        return;
    }
    let session_id = state
        .current_session_id
        .as_ref()
        .map(|id| id.as_str().to_owned());
    for tool in &tools {
        tracing::info!(
            target: "jfc::ui::approval",
            tool_kind = tool.kind.label(),
            tool_id = %tool.id,
            queue_remaining = state.approval_queue.len(),
            "approved → dispatch"
        );
        // Audit: record the approval grant (the security trail).
        crate::changeset::record_approval(tool.kind.label(), true, session_id.clone());
    }
    send_set_in_progress(tx, "add", tool_ids(&tools));
    state.in_flight_tool_batches += 1;
    stream::dispatch_tools_batched(
        tools,
        stream::ToolBatchDispatch {
            tx: tx.clone(),
            dedup: Arc::clone(&state.dedup_cache),
            task_store: Some(Arc::clone(&state.task_store)),
            active_team_name: state.team_context.team_name.clone(),
            current_session_id: state
                .current_session_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
            provider: Arc::clone(&state.provider),
            model: state.model.clone(),
            providers: state.providers.clone(),
            teammate_event_tx: state.teammate_event_tx.clone(),
            local_advisor: stream::LocalAdvisorDispatchContext::from_state(state),
            cancel: state.cancel_token.clone(),
        },
    );
}

pub fn deny_pending_and_queued(state: &mut EngineState, tx: &mpsc::Sender<EngineEvent>) -> usize {
    let mut denied = Vec::new();
    if let Some(pending) = state.pending_approval.take() {
        denied.push(pending.tool);
    }
    denied.extend(state.approval_queue.drain(..));
    let denied_count = denied.len();
    for tool in denied {
        deny_tool(state, tool);
    }
    if denied_count > 0 {
        crate::runtime::send_critical(
            tx,
            EngineEvent::Tool(crate::runtime::ToolEvent::AllComplete),
        );
    }
    denied_count
}

/// Promote the next queued tool into `pending_approval` so the modal cycles
/// through every tool the model emitted in this turn. Auto-applies prior
/// `always_approved` / `session_approved` decisions so the user doesn't get
/// re-prompted for tool kinds they already greenlit, and **dispatches
/// auto-approved tools immediately** via `dispatch_tools_batched`.
///
/// The earlier version pushed auto-approved tools onto `pending_tool_calls`
/// thinking the StreamDone handler would flush them — but `StreamDone(ToolUse)`
/// has already fired by the time the user is approving, so anything dropped
/// into `pending_tool_calls` here would sit there forever.
pub fn advance_approval_queue(state: &mut EngineState) -> Vec<ToolCall> {
    let mut auto_approved: Vec<ToolCall> = Vec::new();
    while let Some(next) = state.approval_queue.pop_front() {
        if !state.tool_needs_approval(&next) {
            tracing::info!(
                target: "jfc::ui::approval",
                tool_kind = next.kind.label(),
                tool_id = %next.id,
                queue_remaining = state.approval_queue.len(),
                "auto-approved → dispatch"
            );
            auto_approved.push(next);
            continue;
        }
        let session_id_for_hook = state
            .current_session_id
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("<no-session>");
        // Fire OnPermissionRequest hook so external scripts can observe
        // (or react to) a pending permission modal.
        crate::hooks::fire_async(
            crate::hooks::HookPoint::OnPermissionRequest,
            &crate::hooks::HookContext::for_tool(
                next.kind.label(),
                "",
                session_id_for_hook,
            )
            .with_extra("kind", "permission")
            .with_extra("tool_id", next.id.to_string()),
        );
        // Also fire the unified OnUserInputRequired hook — signals any
        // handler that the engine is about to block on user interaction.
        crate::hooks::fire_async(
            crate::hooks::HookPoint::OnUserInputRequired,
            &crate::hooks::HookContext::for_tool(
                next.kind.label(),
                "",
                session_id_for_hook,
            )
            .with_extra("kind", "permission")
            .with_extra("message", format!("Awaiting approval for {}", next.kind.label())),
        );
        state.pending_approval = Some(crate::app::PendingApproval {
            tool: next,
            selected: 0,
        });
        break;
    }
    auto_approved
}

pub fn finish_approval_decision(
    state: &mut EngineState,
    tx: &mpsc::Sender<EngineEvent>,
    mut dispatchable: Vec<ToolCall>,
) {
    dispatchable.extend(advance_approval_queue(state));
    if dispatchable.is_empty() && state.pending_approval.is_none() {
        // All tools have been processed (approved/denied) with no
        // dispatched batch. Signal AllComplete so the agentic loop
        // can re-invoke the model with the denial results. Without
        // this, a denial as the last tool leaves the loop stalled —
        // so use send_critical (never drop it on a full channel).
        crate::runtime::send_critical(
            tx,
            EngineEvent::Tool(crate::runtime::ToolEvent::AllComplete),
        );
    } else {
        dispatch_approved_tools(state, dispatchable, tx);
    }
}

/// Mark a previously-displayed (already in `messages`) tool as denied. We
/// look up the existing entry by `id` and mutate its status/output in place,
/// rather than appending a duplicate. The agentic loop's
/// `should_continue_loop` then sees a Failed entry and continues normally.
pub fn deny_tool(state: &mut EngineState, tool: ToolCall) {
    let id = tool.id.as_str().to_owned();
    // Audit: record the denial (the security trail).
    crate::changeset::record_approval(
        tool.kind.label(),
        false,
        state
            .current_session_id
            .as_ref()
            .map(|sid| sid.as_str().to_owned()),
    );
    state.set_in_progress_tool_use_ids("remove", std::slice::from_ref(&id));
    let mark_denied = |msg: &mut crate::types::ChatMessage| {
        for part in &mut msg.parts {
            if let MessagePart::Tool(tc) = part
                && tc.id == tool.id
            {
                tc.status = ToolStatus::Failed;
                tc.output = ToolOutput::Text("Denied by user".into());
                return true;
            }
        }
        false
    };
    if let Some(idx) = state.streaming_assistant_idx
        && let Some(msg) = state.messages.get_mut(idx)
        && mark_denied(msg)
    {
        return;
    }
    for msg in &mut state.messages {
        if mark_denied(msg) {
            return;
        }
    }
}

fn take_queued_approval(state: &mut EngineState, tool_use_id: &str) -> Option<ToolCall> {
    let pos = state
        .approval_queue
        .iter()
        .position(|tool| tool.id.as_str() == tool_use_id)?;
    state.approval_queue.remove(pos)
}

fn find_unresolved_tool_call(state: &EngineState, tool_use_id: &str) -> Option<ToolCall> {
    state.messages.iter().rev().find_map(|msg| {
        msg.parts.iter().find_map(|part| {
            let MessagePart::Tool(tool) = part else {
                return None;
            };
            if tool.id.as_str() == tool_use_id && !tool.status.is_terminal() {
                Some((**tool).clone())
            } else {
                None
            }
        })
    })
}

pub fn handle_remote_approval_response(
    state: &mut EngineState,
    tx: &mpsc::Sender<EngineEvent>,
    tool_use_id: String,
    approved: bool,
) {
    tracing::info!(
        target: "jfc::remote",
        tool_use_id = %tool_use_id,
        approved,
        "remote approval response"
    );

    if state
        .pending_approval
        .as_ref()
        .is_some_and(|pending| pending.tool.id.as_str() == tool_use_id)
    {
        let tool = state.pending_approval.take().expect("checked above").tool;
        if approved {
            insert_tool_into_message(state, &tool);
            finish_approval_decision(state, tx, vec![tool]);
        } else {
            deny_tool(state, tool);
            finish_approval_decision(state, tx, Vec::new());
        }
        return;
    }

    if let Some(tool) = take_queued_approval(state, &tool_use_id) {
        if approved {
            insert_tool_into_message(state, &tool);
            dispatch_approved_tools(state, vec![tool], tx);
        } else {
            deny_tool(state, tool);
            if state.pending_approval.is_none() && state.approval_queue.is_empty() {
                crate::runtime::send_critical(
                    tx,
                    EngineEvent::Tool(crate::runtime::ToolEvent::AllComplete),
                );
            }
        }
        return;
    }

    let Some(tool) = find_unresolved_tool_call(state, &tool_use_id) else {
        tracing::warn!(
            target: "jfc::remote",
            tool_use_id = %tool_use_id,
            approved,
            "dropping orphaned remote approval response: no unresolved tool_use found"
        );
        return;
    };

    tracing::warn!(
        target: "jfc::remote",
        tool_use_id = %tool_use_id,
        approved,
        "recovering orphaned remote approval response against unresolved transcript tool_use"
    );
    if approved {
        dispatch_approved_tools(state, vec![tool], tx);
    } else {
        deny_tool(state, tool);
        if state.pending_approval.is_none() && state.approval_queue.is_empty() {
            crate::runtime::send_critical(
                tx,
                EngineEvent::Tool(crate::runtime::ToolEvent::AllComplete),
            );
        }
    }
}

/// Build a [`PendingQuestion`] from an `AskUserQuestion` tool call. Returns
/// `None` when the input isn't an `AskUserQuestion` or has no usable questions
/// — the caller falls back to recording a failed tool_result so the tool_use
/// stays paired. The `questions` value is already normalized to an array by
/// the jfc-core parser (legacy single-question form lifted to 1 element).
pub fn build_pending_question(tool: &ToolCall) -> Option<PendingQuestion> {
    let ToolInput::AskUserQuestion { questions } = &tool.input else {
        return None;
    };
    let items: Vec<QuestionItem> = questions
        .as_array()
        .map(|arr| arr.iter().filter_map(parse_question).collect())
        .unwrap_or_default();
    if items.is_empty() {
        return None;
    }
    Some(PendingQuestion {
        tool_id: tool.id.clone(),
        items,
        current: 0,
        editing_other: false,
    })
}

/// Parse one question object into a [`QuestionItem`]. Returns `None` when it
/// has no usable options.
fn parse_question(q: &serde_json::Value) -> Option<QuestionItem> {
    let question = q
        .get("question")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let header = q
        .get("header")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let multi_select = q
        .get("multiSelect")
        .or_else(|| q.get("multi_select"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let options: Vec<QuestionOption> = q
        .get("options")
        .and_then(|v| v.as_array())
        .map(|arr| arr.iter().filter_map(parse_option).collect())
        .unwrap_or_default();
    if options.is_empty() {
        return None;
    }
    Some(QuestionItem {
        question,
        header,
        options,
        multi_select,
        selected: 0,
        chosen: std::collections::BTreeSet::new(),
        other_text: String::new(),
        answer: None,
    })
}

/// Parse an option object `{label, description?, preview?}`.
fn parse_option(o: &serde_json::Value) -> Option<QuestionOption> {
    let label = o.get("label").and_then(|v| v.as_str())?.to_owned();
    let description = o
        .get("description")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_owned();
    let preview = o.get("preview").and_then(|v| v.as_str()).map(str::to_owned);
    Some(QuestionOption {
        label,
        description,
        preview,
    })
}
