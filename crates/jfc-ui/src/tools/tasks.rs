use std::sync::Arc;

use tracing::{debug, info, warn};

use super::ExecutionResult;
use super::subagent::execute_skill_in;
use crate::tasks::{DeletedFilter, TaskPatch, TaskStatus, TaskStore};

pub(super) fn execute_task_create(
    store: Option<Arc<TaskStore>>,
    subject: String,
    description: String,
    active_form: Option<String>,
    blocked_by: Vec<String>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", %subject, blocked_count = blocked_by.len(), "task_create: creating");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    match store.create(subject, description, active_form, blocked_by) {
        Ok(task) => {
            debug!(target: "jfc::tools", task_id = %task.id, "task_create: success");
            ExecutionResult::success(
                serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            )
        }
        Err(e) => {
            warn!(target: "jfc::tools", error = %e, "task_create: failed");
            ExecutionResult::failure(e.to_string())
        }
    }
}

pub(super) fn execute_task_update(
    store: Option<Arc<TaskStore>>,
    task_id: &str,
    status: Option<String>,
    subject: Option<String>,
    description: Option<String>,
    owner: Option<String>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", task_id, status = status.as_deref(), "task_update: updating");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let parsed_status = status.as_deref().and_then(|s| match s {
        "pending" => Some(TaskStatus::Pending),
        "in_progress" => Some(TaskStatus::InProgress),
        "completed" => Some(TaskStatus::Completed),
        "deleted" => Some(TaskStatus::Deleted),
        _ => None,
    });
    let patch = TaskPatch {
        subject,
        description,
        status: parsed_status,
        owner,
        ..Default::default()
    };
    match store.update(task_id, patch) {
        Ok(task) => {
            debug!(target: "jfc::tools", task_id, "task_update: success");
            ExecutionResult::success(
                serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            )
        }
        Err(e) => {
            warn!(target: "jfc::tools", task_id, error = %e, "task_update: failed");
            ExecutionResult::failure(e.to_string())
        }
    }
}

pub(super) fn execute_task_list(
    store: Option<Arc<TaskStore>>,
    status_filter: Option<&str>,
    owner_filter: Option<&str>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", status_filter, owner_filter, "task_list: listing");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let mut tasks = store.list(DeletedFilter::Exclude);
    if let Some(sf) = status_filter {
        tasks.retain(|t| {
            let s = serde_json::to_value(&t.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned));
            s.as_deref() == Some(sf)
        });
    }
    if let Some(of) = owner_filter {
        tasks.retain(|t| t.owner.as_deref() == Some(of));
    }
    debug!(target: "jfc::tools", count = tasks.len(), "task_list: result");
    let output =
        serde_json::to_string_pretty(&tasks).unwrap_or_else(|_| format!("{} tasks", tasks.len()));
    ExecutionResult::success(output)
}

pub(super) fn execute_task_done(store: Option<Arc<TaskStore>>, task_id: &str) -> ExecutionResult {
    debug!(target: "jfc::tools", task_id, "task_done: marking complete");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let patch = TaskPatch {
        status: Some(TaskStatus::Completed),
        ..Default::default()
    };
    match store.update(task_id, patch) {
        Ok(task) => {
            debug!(target: "jfc::tools", task_id, "task_done: success");
            ExecutionResult::success(
                serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            )
        }
        Err(e) => {
            warn!(target: "jfc::tools", task_id, error = %e, "task_done: failed");
            ExecutionResult::failure(e.to_string())
        }
    }
}

pub(super) fn execute_task_get(store: Option<Arc<TaskStore>>, task_id: &str) -> ExecutionResult {
    debug!(target: "jfc::tools", task_id, "task_get: retrieving");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let tasks = store.list(DeletedFilter::Exclude);
    match tasks.into_iter().find(|t| t.id == task_id) {
        Some(task) => {
            debug!(target: "jfc::tools", task_id, "task_get: found");
            ExecutionResult::success(
                serde_json::to_string_pretty(&task).unwrap_or_else(|_| format!("{task:?}")),
            )
        }
        None => {
            debug!(target: "jfc::tools", task_id, "task_get: not found");
            ExecutionResult::failure(format!("No task found with id '{task_id}'"))
        }
    }
}

/// Resolve a registered skill by name and return its markdown body as the
/// tool result. Optional `args` (when non-empty) are appended under an
/// `# Args` header so the model can incorporate the caller's context.
///
/// This is read-only by construction — `load_skills` walks the filesystem
/// but doesn't mutate anything, and the body returned here is just a string
/// the model already has the right to read (it's already in the system
/// prompt listing).
pub async fn execute_skill(name: &str, args: Option<&str>) -> ExecutionResult {
    info!(target: "jfc::tools", skill_name = name, has_args = args.is_some(), "skill: invoking");
    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    execute_skill_in(&cwd, name, args).await
}
