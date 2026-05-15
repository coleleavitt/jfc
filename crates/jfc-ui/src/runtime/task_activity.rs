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
