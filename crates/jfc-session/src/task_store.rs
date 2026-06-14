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

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Serialize};

pub use jfc_core::{
    FactoryMetrics, Task, TaskCounts, TaskError, TaskKind, TaskPatch, TaskRisk, TaskStatus,
    TaskValidation, TodoTaskId as TaskId,
};

pub fn task_stores_dir() -> PathBuf {
    static TASK_STORES_DIR: OnceLock<PathBuf> = OnceLock::new();
    TASK_STORES_DIR
        .get_or_init(|| {
            dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("jfc")
                .join("tasks")
        })
        .clone()
}

pub fn task_store_path(session_id: &str) -> PathBuf {
    task_stores_dir().join(format!("{session_id}.json"))
}

pub fn team_tasks_dir(team_name: &str) -> PathBuf {
    let home = std::env::var_os("JFC_SWARM_HOME_OVERRIDE")
        .map(PathBuf::from)
        .or_else(cached_home_dir)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join(".claude")
        .join("tasks")
        .join(sanitize_path_component(team_name))
}

fn cached_home_dir() -> Option<PathBuf> {
    static HOME_DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    HOME_DIR.get_or_init(dirs::home_dir).clone()
}

pub fn team_task_store_path(team_name: &str) -> PathBuf {
    team_tasks_dir(team_name).join("tasks.json")
}

/// Returns the project-level task store path: `<git_root>/.jfc/tasks.json`.
/// Falls back to `./.jfc/tasks.json` if no git root is provided.
pub fn project_task_store_path(git_root: Option<&std::path::Path>) -> PathBuf {
    let root = git_root
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    root.join(".jfc").join("tasks.json")
}

fn sanitize_path_component(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .to_lowercase()
}

fn floor_char_boundary(value: &str, index: usize) -> usize {
    let mut boundary = index.min(value.len());
    while !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    boundary
}

fn is_legacy_placeholder_task(task: &Task) -> bool {
    task.subject.trim().eq_ignore_ascii_case("subj")
        && task.description.trim().eq_ignore_ascii_case("desc")
}

/// Age-based retention for terminal (completed/failed/deleted) tasks. Terminal
/// rows whose `created_at_ms` is older than this are dropped on store open and
/// after external reloads. This handles the common "one long session created
/// 100 tasks" case without a count cap evicting that session's own history,
/// while still bounding growth from stale prior sessions.
const TERMINAL_TASK_RETENTION_MS: u64 = 7 * 24 * 60 * 60 * 1000;

/// Hard upper bound on retained terminal tasks regardless of age. Acts as a
/// safety net so a single session can't bloat the file without limit; the
/// most-recent N are kept once the age window has been applied.
const MAX_TERMINAL_TASKS: usize = 200;

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

/// Milliseconds since the UNIX epoch.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Outcome of [`TaskStore::recover_from_failure`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FailureRecovery {
    /// The failed task id wasn't found.
    Unknown,
    /// Transient failure under the attempt budget — task re-queued as Pending.
    Retried {
        task_id: TaskId,
        attempt: u32,
        max_attempts: u32,
    },
    /// Hard failure — task marked Failed, a replan task created, and the
    /// listed dependents rerouted to block on the replan (preserved, not
    /// destroyed).
    Replanned {
        failed_id: TaskId,
        replan_id: TaskId,
        rerouted: Vec<TaskId>,
        attempts: u32,
    },
}

