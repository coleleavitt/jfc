use crate::{app, types};
use jfc_session::{DeletedFilter, TaskId, TaskStatus};

pub(crate) fn update_task_activities(app: &mut app::App, calls: &[types::ToolCall]) {
    let in_progress: Vec<TaskId> = app
        .task_store
        .list(DeletedFilter::Exclude)
        .iter()
        .filter(|task| matches!(task.status, TaskStatus::InProgress))
        .map(|task| task.id.clone())
        .collect();
    if in_progress.is_empty() {
        return;
    }

    let description = calls
        .iter()
        .map(|call| format!("{}: {}", call.kind.label(), call.input.summary()))
        .collect::<Vec<_>>()
        .join(", ");
    for task_id in in_progress {
        app.task_activities.insert(task_id, description.clone());
    }
}

/// Build a task-state-drift reminder for injection into the next turn, or
/// `None` when task state looks consistent.
///
/// The auto-update mechanisms only cover *delegated* work (`parent_task_id`
/// links a Task call to a todo so the runtime transitions it) and *timestamp*
/// bookkeeping. When the model works **directly** on the plan it must call
/// `TaskUpdate`/`TaskDone` itself — and the corpus shows it often forgets,
/// forcing the user to nudge "update the tasks".
///
/// Following SWE-agent's Agent-Computer-Interface principle (arXiv:2405.15793)
/// — surface state back to the agent rather than silently mutating it — this
/// returns a `<system-reminder>` body that nudges the model to reconcile task
/// state, preserving the model's ownership of task *semantics* (only it knows
/// whether a task is truly done). Two drift signals:
///
///   1. **Stale pending**: there are pending tasks but nothing in progress,
///      and the model did substantive work last turn (it should claim one).
///   2. **Stuck in-progress**: a task has been `InProgress` across many turns
///      with no `TaskDone` — likely finished-but-unmarked.
///
/// Drift only matters when the model actually did mutating work since the last
/// user turn — `did_substantive_work` scans the trailing messages for Edit /
/// Write / MultiEdit / ApplyPatch / Bash tool calls.
pub(crate) fn task_drift_reminder(app: &app::App) -> Option<String> {
    if !did_substantive_work(&app.messages) {
        return None;
    }
    let counts = app.task_store.counts();
    // No plan in play → nothing to reconcile.
    if counts.pending == 0 && counts.in_progress == 0 {
        return None;
    }

    // Signal 1: work happened, tasks are queued, but none is in progress —
    // the model is doing untracked work against a live plan.
    if counts.in_progress == 0 && counts.pending > 0 {
        return Some(format!(
            "You did work this turn but no task is marked in_progress, while {} task(s) \
             remain pending. If that work belongs to a task, mark it in_progress \
             (TaskUpdate) or complete (TaskDone). Keep the task list in sync as you go — \
             don't wait to be asked.",
            counts.pending
        ));
    }

    // Signal 2: something is in progress and work happened — remind to close
    // it out the moment it's done rather than drifting.
    if counts.in_progress > 0 {
        return Some(format!(
            "{} task(s) are in_progress. The moment a task's acceptance criteria are met, \
             call TaskDone immediately (don't batch). If you've moved on to other work, \
             reconcile the in_progress task first.",
            counts.in_progress
        ));
    }

    None
}

/// True if the trailing turn (messages after the last user message) contains a
/// mutating tool call — the signal that "real work" happened and task state
/// should reflect it.
fn did_substantive_work(messages: &[types::ChatMessage]) -> bool {
    use crate::types::{MessagePart, Role};
    use jfc_core::ToolKind;

    // Walk back to the last user message; everything after is the model's turn.
    let start = messages
        .iter()
        .rposition(|m| m.role == Role::User)
        .map(|i| i + 1)
        .unwrap_or(0);

    messages[start..].iter().any(|m| {
        m.parts.iter().any(|p| {
            matches!(
                p,
                MessagePart::Tool(tc) if matches!(
                    tc.kind,
                    ToolKind::Edit
                        | ToolKind::Write
                        | ToolKind::MultiEdit
                        | ToolKind::ApplyPatch
                        | ToolKind::Bash
                        | ToolKind::NotebookEdit
                )
            )
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        ChatMessage, MessagePart, ToolCall, ToolDisplayState, ToolOutput, ToolStatus,
    };
    use jfc_core::{ToolInput, ToolKind};

    fn tool_msg(kind: ToolKind) -> ChatMessage {
        ChatMessage::assistant_parts(vec![MessagePart::tool(ToolCall {
            id: "t".into(),
            kind,
            status: ToolStatus::Completed,
            input: ToolInput::Generic {
                summary: "x".into(),
            },
            output: ToolOutput::Empty,
            display: ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
            thought_signature: None,
        })])
    }

    #[test]
    fn detects_mutating_work_after_user_turn() {
        let msgs = vec![ChatMessage::user("do it".into()), tool_msg(ToolKind::Edit)];
        assert!(did_substantive_work(&msgs));
    }

    #[test]
    fn read_only_work_is_not_substantive() {
        let msgs = vec![
            ChatMessage::user("look".into()),
            tool_msg(ToolKind::Read),
            tool_msg(ToolKind::Grep),
        ];
        assert!(!did_substantive_work(&msgs));
    }

    #[test]
    fn only_counts_after_last_user_turn() {
        // An Edit BEFORE the last user message doesn't count.
        let msgs = vec![
            ChatMessage::user("first".into()),
            tool_msg(ToolKind::Edit),
            ChatMessage::user("second".into()),
            tool_msg(ToolKind::Read),
        ];
        assert!(!did_substantive_work(&msgs));
    }
}
