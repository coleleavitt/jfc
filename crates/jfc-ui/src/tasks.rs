//! v126 task/todo system.
//!
//! Mirrors `cli.js` v126's task tools (TaskCreate / TaskGet / TaskUpdate /
//! TaskList / TaskDelete / TaskDone) and persistent store. Tasks are NOT
//! conversation messages — they live in a JSON file under
//! `~/.config/jfc/tasks/` and survive session resume, compaction, and
//! context-window limits.
//!
//! The data model:
//!
//! ```ignore
//! {
//!   id: "t1",
//!   subject: "Fix authentication bug",
//!   description: "...",
//!   activeForm: "Fixing authentication bug",   // spinner text
//!   status: "pending" | "in_progress" | "completed" | "deleted",
//!   owner: "impl",                             // teammate name (optional)
//!   blocks: ["t2"],                            // tasks blocked by this
//!   blockedBy: ["t0"],                         // tasks blocking this
//!   metadata: { ... }
//! }
//! ```
//!
//! What's intentionally not here:
//! - Live activity descriptions (no teammate runtime yet).
//! - Animation / fade-out for recently-completed tasks (UI polish).
//! - Live activity descriptions (no teammate runtime yet).
//! - Animation / fade-out for recently-completed tasks (UI polish).

#![allow(dead_code)]

use std::borrow::Borrow;
use std::collections::HashMap;
use std::fmt;
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

/// Stable task identity. Kept as a transparent string on disk/wire, but typed
/// in-process so task ids cannot be accidentally mixed with subjects, owners,
/// or tool ids.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TaskId(String);

impl TaskId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for TaskId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for TaskId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for TaskId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for TaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for TaskId {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<TaskId> for &str {
    fn eq(&self, other: &TaskId) -> bool {
        *self == other.as_str()
    }
}

impl From<String> for TaskId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for TaskId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeletedFilter {
    Exclude,
    Include,
}

impl DeletedFilter {
    fn includes_deleted(self) -> bool {
        matches!(self, Self::Include)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskError {
    UnknownTask { id: TaskId },
    UnknownDependency { id: TaskId },
    SelfCycle { id: TaskId },
    DependencyCycle { path: Vec<TaskId> },
}

impl fmt::Display for TaskError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTask { id } => write!(f, "unknown task id `{id}`"),
            Self::UnknownDependency { id } => {
                write!(f, "blockedBy references unknown task id `{id}`")
            }
            Self::SelfCycle { .. } => f.write_str("a task cannot block itself"),
            Self::DependencyCycle { path } => {
                let chain = path
                    .iter()
                    .map(TaskId::as_str)
                    .collect::<Vec<_>>()
                    .join(" -> ");
                write!(f, "blockedBy would create dependency cycle: {chain}")
            }
        }
    }
}

impl std::error::Error for TaskError {}

/// A task's lifecycle status. `Deleted` is a tombstone — `TaskList` filters it
/// out by default but it remains in the store for audit purposes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Pending,
    InProgress,
    Completed,
    Deleted,
}

impl Default for TaskStatus {
    fn default() -> Self {
        Self::Pending
    }
}

/// One task. Field names match v126's wire shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TaskId,
    pub subject: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_form: Option<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub blocks: Vec<TaskId>,
    #[serde(default, rename = "blockedBy")]
    pub blocked_by: Vec<TaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    /// Monotonic creation counter, used for stable sort by recency.
    #[serde(default)]
    pub created_at_ms: u64,
}

impl Task {
    /// Spinner text to show while the task is in_progress. Falls back to the
    /// subject when activeForm wasn't supplied.
    pub fn spinner_text(&self) -> &str {
        self.active_form.as_deref().unwrap_or(&self.subject)
    }
}

/// Persistent task store. Read-modify-write with a `Mutex` because all
/// tool-call dispatch happens on the same `tokio` runtime.
#[derive(Debug, Default)]
pub struct TaskStore {
    inner: Mutex<TaskStoreInner>,
    path: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct TaskStoreInner {
    /// Next free numeric suffix for `t<N>` task ids.
    next_id: u64,
    /// All tasks keyed by id.
    tasks: HashMap<TaskId, Task>,
}

impl TaskStore {
    /// Open or create the task store for the given session id. Path:
    /// `~/.config/jfc/tasks/<session>.json`. A fresh store is returned if the
    /// file doesn't exist or is malformed (we never panic on user data).
    pub fn open(session_id: &str) -> Arc<Self> {
        let path = dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("jfc")
            .join("tasks")
            .join(format!("{session_id}.json"));
        tracing::info!(
            target: "jfc::tasks",
            session_id,
            path = %path.display(),
            "TaskStore::open"
        );
        let inner = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        Arc::new(Self {
            inner: Mutex::new(inner),
            path,
        })
    }

