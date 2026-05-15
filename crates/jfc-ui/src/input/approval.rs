use std::sync::Arc;

use crossterm::event::{self, KeyCode};
use tokio::sync::mpsc;

use crate::app::{App, AppEvent, ApprovalChoice};
use crate::stream;
use crate::types::{MessagePart, ToolCall, ToolOutput, ToolStatus};

/// No-op: approvable tools are already inserted into the assistant message at
/// `StreamTool` time (see `main.rs` handler) so the user can see what's queued.
/// Kept as a stub for the call sites in the approval handlers; the real
/// status update happens via `ToolResult` when the dispatched tool finishes.
fn insert_tool_into_message(_app: &mut App, _tool: &ToolCall) {
    // intentionally empty — the tool is already in `messages` from StreamTool.
}

fn dispatch_approved_tool(app: &App, tool: ToolCall, tx: &mpsc::Sender<AppEvent>) {
    tracing::info!(
        target: "jfc::ui::approval",
        tool_kind = tool.kind.label(),
        tool_id = %tool.id,
        queue_remaining = app.approval_queue.len(),
        "approved → dispatch"
    );
    stream::dispatch_tools_batched(
        vec![tool],
        tx,
        Arc::clone(&app.dedup_cache),
        Some(Arc::clone(&app.task_store)),
        app.team_context.team_name.clone(),
        app.current_session_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        Arc::clone(&app.provider),
        app.model.clone(),
        app.teammate_event_tx.clone(),
        app.cancel_token.clone(),
    );
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
fn advance_approval_queue(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
    let mut auto_approved: Vec<ToolCall> = Vec::new();
    while let Some(next) = app.approval_queue.pop_front() {
        if !app.tool_needs_approval(&next) {
            tracing::info!(
                target: "jfc::ui::approval",
                tool_kind = next.kind.label(),
                tool_id = %next.id,
                queue_remaining = app.approval_queue.len(),
                "auto-approved → dispatch"
            );
            auto_approved.push(next);
            continue;
        }
        app.pending_approval = Some(crate::app::PendingApproval {
            tool: next,
            selected: 0,
        });
        break;
    }
    if !auto_approved.is_empty() {
        stream::dispatch_tools_batched(
            auto_approved,
            tx,
            Arc::clone(&app.dedup_cache),
            Some(Arc::clone(&app.task_store)),
            app.team_context.team_name.clone(),
            app.current_session_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
            Arc::clone(&app.provider),
            app.model.clone(),
            app.teammate_event_tx.clone(),
            app.cancel_token.clone(),
        );
    }
}

/// Mark a previously-displayed (already in `messages`) tool as denied. We
/// look up the existing entry by `id` and mutate its status/output in place,
/// rather than appending a duplicate. The agentic loop's
/// `should_continue_loop` then sees a Failed entry and continues normally.
fn deny_tool(app: &mut App, tool: ToolCall) {
    if let Some(idx) = app.streaming_assistant_idx {
        if let Some(msg) = app.messages.get_mut(idx) {
            for part in &mut msg.parts {
                if let MessagePart::Tool(tc) = part {
                    if tc.id == tool.id {
                        tc.status = ToolStatus::Failed;
                        tc.output = ToolOutput::Text("Denied by user".into());
                        return;
                    }
                }
            }
        }
    }
}

pub(super) fn handle_approval_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<AppEvent>,
) -> bool {
    let Some(ref mut approval) = app.pending_approval else {
        return false;
    };

    match key.code {
        KeyCode::Char('y') | KeyCode::Char('Y') => {
            let tool = app.pending_approval.take().unwrap().tool;
            insert_tool_into_message(app, &tool);
            dispatch_approved_tool(app, tool, tx);
            advance_approval_queue(app, tx);
        }
        KeyCode::Char('n') | KeyCode::Char('N') => {
            let tool = app.pending_approval.take().unwrap().tool;
            deny_tool(app, tool);
            advance_approval_queue(app, tx);
        }
        KeyCode::Char('a') | KeyCode::Char('A') => {
            let name = approval.tool.kind.label().to_owned();
            app.always_approved.push(name);
            let tool = app.pending_approval.take().unwrap().tool;
            insert_tool_into_message(app, &tool);
            dispatch_approved_tool(app, tool, tx);
            advance_approval_queue(app, tx);
        }
        KeyCode::Char('s') | KeyCode::Char('S') => {
            let name = approval.tool.kind.label().to_owned();
            app.session_approved.push(name);
            let tool = app.pending_approval.take().unwrap().tool;
            insert_tool_into_message(app, &tool);
            dispatch_approved_tool(app, tool, tx);
            advance_approval_queue(app, tx);
        }
        KeyCode::Up if approval.selected > 0 => {
            approval.selected -= 1;
        }
        KeyCode::Down => {
            approval.selected = (approval.selected + 1).min(ApprovalChoice::ALL.len() - 1);
        }
        KeyCode::Enter => {
            let choice = ApprovalChoice::ALL[approval.selected];
            let tool = app.pending_approval.take().unwrap().tool;
            match choice {
                ApprovalChoice::Yes | ApprovalChoice::YesSession => {
                    if choice == ApprovalChoice::YesSession {
                        let name = tool.kind.label().to_owned();
                        app.session_approved.push(name);
                    }
                    insert_tool_into_message(app, &tool);
                    dispatch_approved_tool(app, tool, tx);
                }
                ApprovalChoice::Always => {
                    let name = tool.kind.label().to_owned();
                    app.always_approved.push(name);
                    insert_tool_into_message(app, &tool);
                    dispatch_approved_tool(app, tool, tx);
                }
                ApprovalChoice::No => {
                    deny_tool(app, tool);
                }
            }
            advance_approval_queue(app, tx);
        }
        KeyCode::Esc => {
            // Esc cancels the entire batch — drop the queue too. Otherwise
            // a queued tool would surface immediately and the user would
            // have to dismiss them one-by-one.
            app.pending_approval = None;
            app.approval_queue.clear();
        }
        KeyCode::Char('b') | KeyCode::Char('B')
            if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Tern) =>
        {
            let label = approval.tool.kind.label().to_owned();
            let tool = app.pending_approval.take().unwrap().tool;
            insert_tool_into_message(app, &tool);
            dispatch_approved_tool(app, tool, tx);
            let mut drained = 1;
            let mut keep = std::collections::VecDeque::new();
            while let Some(next) = app.approval_queue.pop_front() {
                if next.kind.label() == label {
                    insert_tool_into_message(app, &next);
                    dispatch_approved_tool(app, next, tx);
                    drained += 1;
                } else {
                    keep.push_back(next);
                }
            }
            app.approval_queue = keep;
            if drained > 1 {
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Info,
                        format!("Batch-approved {drained} `{label}` tools"),
                    ),
                );
            }
            advance_approval_queue(app, tx);
        }
        _ => {}
    }
    true
}
