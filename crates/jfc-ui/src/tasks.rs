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

#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};

pub use jfc_core::{
    Task, TaskCounts, TaskError, TaskKind, TaskPatch, TaskRisk, TaskStatus, TaskValidation,
    TodoTaskId as TaskId,
};

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

/// Persistent task store. Read-modify-write with a `Mutex` because all
/// tool-call dispatch happens on the same `tokio` runtime.
#[derive(Debug, Default)]
pub struct TaskStore {
    inner: Mutex<TaskStoreInner>,
    path: PathBuf,
    /// Last on-disk modification time this store has observed — either
    /// because it loaded that revision or wrote it. `reload_if_changed`
    /// compares the live file mtime against this to decide whether an
    /// external writer (a detached background worker in its own process)
    /// has touched the file since. Lock order is always `inner` → this,
    /// matching `persist`, so the two can't deadlock.
    disk_mtime: Mutex<Option<std::time::SystemTime>>,
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
        let inner = Self::load_inner(&path);
        let disk_mtime = Self::file_mtime(&path);
        Arc::new(Self {
            inner: Mutex::new(inner),
            path,
            disk_mtime: Mutex::new(disk_mtime),
        })
    }

    /// Open the task store shared by a swarm team. This intentionally uses
    /// Claude-compatible swarm storage (`~/.claude/tasks/<team>/tasks.json`)
    /// so the leader and in-process teammates coordinate over one list.
    pub fn open_team(team_name: &str) -> Arc<Self> {
        let path = crate::swarm::team_helpers::tasks_dir(team_name).join("tasks.json");
        tracing::info!(
            target: "jfc::tasks",
            team_name,
            path = %path.display(),
            "TaskStore::open_team"
        );
        let disk_mtime = Self::file_mtime(&path);
        Arc::new(Self {
            inner: Mutex::new(Self::load_inner(&path)),
            path,
            disk_mtime: Mutex::new(disk_mtime),
        })
    }

    /// In-memory store (no persistence) — used in tests.
    pub fn in_memory() -> Arc<Self> {
        tracing::debug!(target: "jfc::tasks", "TaskStore::in_memory");
        Arc::new(Self::default())
    }

    /// Copy every task from `src` into `self`, preserving ids. Used when
    /// the leader activates team mode — the active task store is swapped
    /// from the session store to the team store, and any tasks the user
    /// or leader created before the spawn would otherwise become
    /// orphans. Existing tasks in `self` win on id collision (so a team
    /// roster with hand-edited entries isn't clobbered).
    ///
    /// Returns the number of tasks copied. Best-effort: never fails —
    /// callers that hit a lock-poisoned source store get zero copied
    /// rather than a propagated panic.
    pub fn migrate_from(&self, src: &TaskStore) -> usize {
        let src_inner = match src.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let mut dst_inner = match self.inner.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        let mut copied = 0usize;
        for (id, task) in src_inner.tasks.iter() {
            if dst_inner.tasks.contains_key(id) {
                continue;
            }
            dst_inner.tasks.insert(id.clone(), task.clone());
            copied += 1;
        }
        if dst_inner.next_id < src_inner.next_id {
            dst_inner.next_id = src_inner.next_id;
        }
        let snapshot = TaskStoreInner {
            next_id: dst_inner.next_id,
            tasks: dst_inner.tasks.clone(),
        };
        drop(dst_inner);
        drop(src_inner);
        if copied > 0 {
            self.persist(&snapshot);
            tracing::info!(
                target: "jfc::tasks",
                copied,
                "migrate_from: copied tasks across stores"
            );
        }
        copied
    }

    /// Current on-disk modification time of `path`, or `None` if the file
    /// doesn't exist / can't be stat'd. Used to mtime-gate `reload_if_changed`.
    fn file_mtime(path: &PathBuf) -> Option<std::time::SystemTime> {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    }

    /// Re-read the backing file when an external process has modified it
    /// since this handle last loaded or persisted. Returns `true` if the
    /// in-memory state changed.
    ///
    /// Why: the UI's `TaskStore` is loaded once into a `Mutex` and never
    /// re-reads. A detached background worker runs in a *separate process*
    /// with its own handle; its `TaskUpdate`/`TaskDone` writes land in the
    /// JSON file but the UI handle stays stale forever. The UI's render
    /// loop calls this once per tick (mtime-gated, so it's a cheap stat
    /// when nothing changed) to pick up those external writes.
    ///
    /// In-memory stores (empty `path`) are always a no-op.
    pub fn reload_if_changed(&self) -> bool {
        if self.path.as_os_str().is_empty() {
            return false;
        }
        let current = Self::file_mtime(&self.path);
        {
            let seen = self.disk_mtime.lock().unwrap();
            if *seen == current {
                return false;
            }
        }
        let fresh = Self::load_inner(&self.path);
        let mut inner = self.inner.lock().unwrap();
        *inner = fresh;
        drop(inner);
        *self.disk_mtime.lock().unwrap() = current;
        tracing::debug!(
            target: "jfc::tasks",
            path = %self.path.display(),
            "TaskStore::reload_if_changed — picked up external write"
        );
        true
    }

    fn load_inner(path: &PathBuf) -> TaskStoreInner {
        let Some(raw) = std::fs::read_to_string(path).ok() else {
            return TaskStoreInner::default();
        };
        if let Ok(inner) = serde_json::from_str::<TaskStoreInner>(&raw) {
            return inner;
        }
        // Older swarm code wrote a bare task array to tasks.json. Accept it
        // so existing team task files migrate on next persist instead of
        // silently appearing empty.
        if let Ok(tasks) = serde_json::from_str::<Vec<Task>>(&raw) {
            let next_id = tasks
                .iter()
                .filter_map(|t| t.id.as_str().trim_start_matches('t').parse::<u64>().ok())
                .max()
                .unwrap_or(0);
            return TaskStoreInner {
                next_id,
                tasks: tasks.into_iter().map(|t| (t.id.clone(), t)).collect(),
            };
        }
        TaskStoreInner::default()
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
            if std::fs::write(&tmp, json).is_ok() && std::fs::rename(&tmp, &self.path).is_ok() {
                // Record the mtime of the revision we just wrote so a
                // subsequent `reload_if_changed` doesn't treat our own
                // write as an external change and clobber newer in-memory
                // state with a re-read of what we just serialized.
                if let Ok(mut seen) = self.disk_mtime.lock() {
                    *seen = Self::file_mtime(&self.path);
                }
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
        let blocked_by = blocked_by
            .into_iter()
            .map(Into::into)
            .collect::<BTreeSet<TaskId>>();
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
        // its `blocks` list. `BTreeSet::insert` naturally dedupes, so this is
        // idempotent across re-creates with the same id.
        for dep in &blocked_by {
            if let Some(t) = inner.tasks.get_mut(dep.as_str()) {
                t.blocks.insert(id.clone());
            }
        }
        let truncated_subject: &str = if subject.len() > 80 {
            &subject[..subject.floor_char_boundary(80)]
        } else {
            &subject
        };
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
            blocks: BTreeSet::new(),
            blocked_by,
            metadata: None,
            created_at_ms: now_ms,
            acceptance_criteria: None,
            verification_command: None,
            risk: None,
            parent_id: None,
            kind: None,
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
                .collect::<BTreeSet<TaskId>>()
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
        if let Some(ac) = patch.acceptance_criteria {
            task.acceptance_criteria = Some(ac);
        }
        if let Some(vc) = patch.verification_command {
            task.verification_command = Some(vc);
        }
        if let Some(r) = patch.risk {
            task.risk = Some(r);
        }
        if let Some(pid) = patch.parent_id {
            task.parent_id = Some(pid);
        }
        if let Some(k) = patch.kind {
            task.kind = Some(k);
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

    /// Atomically claim the first pending, unowned task whose blockers are
    /// all completed. Used by in-process teammates while idle-polling.
    pub fn claim_next_available(&self, owner: &str) -> Option<Task> {
        let mut inner = self.inner.lock().unwrap();
        let completed = inner
            .tasks
            .values()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        let mut ids = inner.tasks.keys().cloned().collect::<Vec<_>>();
        ids.sort_by_key(|id| {
            id.as_str()
                .strip_prefix('t')
                .and_then(|n| n.parse::<u64>().ok())
                .unwrap_or(0)
        });
        let claim_id = ids.into_iter().find(|id| {
            inner.tasks.get(id.as_str()).is_some_and(|task| {
                task.status == TaskStatus::Pending
                    && task.owner.as_deref().unwrap_or("").is_empty()
                    && task.blocked_by.iter().all(|dep| completed.contains(dep))
            })
        })?;
        let task = inner.tasks.get_mut(claim_id.as_str())?;
        task.owner = Some(owner.to_owned());
        task.status = TaskStatus::InProgress;
        let task = task.clone();
        self.persist(&inner);
        Some(task)
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
                TaskStatus::Failed | TaskStatus::Deleted => {}
            }
        }
        c
    }

    /// List all tasks (excluding deleted). Convenience for plan verification.
    pub fn list_all(&self) -> Vec<Task> {
        self.list(DeletedFilter::Exclude)
    }

    /// Cascade failure: when a task fails, mark all tasks that depend on it
    /// (directly or transitively) as Failed. Returns the list of newly-failed
    /// task IDs. Persists after cascade.
    pub fn cascade_failure(&self, failed_id: &str) -> Vec<TaskId> {
        let mut inner = self.inner.lock().unwrap();
        let mut newly_failed = Vec::new();
        let mut queue = std::collections::VecDeque::new();
        queue.push_back(TaskId::new(failed_id.to_string()));

        while let Some(current_id) = queue.pop_front() {
            // Find all tasks whose blocked_by contains current_id
            let dependents: Vec<TaskId> = inner
                .tasks
                .values()
                .filter(|t| {
                    t.blocked_by.contains(&current_id)
                        && t.status != TaskStatus::Failed
                        && t.status != TaskStatus::Deleted
                        && t.status != TaskStatus::Completed
                })
                .map(|t| t.id.clone())
                .collect();

            for dep_id in dependents {
                if let Some(task) = inner.tasks.get_mut(dep_id.as_str()) {
                    task.status = TaskStatus::Failed;
                    newly_failed.push(dep_id.clone());
                    // Recursively cascade to this task's dependents
                    queue.push_back(dep_id);
                }
            }
        }

        if !newly_failed.is_empty() {
            tracing::info!(
                target: "jfc::tasks",
                failed_id,
                cascaded_count = newly_failed.len(),
                "cascade_failure: propagated failure"
            );
            self.persist(&inner);
        }
        newly_failed
    }

    /// Create a replan task when a task fails. Returns the new task if created.
    pub fn create_replan_task(&self, failed_id: &str) -> Option<Task> {
        let inner = self.inner.lock().unwrap();
        let failed = inner.tasks.get(failed_id)?;
        let subject = format!("Diagnose + replan: {}", failed.subject);
        let description = format!(
            "Task `{}` ('{}') failed. Investigate root cause and create revised subtasks.\n\
             Original description: {}",
            failed.id, failed.subject, failed.description
        );
        let parent = failed.id.clone();
        drop(inner);

        match self.create(subject, description, None, Vec::<TaskId>::new()) {
            Ok(mut task) => {
                // Set parent_id to the failed task and kind=decision
                let patch = TaskPatch {
                    parent_id: Some(parent),
                    kind: Some(TaskKind::Decision),
                    ..Default::default()
                };
                if let Ok(updated) = self.update(task.id.as_str(), patch) {
                    task = updated;
                }
                tracing::info!(
                    target: "jfc::tasks",
                    failed_id,
                    replan_id = %task.id,
                    "create_replan_task: created replan task"
                );
                Some(task)
            }
            Err(e) => {
                tracing::warn!(
                    target: "jfc::tasks",
                    failed_id,
                    error = %e,
                    "create_replan_task: failed to create"
                );
                None
            }
        }
    }

    /// Validate the task graph for health issues. Returns a structured report.
    pub fn validate(&self) -> TaskValidation {
        let inner = self.inner.lock().unwrap();
        let tasks: Vec<&Task> = inner
            .tasks
            .values()
            .filter(|t| t.status != TaskStatus::Deleted)
            .collect();

        let completed_ids: BTreeSet<&TaskId> = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| &t.id)
            .collect();
        let failed_ids: BTreeSet<&TaskId> = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Failed)
            .map(|t| &t.id)
            .collect();

        let mut orphaned = Vec::new();
        let mut blocked_forever = Vec::new();
        let mut no_verification = Vec::new();
        let mut duplicate_subjects = Vec::new();
        let mut parallelization_opportunities = Vec::new();

        // Detect orphaned tasks (parent_id points to non-existent task)
        for t in &tasks {
            if let Some(ref pid) = t.parent_id {
                if !inner.tasks.contains_key(pid.as_str()) {
                    orphaned.push(t.id.clone());
                }
            }
        }

        // Detect tasks blocked forever (all blockers are failed/deleted)
        for t in &tasks {
            if t.status == TaskStatus::Pending && !t.blocked_by.is_empty() {
                let all_blockers_dead = t
                    .blocked_by
                    .iter()
                    .all(|dep| failed_ids.contains(dep) || !inner.tasks.contains_key(dep.as_str()));
                if all_blockers_dead {
                    blocked_forever.push(t.id.clone());
                }
            }
        }

        // Detect tasks without verification path
        for t in &tasks {
            if t.status != TaskStatus::Completed
                && t.status != TaskStatus::Deleted
                && t.acceptance_criteria.is_none()
                && t.verification_command.is_none()
                && !matches!(t.kind, Some(TaskKind::Decision) | Some(TaskKind::Milestone))
            {
                no_verification.push(t.id.clone());
            }
        }

        // Detect duplicate subjects
        let mut subject_counts: HashMap<&str, Vec<&TaskId>> = HashMap::new();
        for t in &tasks {
            if t.status != TaskStatus::Completed {
                subject_counts
                    .entry(t.subject.as_str())
                    .or_default()
                    .push(&t.id);
            }
        }
        for (subject, ids) in &subject_counts {
            if ids.len() > 1 {
                duplicate_subjects.push(format!(
                    "'{}' used by: {}",
                    subject,
                    ids.iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }

        // Detect parallelization opportunities: sequential tasks with no shared deps
        let pending: Vec<&Task> = tasks
            .iter()
            .filter(|t| t.status == TaskStatus::Pending)
            .copied()
            .collect();
        for (i, a) in pending.iter().enumerate() {
            for b in pending.iter().skip(i + 1) {
                // If a blocks b but they share no common blockers, they might be parallelizable
                if a.blocked_by == b.blocked_by
                    && !a.blocked_by.is_empty()
                    && !b.blocks.contains(&a.id)
                    && !a.blocks.contains(&b.id)
                {
                    parallelization_opportunities.push(format!(
                        "{} and {} share the same blockers — could run in parallel",
                        a.id, b.id
                    ));
                }
            }
        }

        TaskValidation {
            orphaned_tasks: orphaned,
            blocked_forever,
            no_verification_path: no_verification,
            duplicate_subjects,
            parallelization_opportunities,
            total_tasks: tasks.len(),
            pending_count: tasks
                .iter()
                .filter(|t| t.status == TaskStatus::Pending)
                .count(),
            in_progress_count: tasks
                .iter()
                .filter(|t| t.status == TaskStatus::InProgress)
                .count(),
            completed_count: completed_ids.len(),
            failed_count: failed_ids.len(),
        }
    }
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
        let expected_blocks: BTreeSet<TaskId> = std::iter::once(t2.id.clone()).collect();
        let expected_blocked_by: BTreeSet<TaskId> = std::iter::once(t1.id.clone()).collect();
        assert_eq!(t1_after.blocks, expected_blocks);
        assert_eq!(t2.blocked_by, expected_blocked_by);
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

    // Regression: when team mode activates, the leader's previously-created
    // session tasks must survive. The old code blew away the active store
    // and every subsequent /TaskUpdate hit "unknown task id t35..." because
    // the team store had never seen those rows.
    #[test]
    fn migrate_from_copies_tasks_across_stores_normal() {
        let src = TaskStore::in_memory();
        src.create("a".into(), "d1".into(), None, Vec::<TaskId>::new())
            .unwrap();
        src.create("b".into(), "d2".into(), None, Vec::<TaskId>::new())
            .unwrap();
        src.create("c".into(), "d3".into(), None, Vec::<TaskId>::new())
            .unwrap();

        let dst = TaskStore::in_memory();
        let copied = dst.migrate_from(&src);
        assert_eq!(copied, 3);
        assert!(dst.get(&TaskId::from("t1")).is_some());
        assert!(dst.get(&TaskId::from("t2")).is_some());
        assert!(dst.get(&TaskId::from("t3")).is_some());

        // Updates on the new store must succeed — proves IDs landed
        // intact, not just structurally.
        let updated = dst
            .update(
                &TaskId::from("t2"),
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .expect("update preserved task id");
        assert_eq!(updated.status, TaskStatus::Completed);
    }

    // Robust: migrating into a store that already holds the same ids
    // keeps the destination's version (so a team file the user
    // hand-edited isn't clobbered by a stale session task).
    #[test]
    fn migrate_from_destination_wins_on_id_collision_robust() {
        let src = TaskStore::in_memory();
        src.create("from src".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();

        let dst = TaskStore::in_memory();
        dst.create("from dst".into(), "kept".into(), None, Vec::<TaskId>::new())
            .unwrap();

        let copied = dst.migrate_from(&src);
        assert_eq!(copied, 0, "id collision must be a no-op");
        let kept = dst.get(&TaskId::from("t1")).expect("dst task present");
        assert_eq!(kept.subject, "from dst");
    }

    // Robust: empty source is a no-op.
    #[test]
    fn migrate_from_empty_source_is_noop_robust() {
        let src = TaskStore::in_memory();
        let dst = TaskStore::in_memory();
        assert_eq!(dst.migrate_from(&src), 0);
    }

    // Normal: a store re-reads the backing file when an external process
    // (a detached background worker) has written to it since the last load.
    #[test]
    fn reload_if_changed_picks_up_external_write_normal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("session.json");
        // Writer process: create a task and persist it.
        let writer = TaskStore {
            inner: Mutex::new(TaskStoreInner::default()),
            path: path.clone(),
            disk_mtime: Mutex::new(None),
        };
        writer
            .create(
                "written externally".into(),
                "".into(),
                None,
                Vec::<TaskId>::new(),
            )
            .unwrap();

        // Reader handle: opened before any further writes, sees nothing yet.
        let reader = TaskStore {
            inner: Mutex::new(TaskStoreInner::default()),
            path: path.clone(),
            disk_mtime: Mutex::new(None),
        };
        // mtime starts unset, so the first reload always pulls the file in.
        assert!(reader.reload_if_changed());
        assert_eq!(reader.list(DeletedFilter::Exclude).len(), 1);
        // Second call with no external change is a cheap no-op.
        assert!(!reader.reload_if_changed());

        // External writer adds another task; the reader picks it up.
        // Force a distinct mtime — some filesystems have coarse timestamps.
        std::thread::sleep(std::time::Duration::from_millis(10));
        writer
            .create(
                "second external".into(),
                "".into(),
                None,
                Vec::<TaskId>::new(),
            )
            .unwrap();
        assert!(reader.reload_if_changed());
        assert_eq!(reader.list(DeletedFilter::Exclude).len(), 2);
    }

    // Robust: an in-memory store (empty path) never reports a reload.
    #[test]
    fn reload_if_changed_in_memory_is_noop_robust() {
        let store = TaskStore::in_memory();
        store
            .create("local".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        assert!(!store.reload_if_changed());
        assert_eq!(store.list(DeletedFilter::Exclude).len(), 1);
    }

    // Regression: next_id advances to the max of (dst, src) so future
    // creates don't clash with migrated ids.
    #[test]
    fn migrate_from_advances_next_id_robust() {
        let src = TaskStore::in_memory();
        for _ in 0..5 {
            src.create("x".into(), "".into(), None, Vec::<TaskId>::new())
                .unwrap();
        }
        // src is now at next_id=5
        let dst = TaskStore::in_memory();
        dst.migrate_from(&src);

        let new_task = dst
            .create("new".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        // Must NOT collide with any of t1..t5
        assert_eq!(new_task.id, "t6");
    }
}
