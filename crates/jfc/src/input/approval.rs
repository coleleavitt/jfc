use crossterm::event::{self, KeyCode};
use tokio::sync::mpsc;

use crate::app::{App, ApprovalChoice, EngineEvent};
use crate::runtime::approvals::{
    deny_pending_and_queued, deny_tool, finish_approval_decision, insert_tool_into_message,
};

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
            if jfc_engine::feature_gates::is_enabled(
                jfc_engine::feature_gates::FeatureGate::Tern,
            ) =>
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
                jfc_engine::toast::push_with_cap(
                    &mut app.engine.toasts,
                    jfc_engine::toast::Toast::new(
                        jfc_engine::toast::ToastKind::Info,
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