    /// In-memory store (no persistence) — used in tests.
    pub fn in_memory() -> Arc<Self> {
        tracing::debug!(target: "jfc::tasks", "TaskStore::in_memory");
        Arc::new(Self::default())
    }

    fn persist(&self, inner: &TaskStoreInner) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        if let Some(parent) = self.path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(inner) {
            let tmp = self.path.with_extension("tmp");
            if std::fs::write(&tmp, json).is_ok() {
                let _ = std::fs::rename(&tmp, &self.path);
            }
        }
    }

    /// Create a new task. Returns Err on duplicate `subject` if you'd want to
    /// dedupe — currently always succeeds. Validates that any `blocked_by`
    /// targets exist.
    pub fn create<B>(
        &self,
        subject: String,
        description: String,
        active_form: Option<String>,
        blocked_by: Vec<B>,
    ) -> Result<Task, TaskError>
    where
        B: Into<TaskId>,
    {
        let mut inner = self.inner.lock().unwrap();
        let blocked_by = blocked_by.into_iter().map(Into::into).collect::<Vec<_>>();
        for dep in &blocked_by {
            if !inner.tasks.contains_key(dep.as_str()) {
                return Err(TaskError::UnknownDependency { id: dep.clone() });
            }
        }
        inner.next_id += 1;
        let id = TaskId::new(format!("t{}", inner.next_id));
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        // Reverse-link: each task in `blocked_by` should record this task in
        // its `blocks` list.
        let blocked_by_clone = blocked_by.clone();
        for dep in &blocked_by_clone {
            if let Some(t) = inner.tasks.get_mut(dep.as_str()) {
                if !t.blocks.contains(&id) {
                    t.blocks.push(id.clone());
                }
            }
        }
        let truncated_subject: &str = if subject.len() > 80 { &subject[..80] } else { &subject };
        tracing::info!(
            target: "jfc::tasks",
            id = %id,
            subject = truncated_subject,
            "TaskStore::create"
        );
        let task = Task {
            id: id.clone(),
            subject,
            description,
            active_form,
            status: TaskStatus::Pending,
            owner: None,
            blocks: Vec::new(),
            blocked_by: blocked_by_clone,
            metadata: None,
            created_at_ms: now_ms,
        };
        inner.tasks.insert(id, task.clone());
        self.persist(&inner);
        Ok(task)
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        tracing::trace!(target: "jfc::tasks", id, "TaskStore::get");
        self.inner.lock().unwrap().tasks.get(id).cloned()
    }

    /// Update a task's mutable fields. Returns Err if the task doesn't exist
    /// or the update would create an immediate self-cycle. Cascading
    /// `unblock` happens automatically when status flips to `Completed`.
    pub fn update(&self, id: &str, patch: TaskPatch) -> Result<Task, TaskError> {
        tracing::debug!(
            target: "jfc::tasks",
            id,
            has_status = patch.status.is_some(),
            has_subject = patch.subject.is_some(),
            has_owner = patch.owner.is_some(),
            "TaskStore::update"
        );
        let mut inner = self.inner.lock().unwrap();
        let task_id = TaskId::from(id);
        if !inner.tasks.contains_key(id) {
            return Err(TaskError::UnknownTask { id: task_id });
        }
        let next_blocked_by = patch.blocked_by.as_ref().map(|deps| {
            deps.iter()
                .map(|dep| TaskId::from(dep.as_str()))
                .collect::<Vec<_>>()
        });
        if let Some(deps) = &next_blocked_by {
            if deps.iter().any(|d| d.as_str() == id) {
                return Err(TaskError::SelfCycle { id: task_id });
            }
            for dep in deps {
                if !inner.tasks.contains_key(dep.as_str()) {
                    return Err(TaskError::UnknownDependency { id: dep.clone() });
                }
                if let Some(mut path) = dependency_path_to(&inner, dep, id) {
                    path.insert(0, TaskId::from(id));
                    return Err(TaskError::DependencyCycle { path });
                }
            }
        }

        let task = inner.tasks.get_mut(id).unwrap();
        if let Some(s) = patch.subject {
            task.subject = s;
        }
        if let Some(d) = patch.description {
            task.description = d;
        }
        if let Some(af) = patch.active_form {
            task.active_form = Some(af);
        }
        if let Some(st) = patch.status {
            task.status = st;
        }
        if let Some(o) = patch.owner {
            task.owner = Some(o);
        }
        if let Some(deps) = next_blocked_by {
            task.blocked_by = deps;
        }
        if let Some(m) = patch.metadata {
            task.metadata = Some(m);
        }

        let updated = task.clone();
        // If we just completed this task, anything it blocks may now be
        // unblockable. We don't auto-flip status — that's the user/agent's
        // call — but we surface in `list_unblocked` for UIs.
        self.persist(&inner);
        Ok(updated)
    }

