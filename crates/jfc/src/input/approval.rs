use std::sync::Arc;

use crossterm::event::{self, KeyCode};
use tokio::sync::mpsc;

use crate::app::{App, ApprovalChoice, EngineEvent, EngineState};
use crate::runtime::ToolEvent;
use crate::stream;
use crate::types::{MessagePart, ToolCall, ToolOutput, ToolStatus};

/// No-op: approvable tools are already inserted into the assistant message at
/// `StreamTool` time (see `main.rs` handler) so the user can see what's queued.
/// Kept as a stub for the call sites in the approval handlers; the real
/// status update happens via `ToolResult` when the dispatched tool finishes.
fn insert_tool_into_message(_state: &mut EngineState, _tool: &ToolCall) {
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

fn dispatch_approved_tools(state: &mut EngineState, tools: Vec<ToolCall>, tx: &mpsc::Sender<EngineEvent>) {
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
            teammate_event_tx: state.teammate_event_tx.clone(),
            local_advisor: stream::LocalAdvisorDispatchContext::from_state(state),
            cancel: state.cancel_token.clone(),
        },
    );
}

pub(super) fn deny_pending_and_queued(state: &mut EngineState, tx: &mpsc::Sender<EngineEvent>) -> usize {
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
        crate::runtime::send_critical(tx, EngineEvent::Tool(crate::runtime::ToolEvent::AllComplete));
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
fn advance_approval_queue(state: &mut EngineState) -> Vec<ToolCall> {
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
        state.pending_approval = Some(crate::app::PendingApproval {
            tool: next,
            selected: 0,
        });
        break;
    }
    auto_approved
}

fn finish_approval_decision(
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
        crate::runtime::send_critical(tx, EngineEvent::Tool(crate::runtime::ToolEvent::AllComplete));
    } else {
        dispatch_approved_tools(state, dispatchable, tx);
    }
}

/// Mark a previously-displayed (already in `messages`) tool as denied. We
/// look up the existing entry by `id` and mutate its status/output in place,
/// rather than appending a duplicate. The agentic loop's
/// `should_continue_loop` then sees a Failed entry and continues normally.
fn deny_tool(state: &mut EngineState, tool: ToolCall) {
    let id = tool.id.as_str().to_owned();
    // Audit: record the denial (the security trail).
    crate::changeset::record_approval(
        tool.kind.label(),
        false,
        state.current_session_id
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

pub(crate) fn handle_remote_approval_response(
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

pub(super) fn handle_approval_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<EngineEvent>,
) -> bool {
    let Some(ref mut approval) = app.engine.pending_approval else {
        return false;
    };

    if key.modifiers.contains(event::KeyModifiers::CONTROL)
        && matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C'))
    {
        return false;
    }

    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let tool = app.engine.pending_approval.take().unwrap().tool;
            insert_tool_into_message(&mut app.engine, &tool);
            finish_approval_decision(&mut app.engine, tx, vec![tool]);
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            let tool = app.engine.pending_approval.take().unwrap().tool;
            deny_tool(&mut app.engine, tool);
            finish_approval_decision(&mut app.engine, tx, Vec::new());
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            let name = approval.tool.kind.label().to_owned();
            app.engine.always_approved.push(name);
            let tool = app.engine.pending_approval.take().unwrap().tool;
            insert_tool_into_message(&mut app.engine, &tool);
            finish_approval_decision(&mut app.engine, tx, vec![tool]);
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            let name = approval.tool.kind.label().to_owned();
            app.engine.session_approved.push(name);
            let tool = app.engine.pending_approval.take().unwrap().tool;
            insert_tool_into_message(&mut app.engine, &tool);
            finish_approval_decision(&mut app.engine, tx, vec![tool]);
        }
        KeyCode::Up if approval.selected > 0 => {
            approval.selected -= 1;
        }
        KeyCode::Down => {
            approval.selected = (approval.selected + 1).min(ApprovalChoice::ALL.len() - 1);
        }
        KeyCode::Enter => {
            let choice = ApprovalChoice::ALL[approval.selected];
            let tool = app.engine.pending_approval.take().unwrap().tool;
            let mut dispatchable = Vec::new();
            match choice {
                ApprovalChoice::Yes | ApprovalChoice::YesSession => {
                    if choice == ApprovalChoice::YesSession {
                        let name = tool.kind.label().to_owned();
                        app.engine.session_approved.push(name);
                    }
                    insert_tool_into_message(&mut app.engine, &tool);
                    dispatchable.push(tool);
                }
                ApprovalChoice::Always => {
                    let name = tool.kind.label().to_owned();
                    app.engine.always_approved.push(name);
                    insert_tool_into_message(&mut app.engine, &tool);
                    dispatchable.push(tool);
                }
                ApprovalChoice::No => {
                    deny_tool(&mut app.engine, tool);
                }
            }
            finish_approval_decision(&mut app.engine, tx, dispatchable);
        }
        KeyCode::Esc => {
            // Esc cancels the entire batch — drop the queue too. Otherwise
            // a queued tool would surface immediately and the user would
            // have to dismiss them one-by-one.
            deny_pending_and_queued(&mut app.engine, tx);
        }
        KeyCode::Char('b') | KeyCode::Char('B')
            if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Tern) =>
        {
            let label = approval.tool.kind.label().to_owned();
            let tool = app.engine.pending_approval.take().unwrap().tool;
            insert_tool_into_message(&mut app.engine, &tool);
            let mut dispatchable = vec![tool];
            let mut drained = 1;
            let mut keep = std::collections::VecDeque::new();
            while let Some(next) = app.engine.approval_queue.pop_front() {
                if next.kind.label() == label {
                    insert_tool_into_message(&mut app.engine, &next);
                    dispatchable.push(next);
                    drained += 1;
                } else {
                    keep.push_back(next);
                }
            }
            app.engine.approval_queue = keep;
            if drained > 1 {
                crate::toast::push_with_cap(
                    &mut app.engine.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Info,
                        format!("Batch-approved {drained} `{label}` tools"),
                    ),
                );
            }
            finish_approval_decision(&mut app.engine, tx, dispatchable);
        }
        _ => {}
    }
    true
}
