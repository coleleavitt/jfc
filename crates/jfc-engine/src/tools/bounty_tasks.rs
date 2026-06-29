use std::sync::Arc;

use jfc_session::{
    DeletedFilter, TaskBountyRef, TaskExecutionMetadata, TaskId, TaskKind, TaskPatch, TaskRisk,
    TaskStatus, TaskStore,
};

pub(crate) struct PostBountyExecution {
    pub description: String,
    pub budget: u64,
    pub acceptance_criteria: String,
    pub max_solvers: Option<u8>,
    pub auto_dispatch: bool,
    pub parent_task_id: Option<String>,
    pub task_store: Option<Arc<TaskStore>>,
}

pub(crate) struct RunBountyExecution {
    pub bounty_id: String,
    pub max_solvers: Option<u8>,
    pub task_store: Option<Arc<TaskStore>>,
}

fn bounty_preview(description: &str) -> String {
    let first_line = description
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("coding bounty")
        .trim();
    if first_line.len() <= 72 {
        return first_line.to_owned();
    }
    let boundary = first_line
        .char_indices()
        .map(|(idx, _)| idx)
        .take_while(|idx| *idx <= 72)
        .last()
        .unwrap_or(72);
    format!("{}...", &first_line[..boundary])
}

fn bounty_task_subject(bounty_id: &str, description: &str) -> String {
    format!("Bounty {bounty_id}: {}", bounty_preview(description))
}

fn bounty_task_metadata(
    bounty_id: &str,
    budget: u64,
    max_solvers: Option<u8>,
    auto_dispatch: bool,
) -> serde_json::Value {
    TaskExecutionMetadata::bounty(
        "bounty market is the selected execution strategy for this task",
        Some(TaskBountyRef::new(
            bounty_id,
            budget,
            max_solvers,
            auto_dispatch,
        )),
    )
    .to_task_metadata(None)
}

fn task_bounty_id(task: &jfc_session::Task) -> Option<String> {
    TaskExecutionMetadata::from_task(task)
        .and_then(|execution| execution.bounty.map(|bounty| bounty.bounty_id))
}

fn find_bounty_task_id(store: &TaskStore, bounty_id: &str) -> Option<String> {
    store
        .list(DeletedFilter::Exclude)
        .into_iter()
        .find(|task| task_bounty_id(task).as_deref() == Some(bounty_id))
        .map(|task| task.id.to_string())
}

fn push_unique_tag(tags: &mut Vec<String>, tag: impl Into<String>) {
    let tag = tag.into();
    if !tags.iter().any(|existing| existing == &tag) {
        tags.push(tag);
    }
}

fn bounty_tags(existing: &[String], bounty_id: &str) -> Vec<String> {
    let mut tags = existing.to_vec();
    push_unique_tag(&mut tags, "bounty");
    push_unique_tag(&mut tags, "market");
    push_unique_tag(&mut tags, bounty_id);
    tags
}

fn annotate_existing_bounty_task(
    store: &TaskStore,
    task_id: &str,
    bounty_id: &str,
    budget: u64,
    acceptance_criteria: &str,
    max_solvers: Option<u8>,
    auto_dispatch: bool,
) -> Option<String> {
    let Some(task) = store.get(task_id) else {
        tracing::warn!(
            target: "jfc::market::tasks",
            bounty_id,
            task_id,
            "post_bounty parent_task_id did not match a task; creating a bounty task instead"
        );
        return None;
    };
    let status = if auto_dispatch {
        TaskStatus::InProgress
    } else {
        task.status
    };
    let owner = auto_dispatch.then(|| "market".to_owned()).or(task.owner);
    match store.update(
        task.id.as_str(),
        TaskPatch {
            status: Some(status),
            owner,
            metadata: Some(
                TaskExecutionMetadata::bounty(
                    "bounty market is the selected execution strategy for this task",
                    Some(TaskBountyRef::new(
                        bounty_id,
                        budget,
                        max_solvers,
                        auto_dispatch,
                    )),
                )
                .to_task_metadata(task.metadata.as_ref()),
            ),
            acceptance_criteria: Some(acceptance_criteria.to_owned()),
            risk: Some(TaskRisk::High),
            kind: task.kind.or(Some(TaskKind::Task)),
            tags: Some(bounty_tags(&task.tags, bounty_id)),
            priority: task.priority.or(Some(1)),
            active_form: Some(format!("Run bounty {bounty_id}")),
            ..Default::default()
        },
    ) {
        Ok(updated) => Some(updated.id.to_string()),
        Err(error) => {
            tracing::warn!(
                target: "jfc::market::tasks",
                bounty_id,
                task_id = %task.id,
                error = %error,
                "failed to annotate parent task for bounty"
            );
            None
        }
    }
}