    /// Remove a task permanently (sets status: deleted, removes from blockers).
    pub fn delete(&self, id: &str) -> Result<(), TaskError> {
        tracing::debug!(target: "jfc::tasks", id, "TaskStore::delete");
        let mut inner = self.inner.lock().unwrap();
        if !inner.tasks.contains_key(id) {
            return Err(TaskError::UnknownTask {
                id: TaskId::from(id),
            });
        }
        // Strip references from other tasks' blocks/blockedBy.
        for t in inner.tasks.values_mut() {
            t.blocks.retain(|b| b.as_str() != id);
            t.blocked_by.retain(|b| b.as_str() != id);
        }
        if let Some(t) = inner.tasks.get_mut(id) {
            t.status = TaskStatus::Deleted;
        }
        self.persist(&inner);
        Ok(())
    }

    /// All tasks, sorted by creation order. Excludes Deleted unless asked.
    /// Sort key is the numeric suffix of the task id (`t1`, `t2`, …) so we
    /// get strict monotonic order even when multiple creates fall in the
    /// same millisecond.
    pub fn list(&self, deleted_filter: DeletedFilter) -> Vec<Task> {
        let mut out: Vec<Task> = self
            .inner
            .lock()
            .unwrap()
            .tasks
            .values()
            .filter(|t| deleted_filter.includes_deleted() || t.status != TaskStatus::Deleted)
            .cloned()
            .collect();
        out.sort_by_key(|t| {
            t.id.as_str()
                .strip_prefix('t')
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0)
        });
        tracing::trace!(
            target: "jfc::tasks",
            filter = ?deleted_filter,
            count = out.len(),
            "TaskStore::list"
        );
        out
    }

    /// Counts by status — used by the UI overflow summary.
    pub fn counts(&self) -> TaskCounts {
        tracing::trace!(target: "jfc::tasks", "TaskStore::counts");
        let inner = self.inner.lock().unwrap();
        let mut c = TaskCounts::default();
        for t in inner.tasks.values() {
            match t.status {
                TaskStatus::Pending => c.pending += 1,
                TaskStatus::InProgress => c.in_progress += 1,
                TaskStatus::Completed => c.completed += 1,
                TaskStatus::Deleted => {}
            }
        }
        c
    }
}

#[derive(Debug, Default, Clone)]
pub struct TaskPatch {
    pub subject: Option<String>,
    pub description: Option<String>,
    pub active_form: Option<String>,
    pub status: Option<TaskStatus>,
    pub owner: Option<String>,
    pub blocked_by: Option<Vec<String>>,
    pub metadata: Option<serde_json::Value>,
}