/// Heuristic: does this failure look transient (worth a retry) rather than a
/// deterministic logic error? Transient signals are network/timeout/lock/
/// rate-limit/IO classes that commonly resolve on a fresh attempt. A genuine
/// compile error or assertion failure is NOT transient — retrying just burns
/// budget, so we fall straight through to replan.
pub fn is_transient_failure(error: &str) -> bool {
    let e = error.to_ascii_lowercase();
    const TRANSIENT: &[&str] = &[
        "timeout",
        "timed out",
        "connection",
        "network",
        "rate limit",
        "rate-limit",
        "429",
        "503",
        "502",
        "504",
        "temporarily",
        "deadlock",
        "lock",
        "resource temporarily unavailable",
        "broken pipe",
        "reset by peer",
        "try again",
        "overloaded",
        "stream error",
        "interrupted",
    ];
    // Deterministic-failure signals override: if it looks like a real logic /
    // build error, never treat it as transient.
    const HARD: &[&str] = &[
        "error[e", // rustc error code
        "compile",
        "assertion failed",
        "assertion `",
        "panicked at",
        "type mismatch",
        "cannot find",
        "unresolved import",
        "test failed",
        "verification failed",
    ];
    if HARD.iter().any(|h| e.contains(h)) {
        return false;
    }
    TRANSIENT.iter().any(|t| e.contains(t))
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
        let path = task_store_path(session_id);
        tracing::info!(
            target: "jfc::tasks",
            session_id,
            path = %path.display(),
            "TaskStore::open"
        );
        let inner = Self::load_inner(&path);
        let disk_mtime = Self::file_mtime(&path);
        let store = Arc::new(Self {
            inner: Mutex::new(inner),
            path,
            disk_mtime: Mutex::new(disk_mtime),
        });
        store.delete_legacy_placeholders();
        store.prune_terminal_tasks(MAX_TERMINAL_TASKS);
        store
    }

    /// Open the task store shared by a swarm team. This intentionally uses
    /// Claude-compatible swarm storage (`~/.claude/tasks/<team>/tasks.json`)
    /// so the leader and in-process teammates coordinate over one list.
    pub fn open_team(team_name: &str) -> Arc<Self> {
        let path = team_task_store_path(team_name);
        tracing::info!(
            target: "jfc::tasks",
            team_name,
            path = %path.display(),
            "TaskStore::open_team"
        );
        let disk_mtime = Self::file_mtime(&path);
        let store = Arc::new(Self {
            inner: Mutex::new(Self::load_inner(&path)),
            path,
            disk_mtime: Mutex::new(disk_mtime),
        });
        store.delete_legacy_placeholders();
        store.prune_terminal_tasks(MAX_TERMINAL_TASKS);
        store
    }

    /// In-memory store (no persistence) — used in tests.
    pub fn in_memory() -> Arc<Self> {
        tracing::debug!(target: "jfc::tasks", "TaskStore::in_memory");
        Arc::new(Self::default())
    }

    /// Backing file path for this store (empty for in-memory stores). Callers
    /// derive the sibling history-log path from this via
    /// `jfc_session::history_path_for`.
    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    /// Open or create the **project-level** task store at
    /// `<git_root>/.jfc/tasks.json`. This is the primary persistence layer
    /// that survives across ALL sessions for the same project. Unlike
    /// per-session stores, every `jfc` instance in the same repo shares this
    /// file. Falls back to `./.jfc/tasks.json` if no git root is provided.
    pub fn open_project(git_root: Option<&std::path::Path>) -> Arc<Self> {
        let path = project_task_store_path(git_root);
        tracing::info!(
            target: "jfc::tasks",
            path = %path.display(),
            "TaskStore::open_project"
        );
        let inner = Self::load_inner(&path);
        let disk_mtime = Self::file_mtime(&path);
        let store = Arc::new(Self {
            inner: Mutex::new(inner),
            path,
            disk_mtime: Mutex::new(disk_mtime),
        });
        store.delete_legacy_placeholders();
        store.prune_terminal_tasks(MAX_TERMINAL_TASKS);
        store
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

    /// Tombstone legacy placeholder tasks produced by earlier tool calls.
    ///
    /// The model occasionally created tasks with the fixture-shaped
    /// subject/description pair `subj`/`desc`. Those rows are not useful work,
    /// but because task stores persist across resumes they can pollute the
    /// pinned task queue indefinitely. This only removes the exact placeholder
    /// pair and leaves all other tasks untouched.
    pub fn delete_legacy_placeholders(&self) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let deleted = Self::delete_legacy_placeholders_from_inner(&mut inner);
        if deleted == 0 {
            return 0;
        }

        self.persist(&inner);
        tracing::warn!(
            target: "jfc::tasks",
            deleted,
            path = %self.path.display(),
            "deleted legacy placeholder tasks"
        );
        deleted
    }

    fn delete_legacy_placeholders_from_inner(inner: &mut TaskStoreInner) -> usize {
        // Hard-remove the fixture-shaped `subj`/`desc` rows entirely — they
        // carry zero audit value. The previous behavior only *tombstoned*
        // them (status=Deleted), and since it skipped already-deleted rows,
        // those tombstones accumulated in tasks.json across every resume
        // (50KB+ of dead placeholder rows observed in practice). Match
        // regardless of current status so existing tombstones are purged too.
        let placeholder_ids: BTreeSet<TaskId> = inner
            .tasks
            .iter()
            .filter(|(_, task)| is_legacy_placeholder_task(task))
            .map(|(id, _)| id.clone())
            .collect();

        if placeholder_ids.is_empty() {
            return 0;
        }

        for id in &placeholder_ids {
            inner.tasks.remove(id);
        }

        // Drop dependency edges that pointed at the removed rows so no
        // surviving task is left blocked by / blocking a vanished id.
        for task in inner.tasks.values_mut() {
            task.blocks.retain(|id| !placeholder_ids.contains(id));
            task.blocked_by.retain(|id| !placeholder_ids.contains(id));
        }

        placeholder_ids.len()
    }

    /// Hard-remove stale terminal tasks while keeping recent terminal rows for
    /// session history. Active tasks are never pruned. Two passes: an age
    /// window (`TERMINAL_TASK_RETENTION_MS`) then a count cap (`max_terminal`).
    ///
    /// Pruned rows are not lost: each is distilled into a `TaskHistoryRecord`
    /// and appended to the sibling `*-history.jsonl` archive before removal, so
    /// the durable "everything we've worked on" log survives even as the hot
    /// working set stays bounded.
    pub fn prune_terminal_tasks(&self, max_terminal: usize) -> usize {
        let mut inner = self.inner.lock().unwrap();
        let now_ms = now_ms();
        let archived = Self::prune_terminal_tasks_from_inner(&mut inner, max_terminal, now_ms);
        let pruned = archived.len();
        if pruned == 0 {
            return 0;
        }

        self.persist(&inner);
        drop(inner);
        self.archive_history(&archived, now_ms);
        tracing::info!(
            target: "jfc::tasks",
            pruned,
            retained = max_terminal,
            retention_ms = TERMINAL_TASK_RETENTION_MS,
            path = %self.path.display(),
            "pruned old terminal tasks (archived to history)"
        );
        pruned
    }

    /// Append distilled records for the just-pruned tasks to the history log.
    /// Best-effort: a missing/unwritable archive never blocks pruning.
    fn archive_history(&self, pruned: &[Task], archived_at_ms: u64) {
        if pruned.is_empty() {
            return;
        }
        let history_path = crate::task_history::history_path_for(&self.path);
        if history_path.as_os_str().is_empty() {
            return; // in-memory store — nothing durable to archive to
        }
        let records: Vec<crate::TaskHistoryRecord> = pruned
            .iter()
            .map(|task| crate::TaskHistoryRecord::from_task(task, archived_at_ms))
            .collect();
        crate::task_history::append_records(&history_path, &records);
    }

    /// Compute the prune set and remove it, returning the *removed task
    /// values* (not just ids) so callers can distill them into history before
    /// they're gone.
    fn prune_terminal_tasks_from_inner(
        inner: &mut TaskStoreInner,
        max_terminal: usize,
        now_ms: u64,
    ) -> Vec<Task> {
        let is_terminal = |task: &Task| {
            matches!(
                task.status,
                TaskStatus::Completed | TaskStatus::Failed | TaskStatus::Deleted
            )
        };

        // Pass 1: age window — drop terminal rows older than the retention
        // horizon. `saturating_sub` guards against clock skew / future stamps.
        let cutoff = now_ms.saturating_sub(TERMINAL_TASK_RETENTION_MS);
        let mut prune_ids: BTreeSet<TaskId> = inner
            .tasks
            .iter()
            .filter(|(_, task)| is_terminal(task) && task.created_at_ms < cutoff)
            .map(|(id, _)| id.clone())
            .collect();

        // Pass 2: count cap over the terminal rows that survived the age
        // window — keep the most-recent `max_terminal`, drop the rest.
        let mut surviving_terminal: Vec<(TaskId, u64)> = inner
            .tasks
            .iter()
            .filter(|(id, task)| is_terminal(task) && !prune_ids.contains(*id))
            .map(|(id, task)| (id.clone(), task.created_at_ms))
            .collect();

        if surviving_terminal.len() > max_terminal {
            surviving_terminal.sort_by(|(left_id, left_created), (right_id, right_created)| {
                right_created
                    .cmp(left_created)
                    .then_with(|| right_id.as_str().cmp(left_id.as_str()))
            });
            for (id, _) in surviving_terminal.into_iter().skip(max_terminal) {
                prune_ids.insert(id);
            }
        }

        if prune_ids.is_empty() {
            return Vec::new();
        }

        let mut removed = Vec::with_capacity(prune_ids.len());
        for id in &prune_ids {
            if let Some(task) = inner.tasks.remove(id) {
                removed.push(task);
            }
        }

        for task in inner.tasks.values_mut() {
            task.blocks.retain(|id| !prune_ids.contains(id));
            task.blocked_by.retain(|id| !prune_ids.contains(id));
        }

        removed
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
        // Hold the cross-process lock for the whole load → prune → write-back
        // cycle so a concurrent writer can't land between our read and our
        // persist and get silently overwritten (lost update).
        let _file_lock = self.acquire_file_lock();
        let now_ms = now_ms();
        let mut fresh = Self::load_inner(&self.path);
        // Re-read mtime under the lock: it names exactly the revision we
        // loaded, not whatever was on disk before we blocked on the lock.
        let loaded_mtime = Self::file_mtime(&self.path);
        let deleted = Self::delete_legacy_placeholders_from_inner(&mut fresh);
        let archived =
            Self::prune_terminal_tasks_from_inner(&mut fresh, MAX_TERMINAL_TASKS, now_ms);
        let pruned = archived.len();
        let mut inner = self.inner.lock().unwrap();
        *inner = fresh;
        drop(inner);
        if deleted > 0 || pruned > 0 {
            if let Ok(inner) = self.inner.lock() {
                self.persist_unlocked(&inner);
            }
            self.archive_history(&archived, now_ms);
            tracing::warn!(
                target: "jfc::tasks",
                deleted,
                pruned,
                path = %self.path.display(),
                "compacted task store after external reload (pruned rows archived to history)"
            );
        } else {
            *self.disk_mtime.lock().unwrap() = loaded_mtime;
        }
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

    /// Cross-process advisory lock guarding read-modify-write cycles on the
    /// backing file (UI process vs detached background workers). Best-effort:
    /// returns `None` when the lock file can't be created, in which case the
    /// caller proceeds unlocked (the pre-lock behavior). The lock releases
    /// when the returned handle drops.
    fn acquire_file_lock(&self) -> Option<std::fs::File> {
        if self.path.as_os_str().is_empty() {
            return None;
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(self.path.with_extension("lock"))
            .ok()?;
        fs2::FileExt::lock_exclusive(&lock).ok()?;
        Some(lock)
    }

    fn persist(&self, inner: &TaskStoreInner) {
        let _file_lock = self.acquire_file_lock();
        self.persist_unlocked(inner);
    }

    /// Write the store to disk. Caller must hold the cross-process file lock
    /// (or accept the pre-lock lost-update semantics).
    fn persist_unlocked(&self, inner: &TaskStoreInner) {
        if self.path.as_os_str().is_empty() {
            return;
        }
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        if let Ok(json) = serde_json::to_string_pretty(inner) {
            // Unique temp name (pid + nanos) so two processes persisting
            // concurrently don't collide on a shared `.tmp` sibling and
            // rename each other's partial write into place.
            let nonce = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let tmp = self
                .path
                .with_extension(format!("tmp-{}-{nonce}", std::process::id()));
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
            &subject[..floor_char_boundary(&subject, 80)]
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
            tags: Vec::new(),
            priority: None,
            effort: None,
            model: None,
            attempt_count: 0,
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
        if let Some(tags) = patch.tags {
            task.tags = tags;
        }
        if let Some(p) = patch.priority {
            task.priority = Some(p);
        }
        if let Some(e) = patch.effort {
            task.effort = Some(e);
        }
        if let Some(m) = patch.model {
            task.model = Some(m);
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

    /// All tasks, sorted by priority (lower = higher priority), then creation order.
    /// Excludes Deleted unless asked.
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
        out.sort_by(|a, b| {
            let pa = a.priority.unwrap_or(5);
            let pb = b.priority.unwrap_or(5);
            pa.cmp(&pb).then_with(|| {
                let id_a =
                    a.id.as_str()
                        .strip_prefix('t')
                        .and_then(|n| n.parse::<u64>().ok())
                        .unwrap_or(0);
                let id_b =
                    b.id.as_str()
                        .strip_prefix('t')
                        .and_then(|n| n.parse::<u64>().ok())
                        .unwrap_or(0);
                id_a.cmp(&id_b)
            })
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
    /// all completed. Prefers higher-priority (lower number) tasks.
    pub fn claim_next_available(&self, owner: &str) -> Option<Task> {
        let mut inner = self.inner.lock().unwrap();
        let completed = inner
            .tasks
            .values()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect::<BTreeSet<_>>();
        let mut ids = inner.tasks.keys().cloned().collect::<Vec<_>>();
        ids.sort_by(|a, b| {
            let task_a = inner.tasks.get(a.as_str());
            let task_b = inner.tasks.get(b.as_str());
            let pa = task_a.and_then(|t| t.priority).unwrap_or(5);
            let pb = task_b.and_then(|t| t.priority).unwrap_or(5);
            pa.cmp(&pb).then_with(|| {
                let id_a = a
                    .as_str()
                    .strip_prefix('t')
                    .and_then(|n| n.parse::<u64>().ok())
                    .unwrap_or(0);
                let id_b = b
                    .as_str()
                    .strip_prefix('t')
                    .and_then(|n| n.parse::<u64>().ok())
                    .unwrap_or(0);
                id_a.cmp(&id_b)
            })
        });
        let now = now_ms();
        let claim_id = ids.into_iter().find(|id| {
            inner.tasks.get(id.as_str()).is_some_and(|task| {
                task.status == TaskStatus::Pending
                    && task.owner.as_deref().unwrap_or("").is_empty()
                    && task.blocked_by.iter().all(|dep| completed.contains(dep))
                    && !in_retry_backoff(task, now)
            })
        })?;
        let task = inner.tasks.get_mut(claim_id.as_str())?;
        task.owner = Some(owner.to_owned());
        task.status = TaskStatus::InProgress;
        clear_retry_after(task); // Forget: a fresh attempt starts a new curve.
        let task = task.clone();
        self.persist(&inner);
        Some(task)
    }

    /// The ready frontier: every pending, unowned task whose blockers are all
    /// completed — the set that is safe to dispatch *in parallel right now*.
    /// This is the batch form of [`Self::claim_next_available`], for schedulers
    /// that fan work across the whole ready set instead of claiming one task at
    /// a time (Terraform `internal/dag/walk.go` ready-set walk). Sorted by
    /// priority (lower first), then creation order, so the ordering matches
    /// `claim_next_available`'s single-task pick.
    pub fn ready_frontier(&self) -> Vec<Task> {
        let inner = self.inner.lock().unwrap();
        let completed: BTreeSet<TaskId> = inner
            .tasks
            .values()
            .filter(|t| t.status == TaskStatus::Completed)
            .map(|t| t.id.clone())
            .collect();
        let now = now_ms();
        let mut ready: Vec<Task> = inner
            .tasks
            .values()
            .filter(|t| {
                t.status == TaskStatus::Pending
                    && t.owner.as_deref().unwrap_or("").is_empty()
                    && t.blocked_by.iter().all(|dep| completed.contains(dep))
                    && !in_retry_backoff(t, now)
            })
            .cloned()
            .collect();
        ready.sort_by(cmp_priority_then_creation);
        tracing::trace!(
            target: "jfc::tasks",
            count = ready.len(),
            "TaskStore::ready_frontier"
        );
        ready
    }

    /// Reset tasks stuck `InProgress` under `owner` back to Pending + unowned
    /// so they can be re-claimed. The factory uses this when it is fully idle
    /// (no live agent, no active turn) yet a task it claimed never reached a
    /// terminal state — e.g. a turn ended without TaskDone, or a crash left
    /// the claim dangling. Returns the ids that were re-queued.
    ///
    /// Only touches tasks whose `owner == owner`, so genuinely in-flight work
    /// owned by a live subagent (a different owner string) is never disturbed.
    pub fn requeue_stuck(&self, owner: &str) -> Vec<TaskId> {
        let mut inner = self.inner.lock().unwrap();
        let mut requeued = Vec::new();
        for task in inner.tasks.values_mut() {
            if task.status == TaskStatus::InProgress && task.owner.as_deref() == Some(owner) {
                task.status = TaskStatus::Pending;
                task.owner = None;
                requeued.push(task.id.clone());
            }
        }
        if !requeued.is_empty() {
            tracing::info!(
                target: "jfc::tasks",
                owner,
                count = requeued.len(),
                "TaskStore::requeue_stuck reset dangling in_progress tasks"
            );
            self.persist(&inner);
        }
        requeued
    }

    /// Counts by status — used by the UI overflow summary.
    pub fn counts(&self) -> TaskCounts {
        tracing::trace!(target: "jfc::tasks", "TaskStore::counts");
        let inner = self.inner.lock().unwrap();
        let mut c = TaskCounts::default();
        for t in inner.tasks.values() {
            match t.status {
                // Queued + Blocked are unstarted open work — count with Pending.
                TaskStatus::Pending | TaskStatus::Queued | TaskStatus::Blocked => c.pending += 1,
                TaskStatus::InProgress => c.in_progress += 1,
                TaskStatus::Completed => c.completed += 1,
                TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Deleted => {}
            }
        }
        c
    }

    /// List all tasks (excluding deleted). Convenience for plan verification.
    pub fn list_all(&self) -> Vec<Task> {
        self.list(DeletedFilter::Exclude)
    }

    /// Compute factory throughput + quality metrics (Morescient GAI,
    /// arXiv:2406.04710). Walks the task store once and aggregates counts
    /// the scheduler and `/factory` view surface.
    pub fn factory_metrics(&self) -> FactoryMetrics {
        let inner = self.inner.lock().unwrap();
        self.factory_metrics_inner(&inner)
    }

    /// Compute metrics from an already-held inner guard (avoids re-locking
    /// from callers like `recover_from_failure` that hold the mutex).
    fn factory_metrics_inner(&self, inner: &TaskStoreInner) -> FactoryMetrics {
        let mut m = FactoryMetrics::default();
        for t in inner.tasks.values() {
            match t.status {
                TaskStatus::Pending | TaskStatus::Queued | TaskStatus::Blocked => m.pending += 1,
                TaskStatus::InProgress => m.in_progress += 1,
                TaskStatus::Completed => m.completed += 1,
                TaskStatus::Failed => m.failed += 1,
                // Cancelled/Deleted aren't throughput — skip (Cancelled is a
                // user abort, not a factory failure).
                TaskStatus::Cancelled | TaskStatus::Deleted => continue,
            }
            if t.tags.contains(&"replan".to_string()) {
                m.replan_tasks += 1;
            }
            if t.attempt_count > 0 {
                m.retried_tasks += 1;
                m.total_attempts += t.attempt_count;
            }
            if t.attempt_count > 1 {
                m.multi_attempt_tasks += 1;
            }
        }
        m
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

    /// Default attempt budget before a transient failure becomes a hard fail.
    pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

    /// Proactively recover from a task failure (Agentic Task Graph, arXiv:
    /// 2605.11951): instead of blindly cascading the whole dependent subtree
    /// to `Failed`, classify the failure and preserve recoverable work.
    ///
    /// - **Bounded retry**: increments `attempt_count`. If the failure looks
    ///   transient (see [`is_transient_failure`]) and attempts are under
    ///   `max_attempts`, the task is re-queued as `Pending` for another go —
    ///   no dependents are touched.
    /// - **Hard fail + non-destructive reroute**: once the budget is spent (or
    ///   the failure is non-transient), the task is marked `Failed`, a replan
    ///   task is created, and direct dependents are **rerouted** to block on
    ///   the replan task (added to their `blocked_by`) rather than being
    ///   destroyed. The subtree is preserved and unblocks automatically once
    ///   the replan completes.
    ///
    /// Returns a [`FailureRecovery`] describing what happened so the caller
    /// can craft the right system-reminder.
    pub fn recover_from_failure(&self, failed_id: &str, error: &str) -> FailureRecovery {
        let max_attempts = Self::DEFAULT_MAX_ATTEMPTS;
        let mut inner = self.inner.lock().unwrap();

        let Some(task) = inner.tasks.get_mut(failed_id) else {
            return FailureRecovery::Unknown;
        };
        task.attempt_count += 1;
        let attempts = task.attempt_count;
        let transient = is_transient_failure(error);

        // Retry path: transient + budget remaining → re-queue, touch nothing
        // else. This is the proactive bit — most transient failures (flaky
        // test, network blip, lock contention) resolve on a fresh attempt.
        if transient && attempts < max_attempts {
            task.status = TaskStatus::Pending;
            task.owner = None;
            // Exponential backoff: don't let the claim paths re-dispatch this
            // task until the deadline passes, so a flaky task can't hot-loop.
            let backoff_ms = retry_backoff_ms(attempts);
            set_retry_after(task, now_ms() + backoff_ms);
            let id = task.id.clone();
            self.persist(&inner);
            tracing::info!(
                target: "jfc::tasks",
                failed_id, attempts, max_attempts, backoff_ms,
                "recover_from_failure: transient failure, re-queued for retry after backoff"
            );
            return FailureRecovery::Retried {
                task_id: id,
                attempt: attempts,
                max_attempts,
            };
        }

        // Hard-fail path: mark the task Failed.
        task.status = TaskStatus::Failed;
        let subject = task.subject.clone();
        let description = task.description.clone();
        let failed_tid = task.id.clone();

        // Find direct, recoverable dependents (blocked_by contains failed_id
        // and not already terminal). These are REROUTED, not destroyed.
        let dependents: Vec<TaskId> = inner
            .tasks
            .values()
            .filter(|t| {
                t.blocked_by.contains(&failed_tid)
                    && !matches!(
                        t.status,
                        TaskStatus::Failed | TaskStatus::Deleted | TaskStatus::Completed
                    )
            })
            .map(|t| t.id.clone())
            .collect();

        // Create the replan task inline (we hold the lock; build it directly).
        // Compute factory metrics for the replan prompt context.
        let fm = self.factory_metrics_inner(&inner);
        let replan_id = TaskId::new(format!("{}-replan", failed_tid.as_str()));
        let replan = Task {
            id: replan_id.clone(),
            subject: format!("Diagnose + replan: {subject}"),
            description: format!(
                "Task `{failed_tid}` ('{subject}') failed after {attempts} attempt(s): {error}\n\n\
                 Investigate the root cause and create revised subtasks. {} dependent task(s) \
                 are blocked on this replan and will unblock automatically when it completes.\n\n\
                 Factory health: {}/{} completed, success rate {}, rework ratio {:.0}%.\n\n\
                 Original description: {description}",
                dependents.len(),
                fm.completed,
                fm.completed + fm.failed,
                fm.success_rate()
                    .map(|r| format!("{:.0}%", r * 100.0))
                    .unwrap_or_else(|| "—".to_string()),
                fm.rework_ratio() * 100.0,
            ),
            active_form: None,
            status: TaskStatus::Pending,
            owner: None,
            blocks: dependents.iter().cloned().collect(),
            blocked_by: BTreeSet::new(),
            metadata: None,
            created_at_ms: now_ms(),
            acceptance_criteria: None,
            verification_command: None,
            risk: None,
            parent_id: Some(failed_tid.clone()),
            kind: Some(TaskKind::Decision),
            tags: vec!["replan".to_string()],
            priority: None,
            effort: None,
            model: None,
            attempt_count: 0,
        };
        inner.tasks.insert(replan_id.clone(), replan);

        // Reroute: each recoverable dependent now blocks on the replan instead
        // of (only) the failed task. It stays Pending — preserved, not failed.
        for dep_id in &dependents {
            if let Some(dep) = inner.tasks.get_mut(dep_id.as_str()) {
                dep.blocked_by.insert(replan_id.clone());
            }
        }

        self.persist(&inner);
        tracing::info!(
            target: "jfc::tasks",
            failed_id, attempts,
            rerouted = dependents.len(),
            replan_id = %replan_id,
            "recover_from_failure: hard fail, created replan + rerouted dependents"
        );
        FailureRecovery::Replanned {
            failed_id: failed_tid,
            replan_id,
            rerouted: dependents,
            attempts,
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
            if let Some(ref pid) = t.parent_id
                && !inner.tasks.contains_key(pid.as_str())
            {
                orphaned.push(t.id.clone());
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

        // Ready frontier: pending, unowned, all blockers completed, and not in
        // retry-backoff — the set safe to fan out in parallel right now
        // (Terraform ready-set walk).
        let now = now_ms();
        let mut ready: Vec<TaskId> = tasks
            .iter()
            .filter(|t| {
                t.status == TaskStatus::Pending
                    && t.owner.as_deref().unwrap_or("").is_empty()
                    && t.blocked_by.iter().all(|dep| completed_ids.contains(dep))
                    && !in_retry_backoff(t, now)
            })
            .map(|t| t.id.clone())
            .collect();
        ready.sort();

        // Tarjan cycle detection + transitive upstream-failed propagation.
        let dependency_cycles = dependency_cycles(&inner);
        let upstream_failed = upstream_failed_tasks(&inner);

        TaskValidation {
            orphaned_tasks: orphaned,
            blocked_forever,
            no_verification_path: no_verification,
            duplicate_subjects,
            parallelization_opportunities,
            dependency_cycles,
            upstream_failed,
            ready,
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

/// Order tasks by priority (lower number = higher priority, default 5), then
/// by numeric creation order (`t<N>`). Shared by `ready_frontier` so its
/// ordering matches `claim_next_available`'s single-task pick.
fn cmp_priority_then_creation(a: &Task, b: &Task) -> std::cmp::Ordering {
    let pa = a.priority.unwrap_or(5);
    let pb = b.priority.unwrap_or(5);
    let seq = |t: &Task| {
        t.id.as_str()
            .strip_prefix('t')
            .and_then(|n| n.parse::<u64>().ok())
            .unwrap_or(0)
    };
    pa.cmp(&pb).then_with(|| seq(a).cmp(&seq(b)))
}

/// Whether a blocker id refers to a "dead" task — Failed, Cancelled, Deleted, or
/// missing entirely. Matches the existing `blocked_forever` semantics in `validate`.
fn blocker_is_dead(inner: &TaskStoreInner, id: &TaskId) -> bool {
    match inner.tasks.get(id.as_str()) {
        None => true,
        Some(t) => matches!(
            t.status,
            TaskStatus::Failed | TaskStatus::Cancelled | TaskStatus::Deleted
        ),
    }
}

// ── Retry backoff ──────────────────────────────────────────────────────────
//
// When `recover_from_failure` re-queues a transient failure it stamps the task
// with a `retry_after_ms` deadline computed from an exponential curve, and the
// claim paths refuse to re-dispatch the task until that deadline passes. This
// is the per-item exponential backoff half of the Kubernetes workqueue
// discipline — the richer `MaxOf(exp, token-bucket)` form with `NumRequeues`/
// `Forget` lives in `jfc_economy::rate_limiter::RetryRateLimiter` for the
// bounty/subagent layer; the task store only needs the per-item curve, and
// can't depend on `jfc-economy`, so the (short) formula is mirrored here.

/// Base retry delay; doubles per attempt up to [`RETRY_MAX_MS`].
const RETRY_BASE_MS: u64 = 1_000;
/// Ceiling on the retry backoff (1 minute).
const RETRY_MAX_MS: u64 = 60_000;
/// Metadata key holding the epoch-ms deadline before which a re-queued task
/// must not be re-claimed.
const RETRY_AFTER_KEY: &str = "retry_after_ms";

/// `base * 2^(attempt-1)`, saturating, capped at [`RETRY_MAX_MS`]. `attempt`
/// is 1-based (the `attempt_count` value *after* it is incremented), so the
/// first retry waits `base`, the second `2*base`, etc.
fn retry_backoff_ms(attempt: u32) -> u64 {
    let exp = attempt.saturating_sub(1).min(20); // 2^20 already exceeds the cap
    RETRY_BASE_MS.saturating_mul(1u64 << exp).min(RETRY_MAX_MS)
}

/// Stamp a task with its next-retry deadline (epoch ms).
fn set_retry_after(task: &mut Task, deadline_ms: u64) {
    let obj = task.metadata.get_or_insert_with(|| serde_json::json!({}));
    if let Some(map) = obj.as_object_mut() {
        map.insert(RETRY_AFTER_KEY.into(), serde_json::json!(deadline_ms));
    } else {
        *obj = serde_json::json!({ RETRY_AFTER_KEY: deadline_ms });
    }
}

/// Clear a task's retry deadline — the `Forget` half of the contract, called
/// when the task is (re-)claimed so a future failure starts a fresh curve.
fn clear_retry_after(task: &mut Task) {
    if let Some(map) = task.metadata.as_mut().and_then(|m| m.as_object_mut()) {
        map.remove(RETRY_AFTER_KEY);
    }
}

/// Whether `task` is still inside its retry-backoff window as of `now_ms`.
fn in_retry_backoff(task: &Task, now_ms: u64) -> bool {
    task.metadata
        .as_ref()
        .and_then(|m| m.as_object())
        .and_then(|m| m.get(RETRY_AFTER_KEY))
        .and_then(|v| v.as_u64())
        .is_some_and(|deadline| deadline > now_ms)
}

/// Tasks transitively blocked by a dead (Failed/Deleted/missing) task —
/// Terraform's `upstreamFailed`. Fixpoint over `blocked_by`: a non-terminal
/// task is upstream-failed if *any* of its blockers is dead or itself
/// upstream-failed. Only Pending/InProgress tasks are reported (terminal tasks
/// own their state). This is the transitive superset of `blocked_forever`
/// (which requires *all* blockers dead and only the direct case).
fn upstream_failed_tasks(inner: &TaskStoreInner) -> Vec<TaskId> {
    let mut failed: BTreeSet<TaskId> = BTreeSet::new();
    loop {
        let mut changed = false;
        for (id, task) in &inner.tasks {
            if !matches!(task.status, TaskStatus::Pending | TaskStatus::InProgress) {
                continue;
            }
            if failed.contains(id) {
                continue;
            }
            let stuck = task
                .blocked_by
                .iter()
                .any(|dep| blocker_is_dead(inner, dep) || failed.contains(dep));
            if stuck {
                failed.insert(id.clone());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    failed.into_iter().collect()
}

/// Iterative-recursion Tarjan SCC over the `blocked_by` edges of the
/// non-deleted task graph. Returns only components that form a real cycle:
/// any SCC of size > 1, plus any single task that lists itself as a blocker.
/// The per-edge guard in `update()` rejects *new* cycles, but a graph loaded
/// from disk or hand-edited JSON can still contain one — this surfaces it.
/// Mirrors Terraform `internal/dag/tarjan.go`. Task graphs are small (tens of
/// nodes), so recursive descent is safe.
fn dependency_cycles(inner: &TaskStoreInner) -> Vec<Vec<TaskId>> {
    struct Tarjan<'a> {
        inner: &'a TaskStoreInner,
        index: usize,
        indices: HashMap<TaskId, usize>,
        lowlink: HashMap<TaskId, usize>,
        on_stack: HashSet<TaskId>,
        stack: Vec<TaskId>,
        sccs: Vec<Vec<TaskId>>,
    }
    impl Tarjan<'_> {
        fn connect(&mut self, v: &TaskId) {
            self.indices.insert(v.clone(), self.index);
            self.lowlink.insert(v.clone(), self.index);
            self.index += 1;
            self.stack.push(v.clone());
            self.on_stack.insert(v.clone());
            if let Some(task) = self.inner.tasks.get(v.as_str()) {
                for w in &task.blocked_by {
                    let alive = self
                        .inner
                        .tasks
                        .get(w.as_str())
                        .is_some_and(|t| t.status != TaskStatus::Deleted);
                    if !alive {
                        continue;
                    }
                    if !self.indices.contains_key(w) {
                        self.connect(w);
                        let low = self.lowlink[v].min(self.lowlink[w]);
                        self.lowlink.insert(v.clone(), low);
                    } else if self.on_stack.contains(w) {
                        let low = self.lowlink[v].min(self.indices[w]);
                        self.lowlink.insert(v.clone(), low);
                    }
                }
            }
            if self.lowlink[v] == self.indices[v] {
                let mut scc = Vec::new();
                while let Some(w) = self.stack.pop() {
                    self.on_stack.remove(&w);
                    let is_root = &w == v;
                    scc.push(w);
                    if is_root {
                        break;
                    }
                }
                self.sccs.push(scc);
            }
        }
    }

    let mut t = Tarjan {
        inner,
        index: 0,
        indices: HashMap::new(),
        lowlink: HashMap::new(),
        on_stack: HashSet::new(),
        stack: Vec::new(),
        sccs: Vec::new(),
    };
    // Deterministic root order so cycle reporting is stable across runs.
    let mut roots: Vec<TaskId> = inner
        .tasks
        .iter()
        .filter(|(_, task)| task.status != TaskStatus::Deleted)
        .map(|(id, _)| id.clone())
        .collect();
    roots.sort();
    for v in &roots {
        if !t.indices.contains_key(v) {
            t.connect(v);
        }
    }
    t.sccs
        .into_iter()
        .filter(|scc| {
            scc.len() > 1
                || scc.first().is_some_and(|v| {
                    inner
                        .tasks
                        .get(v.as_str())
                        .is_some_and(|task| task.blocked_by.contains(v))
                })
        })
        .map(|mut scc| {
            scc.sort();
            scc
        })
        .collect()
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

    // Normal: the count cap keeps the most-recent terminal rows and never
    // touches active tasks. Timestamps are recent (within the age window) so
    // only the count-cap pass fires.
    #[test]
    fn prune_terminal_tasks_keeps_recent_and_active_normal() {
        let store = TaskStore::in_memory();
        let mut terminal_ids = Vec::new();

        for n in 0..55 {
            let task = store
                .create(
                    format!("done {n}"),
                    "details".into(),
                    None,
                    Vec::<TaskId>::new(),
                )
                .unwrap();
            store
                .update(
                    task.id.as_str(),
                    TaskPatch {
                        status: Some(TaskStatus::Completed),
                        ..Default::default()
                    },
                )
                .unwrap();
            terminal_ids.push(task.id);
        }

        let active = store
            .create(
                "active".into(),
                "details".into(),
                None,
                vec![terminal_ids[0].clone()],
            )
            .unwrap();

        // Recent timestamps spaced 1s apart: index 0 is the oldest, 54 the
        // newest, all comfortably inside the retention window.
        let base = now_ms();
        {
            let mut inner = store.inner.lock().unwrap();
            for (n, id) in terminal_ids.iter().enumerate() {
                inner.tasks.get_mut(id).unwrap().created_at_ms = base - (55 - n as u64) * 1000;
            }
            inner.tasks.get_mut(&active.id).unwrap().created_at_ms = base;
        }

        // Cap of 50 over 55 recent terminal rows drops the 5 oldest.
        assert_eq!(store.prune_terminal_tasks(50), 5);
        assert!(store.get(terminal_ids[0].as_str()).is_none());
        assert!(store.get(terminal_ids[54].as_str()).is_some());

        let active_after = store.get(active.id.as_str()).unwrap();
        assert_eq!(active_after.status, TaskStatus::Pending);
        assert!(!active_after.blocked_by.contains(&terminal_ids[0]));
    }

    // Normal: the age window drops terminal rows older than the retention
    // horizon even when the count is well under the cap, and never prunes
    // active tasks regardless of age.
    #[test]
    fn prune_terminal_tasks_age_window_drops_stale_normal() {
        let store = TaskStore::in_memory();
        let now = now_ms();

        let stale = store
            .create("stale".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let fresh = store
            .create("fresh".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        for id in [&stale.id, &fresh.id] {
            store
                .update(
                    id.as_str(),
                    TaskPatch {
                        status: Some(TaskStatus::Completed),
                        ..Default::default()
                    },
                )
                .unwrap();
        }
        // An ancient *active* task must survive — age pruning is terminal-only.
        let old_active = store
            .create("old-active".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();

        {
            let mut inner = store.inner.lock().unwrap();
            // stale: 8 days old (past 7-day retention). fresh: 1 hour old.
            inner.tasks.get_mut(&stale.id).unwrap().created_at_ms = now - 8 * 24 * 60 * 60 * 1000;
            inner.tasks.get_mut(&fresh.id).unwrap().created_at_ms = now - 60 * 60 * 1000;
            inner.tasks.get_mut(&old_active.id).unwrap().created_at_ms =
                now - 30 * 24 * 60 * 60 * 1000;
        }

        // Cap is high; only the age window fires, dropping just `stale`.
        assert_eq!(store.prune_terminal_tasks(200), 1);
        assert!(store.get(stale.id.as_str()).is_none());
        assert!(store.get(fresh.id.as_str()).is_some());
        assert!(store.get(old_active.id.as_str()).is_some());
    }

    // Normal: pruned terminal tasks are archived to the sibling history JSONL
    // and can be read back, while the live store no longer contains them. This
    // is the working-memory → archival-memory handoff end-to-end.
    #[test]
    fn prune_archives_to_history_jsonl_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store_path = tmp.path().join("tasks.json");
        // Build a store directly on disk (bypass session-id path resolution).
        let store = Arc::new(TaskStore {
            inner: Mutex::new(TaskStoreInner::default()),
            path: store_path.clone(),
            disk_mtime: Mutex::new(None),
        });

        let task = store
            .create(
                "archived subject".into(),
                "d".into(),
                None,
                Vec::<TaskId>::new(),
            )
            .unwrap();
        store
            .update(
                task.id.as_str(),
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    tags: Some(vec!["perf".into()]),
                    ..Default::default()
                },
            )
            .unwrap();

        // Force the age window to fire by backdating the terminal task.
        {
            let mut inner = store.inner.lock().unwrap();
            inner.tasks.get_mut(&task.id).unwrap().created_at_ms = 1;
        }

        assert_eq!(store.prune_terminal_tasks(200), 1);
        // Live store no longer has it.
        assert!(store.get(task.id.as_str()).is_none());

        // History log exists beside the store and round-trips.
        let history_path = crate::task_history::history_path_for(&store_path);
        let records = crate::task_history::read_records(&history_path, 10, None);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].subject, "archived subject");
        assert_eq!(records[0].status, "completed");
        assert_eq!(records[0].tags, vec!["perf".to_owned()]);

        // Query filter matches by tag/subject.
        assert_eq!(
            crate::task_history::read_records(&history_path, 10, Some("perf")).len(),
            1
        );
        assert_eq!(
            crate::task_history::read_records(&history_path, 10, Some("nomatch")).len(),
            0
        );
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
        let expected_blocked_by: BTreeSet<TaskId> = std::iter::once(t1.id).collect();
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
        drop(t3);
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

    #[test]
    fn delete_legacy_placeholders_removes_exact_fixture_rows_robust() {
        let store = TaskStore::in_memory();
        let placeholder = store
            .create("subj".into(), "desc".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let keeper = store
            .create(
                "subj".into(),
                "real task body".into(),
                None,
                vec![placeholder.id.clone()],
            )
            .unwrap();

        let deleted = store.delete_legacy_placeholders();

        assert_eq!(deleted, 1);
        // Hard-removed, not tombstoned — the fixture row is gone entirely so
        // it can't accumulate across resumes.
        assert!(store.get(placeholder.id.as_str()).is_none());
        let keeper_after = store.get(keeper.id.as_str()).unwrap();
        assert!(keeper_after.blocked_by.is_empty());
        assert_eq!(store.list(DeletedFilter::Exclude).len(), 1);
        assert_eq!(store.list(DeletedFilter::Include).len(), 1);
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
            path,
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

    // ─── Proactive failure recovery (Surface 1) ──────────────────────────

    #[test]
    fn transient_failure_classification() {
        assert!(is_transient_failure("Stream idle timeout reached"));
        assert!(is_transient_failure("connection reset by peer"));
        assert!(is_transient_failure("HTTP 429 rate limit"));
        assert!(is_transient_failure("deadlock detected, try again"));
        // Hard/deterministic failures are NOT transient.
        assert!(!is_transient_failure("error[E0308]: mismatched types"));
        assert!(!is_transient_failure("assertion failed: x == y"));
        assert!(!is_transient_failure("test failed: 3 passed; 1 failed"));
        // A timeout phrase inside a compile error stays hard.
        assert!(!is_transient_failure(
            "compile error: function `timeout` not found"
        ));
    }

    #[test]
    fn transient_failure_retries_under_budget() {
        let store = TaskStore::in_memory();
        let t = store
            .create("flaky".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let r = store.recover_from_failure(t.id.as_str(), "connection timeout");
        match r {
            FailureRecovery::Retried { attempt, .. } => assert_eq!(attempt, 1),
            other => panic!("expected Retried, got {other:?}"),
        }
        // Re-queued as Pending, attempt_count bumped.
        let after = store.get(t.id.as_str()).unwrap();
        assert_eq!(after.status, TaskStatus::Pending);
        assert_eq!(after.attempt_count, 1);
    }

    #[test]
    fn transient_failure_hard_fails_after_budget() {
        let store = TaskStore::in_memory();
        let t = store
            .create("flaky".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        // Exhaust the budget (DEFAULT_MAX_ATTEMPTS = 3).
        let _ = store.recover_from_failure(t.id.as_str(), "timeout");
        let _ = store.recover_from_failure(t.id.as_str(), "timeout");
        let r = store.recover_from_failure(t.id.as_str(), "timeout");
        assert!(
            matches!(r, FailureRecovery::Replanned { .. }),
            "should hard-fail after budget, got {r:?}"
        );
        let after = store.get(t.id.as_str()).unwrap();
        assert_eq!(after.status, TaskStatus::Failed);
        assert_eq!(after.attempt_count, 3);
    }

    #[test]
    fn hard_failure_reroutes_dependents_non_destructively() {
        let store = TaskStore::in_memory();
        let base = store
            .create("base".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        // A dependent blocked by `base`.
        let dep = store
            .create("dependent".into(), "d".into(), None, vec![base.id.clone()])
            .unwrap();

        // Non-transient failure → immediate hard fail + reroute.
        let r = store.recover_from_failure(base.id.as_str(), "error[E0308]: mismatched types");
        let FailureRecovery::Replanned {
            replan_id,
            rerouted,
            ..
        } = r
        else {
            panic!("expected Replanned");
        };

        // The dependent is PRESERVED (still Pending), not Failed.
        let dep_after = store.get(dep.id.as_str()).unwrap();
        assert_eq!(
            dep_after.status,
            TaskStatus::Pending,
            "dependent must be preserved, not destroyed"
        );
        // And it's now blocked on the replan task.
        assert!(
            dep_after.blocked_by.contains(&replan_id),
            "dependent should be rerouted to block on the replan task"
        );
        assert!(rerouted.contains(&dep.id));

        // The replan task exists and is Pending.
        let replan = store.get(replan_id.as_str()).unwrap();
        assert_eq!(replan.status, TaskStatus::Pending);
        assert_eq!(replan.kind, Some(TaskKind::Decision));
    }

    #[test]
    fn non_transient_failure_skips_retry() {
        let store = TaskStore::in_memory();
        let t = store
            .create("logic".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        // First failure, but non-transient → straight to replan (no retry).
        let r = store.recover_from_failure(t.id.as_str(), "assertion failed");
        assert!(matches!(r, FailureRecovery::Replanned { attempts: 1, .. }));
    }

    #[test]
    fn unknown_task_recovery_is_graceful() {
        let store = TaskStore::in_memory();
        assert_eq!(
            store.recover_from_failure("nope", "x"),
            FailureRecovery::Unknown
        );
    }

    // ─── Factory metrics (Surface 2) ─────────────────────────────────────

    #[test]
    fn factory_metrics_empty_store() {
        let store = TaskStore::in_memory();
        let m = store.factory_metrics();
        assert_eq!(m.total(), 0);
        assert_eq!(m.success_rate(), None);
        assert_eq!(m.rework_ratio(), 0.0);
    }

    #[test]
    fn factory_metrics_success_rate_and_rework() {
        let store = TaskStore::in_memory();
        let a = store
            .create("a".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let b = store
            .create("b".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        // a completes, b hard-fails (non-transient → replan created).
        store
            .update(
                a.id.as_str(),
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
        store.recover_from_failure(b.id.as_str(), "assertion failed");

        let m = store.factory_metrics();
        assert_eq!(m.completed, 1);
        assert_eq!(m.failed, 1);
        // success = completed / (completed + failed) = 1/2.
        assert_eq!(m.success_rate(), Some(0.5));
        // One replan task was created → rework.
        assert_eq!(m.replan_tasks, 1);
        assert!(m.rework_ratio() > 0.0);
    }

    #[test]
    fn factory_metrics_counts_retries() {
        let store = TaskStore::in_memory();
        let t = store
            .create("flaky".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        // Two transient retries (attempt_count → 2), then completes.
        store.recover_from_failure(t.id.as_str(), "timeout");
        store.recover_from_failure(t.id.as_str(), "timeout");
        store
            .update(
                t.id.as_str(),
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();

        let m = store.factory_metrics();
        assert_eq!(m.retried_tasks, 1);
        assert_eq!(m.total_attempts, 2);
        assert_eq!(m.multi_attempt_tasks, 1); // attempt_count > 1
    }

    #[test]
    fn requeue_stuck_resets_factory_owned_in_progress() {
        let store = TaskStore::in_memory();
        store
            .create("a".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        store
            .create("b".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();

        // Factory claims one task → it goes in_progress, owned by jfc-factory.
        let claimed = store.claim_next_available("jfc-factory").unwrap();
        assert_eq!(claimed.status, TaskStatus::InProgress);

        // A different owner (a live subagent) holds the other task.
        store
            .update(
                "t2",
                TaskPatch {
                    status: Some(TaskStatus::InProgress),
                    owner: Some("subagent-xyz".into()),
                    ..Default::default()
                },
            )
            .unwrap();

        // Reaping the factory owner resets ONLY t1, never the subagent's t2.
        let requeued = store.requeue_stuck("jfc-factory");
        assert_eq!(requeued.len(), 1);
        assert_eq!(requeued[0].as_str(), "t1");

        let t1 = store.get("t1").unwrap();
        assert_eq!(t1.status, TaskStatus::Pending);
        assert!(
            t1.owner.is_none(),
            "reaped task must be unowned for re-claim"
        );

        let t2 = store.get("t2").unwrap();
        assert_eq!(t2.status, TaskStatus::InProgress, "subagent work untouched");
        assert_eq!(t2.owner.as_deref(), Some("subagent-xyz"));

        // Idempotent: a second reap with nothing stuck is a no-op.
        assert!(store.requeue_stuck("jfc-factory").is_empty());

        // And the reaped task is immediately re-claimable.
        let reclaimed = store.claim_next_available("jfc-factory").unwrap();
        assert_eq!(reclaimed.id.as_str(), "t1");
    }

    #[test]
    fn effort_and_model_patch_and_serde_round_trip() {
        let store = TaskStore::in_memory();
        let t = store
            .create("hard task".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        assert!(t.effort.is_none() && t.model.is_none());

        let updated = store
            .update(
                t.id.as_str(),
                TaskPatch {
                    effort: Some("max".into()),
                    model: Some("claude-opus-4".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(updated.effort.as_deref(), Some("max"));
        assert_eq!(updated.model.as_deref(), Some("claude-opus-4"));

        // Persisted fields survive a JSON round-trip (they're skipped only
        // when None, so a set value must serialize and parse back).
        let json = serde_json::to_string(&updated).unwrap();
        assert!(json.contains("\"effort\":\"max\""));
        assert!(json.contains("\"model\":\"claude-opus-4\""));
        let back: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(back.effort.as_deref(), Some("max"));
        assert_eq!(back.model.as_deref(), Some("claude-opus-4"));
    }

    // ---- Parallel-DAG enhancements (Terraform internal/dag) ----

    fn complete(store: &TaskStore, id: &str) {
        store
            .update(
                id,
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    fn fail(store: &TaskStore, id: &str) {
        store
            .update(
                id,
                TaskPatch {
                    status: Some(TaskStatus::Failed),
                    ..Default::default()
                },
            )
            .unwrap();
    }

    // Normal: the ready frontier is exactly the pending, unowned tasks whose
    // blockers are all completed — and it grows as blockers complete.
    #[test]
    fn ready_frontier_tracks_completed_blockers_normal() {
        let store = TaskStore::in_memory();
        let t1 = store
            .create("a".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let _t2 = store
            .create("b".into(), "".into(), None, vec![t1.id])
            .unwrap();

        // Only the unblocked root is ready initially.
        let ready: Vec<String> = store
            .ready_frontier()
            .iter()
            .map(|t| t.id.to_string())
            .collect();
        assert_eq!(ready, vec!["t1".to_string()]);

        // Completing the blocker promotes the dependent into the frontier;
        // the completed task itself drops out (it's no longer Pending).
        complete(&store, "t1");
        let ready: Vec<String> = store
            .ready_frontier()
            .iter()
            .map(|t| t.id.to_string())
            .collect();
        assert_eq!(ready, vec!["t2".to_string()]);
    }

    // Normal: a claimed (owned) task is not in the ready frontier even though
    // its blockers are clear — it's already being worked.
    #[test]
    fn ready_frontier_excludes_owned_tasks_normal() {
        let store = TaskStore::in_memory();
        store
            .create("solo".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        assert_eq!(store.ready_frontier().len(), 1);
        store.claim_next_available("worker").unwrap();
        assert!(store.ready_frontier().is_empty());
    }

    // Robust: transitive upstreamFailed — a failed task taints every task that
    // (transitively) depends on it, so they are not reported as their own
    // failures. validate().upstream_failed is the transitive superset.
    #[test]
    fn validate_propagates_upstream_failed_robust() {
        let store = TaskStore::in_memory();
        let t1 = store
            .create("root".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let t2 = store
            .create("mid".into(), "".into(), None, vec![t1.id])
            .unwrap();
        let _t3 = store
            .create("leaf".into(), "".into(), None, vec![t2.id])
            .unwrap();

        fail(&store, "t1");
        let v = store.validate();
        // t2 (direct) and t3 (transitive) are upstream-failed; the failed t1
        // itself is not (it owns its terminal state).
        assert_eq!(
            v.upstream_failed,
            vec![TaskId::from("t2"), TaskId::from("t3")]
        );
        // blocked_forever is the *direct, all-blockers-dead* subset: t2 only.
        assert_eq!(v.blocked_forever, vec![TaskId::from("t2")]);
    }

    // Robust: Tarjan finds a cycle that the per-edge guard cannot — one
    // injected directly into the store as if loaded from hand-edited JSON.
    #[test]
    fn validate_detects_injected_dependency_cycle_robust() {
        let store = TaskStore::in_memory();
        let t1 = store
            .create("a".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        let t2 = store
            .create("b".into(), "".into(), None, vec![t1.id.clone()])
            .unwrap();
        // Inject the back-edge t1 -> t2 directly (bypassing update()'s guard),
        // forming the cycle t1 <-> t2.
        {
            let mut inner = store.inner.lock().unwrap();
            inner
                .tasks
                .get_mut(t1.id.as_str())
                .unwrap()
                .blocked_by
                .insert(t2.id);
        }
        let v = store.validate();
        assert_eq!(v.dependency_cycles.len(), 1, "exactly one cycle");
        assert_eq!(
            v.dependency_cycles[0],
            vec![TaskId::from("t1"), TaskId::from("t2")]
        );
    }

    // Normal: an acyclic graph reports no cycles.
    #[test]
    fn validate_no_cycles_on_dag_normal() {
        let store = TaskStore::in_memory();
        let t1 = store
            .create("a".into(), "".into(), None, Vec::<TaskId>::new())
            .unwrap();
        store
            .create("b".into(), "".into(), None, vec![t1.id])
            .unwrap();
        assert!(store.validate().dependency_cycles.is_empty());
    }

    // ---- Retry backoff (k8s per-item exponential curve) ----

    // Normal: the backoff curve doubles per attempt and caps.
    #[test]
    fn retry_backoff_curve_normal() {
        assert_eq!(retry_backoff_ms(1), 1_000); // base
        assert_eq!(retry_backoff_ms(2), 2_000);
        assert_eq!(retry_backoff_ms(3), 4_000);
        assert_eq!(retry_backoff_ms(99), RETRY_MAX_MS); // saturates, never panics
    }

    // Robust: a transient retry stamps a future deadline and the task is NOT
    // re-claimable until it passes — no hot-loop retrying.
    #[test]
    fn transient_retry_backs_off_claim_robust() {
        let store = TaskStore::in_memory();
        let t = store
            .create("flaky".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        store.recover_from_failure(t.id.as_str(), "connection timeout");

        let after = store.get(t.id.as_str()).unwrap();
        assert_eq!(after.status, TaskStatus::Pending);
        // Deadline is in the future.
        assert!(in_retry_backoff(&after, now_ms()));
        // And the factory can't immediately re-claim it.
        assert!(store.claim_next_available("jfc-factory").is_none());
        // It's also excluded from the ready frontier.
        assert!(store.ready_frontier().is_empty());
    }

    // Robust: once the backoff deadline is in the past, the task is claimable
    // again and the deadline is cleared on claim (Forget).
    #[test]
    fn elapsed_backoff_is_claimable_and_forgotten_robust() {
        let store = TaskStore::in_memory();
        let t = store
            .create("flaky".into(), "d".into(), None, Vec::<TaskId>::new())
            .unwrap();
        // Inject a deadline in the past (as if the backoff has elapsed).
        {
            let mut inner = store.inner.lock().unwrap();
            let task = inner.tasks.get_mut(t.id.as_str()).unwrap();
            set_retry_after(task, 1); // epoch ms 1 — long past
        }
        let claimed = store.claim_next_available("jfc-factory").unwrap();
        assert_eq!(claimed.id, t.id);
        // The retry deadline was forgotten on claim.
        assert!(!in_retry_backoff(&claimed, now_ms()));
        assert!(
            store.get(t.id.as_str()).unwrap().metadata.is_none() || {
                let m = store.get(t.id.as_str()).unwrap();
                !in_retry_backoff(&m, now_ms())
            }
        );
    }
}
