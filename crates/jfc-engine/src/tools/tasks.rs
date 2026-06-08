use std::sync::Arc;

use tracing::{debug, info, warn};

use super::ExecutionResult;
use super::subagent::execute_skill_in;
use jfc_session::{DeletedFilter, TaskKind, TaskPatch, TaskRisk, TaskStatus, TaskStore};

pub struct TaskCreateRequest {
    pub subject: String,
    pub description: String,
    pub active_form: Option<String>,
    pub blocked_by: Vec<String>,
    pub acceptance_criteria: Option<String>,
    pub verification_command: Option<String>,
    pub risk: Option<String>,
    pub parent_id: Option<String>,
    pub kind: Option<String>,
    pub tags: Vec<String>,
    pub priority: Option<u8>,
    pub effort: Option<String>,
    pub model: Option<String>,
}

pub struct TaskUpdateRequest {
    pub task_id: String,
    pub status: Option<String>,
    pub subject: Option<String>,
    pub description: Option<String>,
    pub owner: Option<String>,
    pub acceptance_criteria: Option<String>,
    pub verification_command: Option<String>,
    pub risk: Option<String>,
    pub parent_id: Option<String>,
    pub kind: Option<String>,
    pub blocked_by: Vec<String>,
    pub tags: Vec<String>,
    pub priority: Option<u8>,
    pub effort: Option<String>,
    pub model: Option<String>,
}