fn dependency_path_to(inner: &TaskStoreInner, start: &TaskId, target: &str) -> Option<Vec<TaskId>> {
    let task = inner.tasks.get(start.as_str())?;
    if task.blocked_by.iter().any(|dep| dep.as_str() == target) {
        return Some(vec![start.clone(), TaskId::from(target)]);
    }

    for dep in &task.blocked_by {
        if let Some(mut path) = dependency_path_to(inner, dep, target) {
            path.insert(0, start.clone());
            return Some(path);
        }
    }

    None
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TaskCounts {
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: create→get→update→list round-trips.
    #[test]
    fn create_get_update_list_roundtrip_normal() {
        let store = TaskStore::in_memory();
        let task = store
            .create(
                "Fix auth bug".into(),
                "details".into(),
                Some("Fixing auth bug".into()),
                Vec::<TaskId>::new(),
            )
            .unwrap();
        assert_eq!(task.id, "t1");
        assert_eq!(task.status, TaskStatus::Pending);
        assert_eq!(task.spinner_text(), "Fixing auth bug");

        let fetched = store.get(&task.id).expect("present");
        assert_eq!(fetched.subject, "Fix auth bug");

        let updated = store
            .update(
                &task.id,
                TaskPatch {
                    status: Some(TaskStatus::InProgress),
                    owner: Some("impl".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.status, TaskStatus::InProgress);
        assert_eq!(updated.owner.as_deref(), Some("impl"));

        let list = store.list(DeletedFilter::Exclude);
        assert_eq!(list.len(), 1);
    }

    // Normal: blocked_by cross-links — when t2 declares blocked_by=[t1], t1's
    // `blocks` list includes t2.
    #[test]
    fn blocked_by_cross_links_normal() {
        let store = TaskStore::in_memory();
        let t1 = store
            .create("first".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let t2 = store
            .create("second".into(), "".into(), None, vec![t1.id.clone()])
            .unwrap();
        let t1_after = store.get(&t1.id).unwrap();
        assert_eq!(t1_after.blocks, vec![t2.id.clone()]);
        assert_eq!(t2.blocked_by, vec![t1.id.clone()]);
    }

    // Robust: blocked_by referencing a non-existent task fails create cleanly.
    #[test]
    fn create_with_unknown_dep_errors_robust() {
        let store = TaskStore::in_memory();
        let result = store.create(
            "needs ghost".into(),
            "".into(),
            None,
            vec![TaskId::from("t999")],
        );
        assert!(result.is_err());
    }

    // Robust: a task can't declare itself in its own blocked_by list (immediate
    // self-cycle).
    #[test]
    fn update_self_cycle_rejected_robust() {
        let store = TaskStore::in_memory();
        let t = store
            .create("solo".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let result = store.update(
            &t.id,
            TaskPatch {
                blocked_by: Some(vec![t.id.to_string()]),
                ..Default::default()
            },
        );
        assert!(result.is_err());
    }

    // Robust: transitive blocked_by cycles are rejected with a proof-like path.
    #[test]
    fn update_transitive_cycle_rejected_robust() {
        let store = TaskStore::in_memory();
        let t1 = store
            .create("a".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let t2 = store
            .create("b".into(), "".into(), None, vec![t1.id.clone()])
            .unwrap();

        let err = store
            .update(
                t1.id.as_str(),
                TaskPatch {
                    blocked_by: Some(vec![t2.id.to_string()]),
                    ..Default::default()
                },
            )
            .unwrap_err();

        assert!(matches!(err, TaskError::DependencyCycle { .. }));
        assert_eq!(
            err.to_string(),
            "blockedBy would create dependency cycle: t1 -> t2 -> t1"
        );
    }

    // Robust: deleting a task strips it from other tasks' blocks/blockedBy
    // lists so they don't dangle.
    #[test]
    fn delete_strips_dependent_links_robust() {
        let store = TaskStore::in_memory();
        let t1 = store
            .create("a".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let t2 = store
            .create("b".into(), "".into(), None, vec![t1.id.clone()])
            .unwrap();
        store.delete(&t1.id).unwrap();
        let t2_after = store.get(&t2.id).unwrap();
        assert!(t2_after.blocked_by.is_empty(), "{t2_after:?}");
        // The deleted task itself remains as a tombstone with status=Deleted.
        let t1_after = store.get(&t1.id).unwrap();
        assert_eq!(t1_after.status, TaskStatus::Deleted);
        // Default list excludes deleted.
        assert_eq!(store.list(DeletedFilter::Exclude).len(), 1);
        assert_eq!(store.list(DeletedFilter::Include).len(), 2);
    }

    // Normal: counts() bins by status.
    #[test]
    fn counts_bins_by_status_normal() {
        let store = TaskStore::in_memory();
        let t1 = store
            .create("a".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let t2 = store
            .create("b".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let t3 = store
            .create("c".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        store
            .update(
                &t1.id,
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        store
            .update(
                &t2.id,
                TaskPatch {
                    status: Some(TaskStatus::InProgress),
                    ..Default::default()
                },
            )
            .unwrap();
        let _ = t3;
        let c = store.counts();
        assert_eq!(c.completed, 1);
        assert_eq!(c.in_progress, 1);
        assert_eq!(c.pending, 1);
    }

    // Normal: sequential creates produce monotonically increasing ids and
    // timestamps so list() returns creation order.
    #[test]
    fn list_returns_creation_order_normal() {
        let store = TaskStore::in_memory();
        let names = ["a", "b", "c", "d"];
        for n in names {
            store
                .create(n.into(), "".into(), None, Vec::<TaskId>::new())
                .unwrap();
        }
        let listed: Vec<String> = store
            .list(DeletedFilter::Exclude)
            .into_iter()
            .map(|t| t.subject)
            .collect();
        assert_eq!(listed, names);
    }

    // Robust: TaskStatus serializes as snake_case strings (matches v126 wire
    // shape exactly).
    #[test]
    fn task_status_snake_case_serde_robust() {
        let s = serde_json::to_string(&TaskStatus::InProgress).unwrap();
        assert_eq!(s, "\"in_progress\"");
        let parsed: TaskStatus = serde_json::from_str("\"completed\"").unwrap();
        assert_eq!(parsed, TaskStatus::Completed);
    }
}