pub(crate) fn create_bounty_task(
    store: Option<&Arc<TaskStore>>,
    bounty_id: &str,
    description: &str,
    budget: u64,
    acceptance_criteria: &str,
    max_solvers: Option<u8>,
    auto_dispatch: bool,
    parent_task_id: Option<&str>,
) -> Option<String> {
    let store = store?;
    if let Some(parent_task_id) = parent_task_id
        && let Some(task_id) = annotate_existing_bounty_task(
            store,
            parent_task_id,
            bounty_id,
            budget,
            acceptance_criteria,
            max_solvers,
            auto_dispatch,
        )
    {
        return Some(task_id);
    }
    let task = match store.create(
        bounty_task_subject(bounty_id, description),
        format!(
            "{description}\n\nAcceptance criteria:\n{acceptance_criteria}\n\nMarket bounty `{bounty_id}` with budget {budget} tokens."
        ),
        Some(format!("Run bounty {bounty_id}")),
        Vec::<String>::new(),
    ) {
        Ok(task) => task,
        Err(jfc_session::TaskError::DuplicateSubject { existing_id, .. }) => {
            return Some(existing_id.to_string());
        }
        Err(error) => {
            tracing::warn!(
                target: "jfc::market::tasks",
                bounty_id,
                error = %error,
                "failed to create bounty task"
            );
            return None;
        }
    };
    let status = if auto_dispatch {
        TaskStatus::InProgress
    } else {
        TaskStatus::Pending
    };
    let owner = auto_dispatch.then(|| "market".to_owned());
    let patch = TaskPatch {
        status: Some(status),
        owner,
        metadata: Some(bounty_task_metadata(
            bounty_id,
            budget,
            max_solvers,
            auto_dispatch,
        )),
        acceptance_criteria: Some(acceptance_criteria.to_owned()),
        risk: Some(TaskRisk::High),
        parent_id: parent_task_id.map(TaskId::from),
        kind: Some(TaskKind::Task),
        tags: Some(bounty_tags(&[], bounty_id)),
        priority: Some(1),
        ..Default::default()
    };
    match store.update(task.id.as_str(), patch) {
        Ok(updated) => Some(updated.id.to_string()),
        Err(error) => {
            tracing::warn!(
                target: "jfc::market::tasks",
                bounty_id,
                task_id = %task.id,
                error = %error,
                "failed to annotate bounty task"
            );
            Some(task.id.to_string())
        }
    }
}

pub(crate) fn ensure_bounty_task_for_run(
    store: Option<&Arc<TaskStore>>,
    bounty_id: &str,
    max_solvers: Option<u8>,
) -> Option<String> {
    let store = store?;
    if let Some(task_id) = find_bounty_task_id(store, bounty_id) {
        return Some(task_id);
    }
    create_bounty_task(
        Some(store),
        bounty_id,
        &format!("Run posted market bounty `{bounty_id}`"),
        0,
        "Drive the bounty through Solve -> Validate -> Settle.",
        max_solvers,
        true,
        None,
    )
}

pub(crate) fn update_bounty_task(
    store: Option<&Arc<TaskStore>>,
    bounty_id: &str,
    task_id: Option<&str>,
    status: TaskStatus,
    active_form: impl Into<String>,
) -> Option<String> {
    let store = store?;
    let id = task_id
        .map(str::to_owned)
        .or_else(|| find_bounty_task_id(store, bounty_id))?;
    let owner = matches!(status, TaskStatus::InProgress).then(|| "market".to_owned());
    match store.update(
        &id,
        TaskPatch {
            status: Some(status),
            active_form: Some(active_form.into()),
            owner,
            ..Default::default()
        },
    ) {
        Ok(updated) => Some(updated.id.to_string()),
        Err(error) => {
            tracing::warn!(
                target: "jfc::market::tasks",
                bounty_id,
                task_id = %id,
                error = %error,
                "failed to update bounty task"
            );
            None
        }
    }
}