pub fn execute_task_create(
    store: Option<Arc<TaskStore>>,
    request: TaskCreateRequest,
) -> ExecutionResult {
    let TaskCreateRequest {
        subject,
        description,
        active_form,
        blocked_by,
        acceptance_criteria,
        verification_command,
        risk,
        parent_id,
        kind,
        tags,
        priority,
        effort,
        model,
    } = request;
    debug!(target: "jfc::tools", %subject, blocked_count = blocked_by.len(), "task_create: creating");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    if is_placeholder_task_input(&subject, &description) {
        warn!(
            target: "jfc::tools",
            %subject,
            "task_create: rejected placeholder task"
        );
        return ExecutionResult::failure(
            "TaskCreate rejected placeholder subject/description; provide a real task title and description",
        );
    }
    match store.create(subject, description, active_form, blocked_by) {
        Ok(task) => {
            // Apply optional extended fields via a patch
            let parsed_risk = risk.as_deref().and_then(parse_risk);
            let parsed_kind = kind.as_deref().and_then(parse_kind);
            let has_extras = acceptance_criteria.is_some()
                || verification_command.is_some()
                || parsed_risk.is_some()
                || parent_id.is_some()
                || parsed_kind.is_some()
                || !tags.is_empty()
                || priority.is_some()
                || effort.is_some()
                || model.is_some();
            if has_extras {
                let patch = TaskPatch {
                    acceptance_criteria,
                    verification_command,
                    risk: parsed_risk,
                    parent_id: parent_id.map(jfc_session::TaskId::from),
                    kind: parsed_kind,
                    tags: if tags.is_empty() { None } else { Some(tags) },
                    priority,
                    effort,
                    model,
                    ..Default::default()
                };
                match store.update(task.id.as_str(), patch) {
                    Ok(updated) => {
                        debug!(target: "jfc::tools", task_id = %updated.id, "task_create: success with extras");
                        return ExecutionResult::success(
                            serde_json::to_string_pretty(&updated)
                                .unwrap_or_else(|_| format!("{updated:?}")),
                        );
                    }
                    Err(e) => {
                        warn!(target: "jfc::tools", error = %e, "task_create: extras patch failed");
                    }
                }
            }
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

fn is_placeholder_task_input(subject: &str, description: &str) -> bool {
    subject.trim().eq_ignore_ascii_case("subj") && description.trim().eq_ignore_ascii_case("desc")
}
pub fn execute_task_update(
    store: Option<Arc<TaskStore>>,
    request: TaskUpdateRequest,
) -> ExecutionResult {
    let TaskUpdateRequest {
        task_id,
        status,
        subject,
        description,
        owner,
        acceptance_criteria,
        verification_command,
        risk,
        parent_id,
        kind,
        blocked_by,
        tags,
        priority,
        effort,
        model,
    } = request;
    debug!(target: "jfc::tools", task_id, status = status.as_deref(), "task_update: updating");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let parsed_status = match status.as_deref() {
        Some("pending") => Some(TaskStatus::Pending),
        Some("in_progress") => Some(TaskStatus::InProgress),
        Some("completed") => Some(TaskStatus::Completed),
        Some("failed") => Some(TaskStatus::Failed),
        Some("deleted") => Some(TaskStatus::Deleted),
        Some(other) => {
            return ExecutionResult::failure(format!(
                "Invalid task status '{other}'. Expected one of: pending, in_progress, completed, failed, deleted"
            ));
        }
        None => None,
    };
    let patch = TaskPatch {
        subject,
        description,
        status: parsed_status,
        owner,
        acceptance_criteria,
        verification_command,
        risk: risk.as_deref().and_then(parse_risk),
        parent_id: parent_id.map(jfc_session::TaskId::from),
        kind: kind.as_deref().and_then(parse_kind),
        blocked_by: if blocked_by.is_empty() {
            None
        } else {
            Some(blocked_by)
        },
        tags: if tags.is_empty() { None } else { Some(tags) },
        priority,
        effort,
        model,
        ..Default::default()
    };
    match store.update(&task_id, patch) {
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

pub fn execute_task_validate(store: Option<Arc<TaskStore>>) -> ExecutionResult {
    debug!(target: "jfc::tools", "task_validate: validating");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let validation = store.validate();
    let output =
        serde_json::to_string_pretty(&validation).unwrap_or_else(|_| format!("{validation:?}"));
    ExecutionResult::success(output)
}

fn parse_risk(s: &str) -> Option<TaskRisk> {
    match s {
        "low" => Some(TaskRisk::Low),
        "medium" => Some(TaskRisk::Medium),
        "high" => Some(TaskRisk::High),
        _ => None,
    }
}

fn parse_kind(s: &str) -> Option<TaskKind> {
    match s {
        "milestone" => Some(TaskKind::Milestone),
        "task" => Some(TaskKind::Task),
        "check" => Some(TaskKind::Check),
        "decision" => Some(TaskKind::Decision),
        _ => None,
    }
}

/// Cap on archived history records returned in one `TaskList` call. The log is
/// append-only and unbounded on disk; this keeps the tool result token-bounded.
const TASK_HISTORY_RETRIEVAL_LIMIT: usize = 100;

pub fn execute_task_list(
    store: Option<Arc<TaskStore>>,
    status_filter: Option<&str>,
    owner_filter: Option<&str>,
    include_history: bool,
    history_query: Option<&str>,
) -> ExecutionResult {
    debug!(target: "jfc::tools", status_filter, owner_filter, include_history, "task_list: listing");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };
    let mut tasks = store.list(DeletedFilter::Exclude);

    // CC 2.1.167 parity: exclude tasks with metadata._internal = true.
    // These are system-internal tasks the model shouldn't see in its list.
    tasks.retain(|t| {
        !t.metadata
            .as_ref()
            .and_then(|m| m.get("_internal"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    });

    if let Some(sf) = status_filter {
        tasks.retain(|t| {
            let s = serde_json::to_value(t.status)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned));
            s.as_deref() == Some(sf)
        });
    }
    if let Some(of) = owner_filter {
        tasks.retain(|t| t.owner.as_deref() == Some(of));
    }
    debug!(target: "jfc::tools", count = tasks.len(), "task_list: result");

    // CC 2.1.167 parity: strip completed task IDs from blockedBy arrays.
    // A task blocked by an already-completed dependency is effectively
    // unblocked — showing it as blocked is confusing.
    let completed_ids: std::collections::HashSet<String> = tasks
        .iter()
        .filter(|t| t.status == TaskStatus::Completed)
        .map(|t| t.id.as_str().to_owned())
        .collect();

    // Serialize tasks, stripping completed IDs from blockedBy per CC 2.1.167.
    let tasks_json: Vec<serde_json::Value> = tasks
        .iter()
        .map(|t| {
            let mut v = serde_json::to_value(t).unwrap_or(serde_json::Value::Null);
            if let Some(obj) = v.as_object_mut() {
                if let Some(blocked) = obj.get_mut("blocked_by") {
                    if let Some(arr) = blocked.as_array_mut() {
                        arr.retain(|id| {
                            id.as_str()
                                .map(|s| !completed_ids.contains(s))
                                .unwrap_or(true)
                        });
                    }
                }
            }
            v
        })
        .collect();

    // Without history retrieval, preserve the original array-of-tasks shape so
    // existing callers/tests see no change.
    if !include_history {
        let output = serde_json::to_string_pretty(&tasks_json)
            .unwrap_or_else(|_| format!("{} tasks", tasks.len()));
        return ExecutionResult::success(output);
    }

    // Archival memory: read back the durable "everything we've worked on" log
    // (pruned terminal tasks) from the sibling JSONL, newest first.
    let history_path = jfc_session::history_path_for(store.path());
    let history =
        jfc_session::read_task_history(&history_path, TASK_HISTORY_RETRIEVAL_LIMIT, history_query);
    debug!(
        target: "jfc::tools",
        history_count = history.len(),
        history_query,
        "task_list: included archived history"
    );
    let combined = serde_json::json!({
        "active": tasks_json,
        "history": history,
        "history_truncated": history.len() == TASK_HISTORY_RETRIEVAL_LIMIT,
    });
    let output = serde_json::to_string_pretty(&combined)
        .unwrap_or_else(|_| format!("{} active, {} history", tasks.len(), history.len()));
    ExecutionResult::success(output)
}

pub fn execute_task_done(store: Option<Arc<TaskStore>>, task_id: &str) -> ExecutionResult {
    debug!(target: "jfc::tools", task_id, "task_done: marking complete");
    let Some(store) = store else {
        return ExecutionResult::failure("Task store not available");
    };

    // Verification gate: if the task has a verification_command, run it
    // before allowing completion. This prevents stub/incomplete work from
    // being marked done — the command must exit 0.
    if let Some(task) = store.get(task_id)
        && let Some(ref cmd) = task.verification_command
        && !cmd.trim().is_empty()
    {
        debug!(
            target: "jfc::tools",
            task_id,
            cmd,
            "task_done: running verification command"
        );
        let output = std::process::Command::new("bash")
            .arg("-c")
            .arg(cmd)
            .output();
        match output {
            Ok(result) if result.status.success() => {
                debug!(
                    target: "jfc::tools",
                    task_id,
                    "task_done: verification passed"
                );
            }
            Ok(result) => {
                let stderr = String::from_utf8_lossy(&result.stderr);
                let stdout = String::from_utf8_lossy(&result.stdout);
                let truncated_output: String = format!(
                    "stdout: {}\nstderr: {}",
                    &stdout[..stdout.len().min(500)],
                    &stderr[..stderr.len().min(500)]
                );
                warn!(
                    target: "jfc::tools",
                    task_id,
                    exit_code = ?result.status.code(),
                    "task_done: verification FAILED — task remains in_progress"
                );
                return ExecutionResult::failure(format!(
                    "Verification failed (exit {}). Task remains in_progress.\n\
                             Command: {cmd}\n{truncated_output}",
                    result.status.code().unwrap_or(-1)
                ));
            }
            Err(e) => {
                warn!(
                    target: "jfc::tools",
                    task_id,
                    error = %e,
                    "task_done: verification command failed to execute"
                );
                return ExecutionResult::failure(format!(
                    "Verification command failed to execute: {e}. Task remains in_progress."
                ));
            }
        }
    }

    // Evaluator gate: scan recently-modified files for stub patterns
    // (e.g. placeholder macros). Only fires when a git root is
    // discoverable AND we're not in a test environment (tests run in the
    // repo itself and would always find placeholders in unrelated files).
    // Disable with JFC_SKIP_EVALUATOR=1 for CI/test contexts.
    if std::env::var("JFC_SKIP_EVALUATOR").is_err()
        && !cfg!(test)
        && let Some(root) = crate::context::discover_git_root()
    {
        let eval = crate::sprint::evaluate_work_quality(&root);
        if !eval.passed {
            warn!(
                target: "jfc::tools",
                task_id,
                issue_count = eval.issues.len(),
                "task_done: evaluator detected stub patterns — rejecting completion"
            );
            return ExecutionResult::failure(format!(
                "Evaluator rejected: stub/placeholder patterns found in modified files. \
                 Fix these before marking done.\n\n{eval}"
            ));
        }
    }

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

pub fn execute_task_stop(_app: &str, task_id: &str) -> ExecutionResult {
    debug!(target: "jfc::tools", task_id, "task_stop: requesting stop");
    // TaskStop is handled specially by the runtime event loop which has
    // access to App. We return a success message here and the event loop
    // performs the actual cancellation when it processes the tool result.
    ExecutionResult::success(format!(
        "Stop signal sent to task {task_id}. The runtime will cancel it."
    ))
}

pub fn execute_task_get(store: Option<Arc<TaskStore>>, task_id: &str) -> ExecutionResult {
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    fn is_internal(metadata: Option<&serde_json::Value>) -> bool {
        metadata
            .and_then(|m| m.get("_internal"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }

    #[test]
    fn internal_filter_detects_true_normal() {
        let meta = json!({"_internal": true});
        assert!(is_internal(Some(&meta)));
    }

    #[test]
    fn internal_filter_allows_false_normal() {
        let meta = json!({"_internal": false});
        assert!(!is_internal(Some(&meta)));
    }

    #[test]
    fn internal_filter_allows_no_metadata_robust() {
        assert!(!is_internal(None));
    }

    #[test]
    fn internal_filter_allows_other_metadata_robust() {
        let meta = json!({"priority": 1, "kind": "task"});
        assert!(!is_internal(Some(&meta)));
    }

    #[test]
    fn blocked_by_completed_filter_removes_completed_ids_normal() {
        let mut arr = vec![
            json!("t1"),
            json!("t2"),
            json!("t3"),
        ];
        let completed: std::collections::HashSet<String> =
            ["t1".to_string(), "t3".to_string()].into();
        arr.retain(|id| {
            id.as_str()
                .map(|s| !completed.contains(s))
                .unwrap_or(true)
        });
        assert_eq!(arr, vec![json!("t2")]);
    }
}
