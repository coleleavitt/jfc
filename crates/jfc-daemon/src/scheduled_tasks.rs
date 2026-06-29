//! Scheduled / recurring agentic tasks.
//!
//! Mirrors Perplexity Computer's recurring-task surface found in the 2026-06-11
//! mindemon dump: `/rest/thread/list_scheduled_computer_tasks` +
//! `list_archived_computer_tasks`, plus the UI affordances "Create a Scheduled
//! Search", "Automations and recurring templates", and "Connected to your tools
//! and scheduled on autopilot".
//!
//! Where [`crate::cron::CronJob`] schedules a *shell command*, a
//! [`ScheduledTask`] schedules a recurring *agentic prompt* — a natural-language
//! instruction the agent runs on a [`CronSchedule`] — with a lifecycle (active /
//! paused / archived) and run history. The firing decision reuses
//! [`crate::cron::should_fire_cron`] so there is one schedule engine.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::cron::{CronJob, CronSchedule, should_fire_cron};

const SCHEDULED_TASKS_SESSION_ID: &str = "__daemon__";
const SCHEDULED_TASKS_KIND: &str = "scheduled_tasks";

fn artifact_key(path: &Path) -> String {
    path.display().to_string()
}

fn artifact_store(path: &Path) -> std::io::Result<jfc_knowledge::KnowledgeStore> {
    let default_base = crate::state::DaemonPaths::default_user().base_dir;
    if path.starts_with(&default_base) {
        return jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open_default())
            .map_err(std::io::Error::other);
    }
    let db_dir = path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&db_dir)?;
    jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open(
        &db_dir.join("knowledge.db"),
    ))
    .map_err(std::io::Error::other)
}

/// Lifecycle state of a scheduled agentic task. Mirrors the
/// scheduled-vs-archived split in Perplexity's two list endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum TaskLifecycle {
    /// Active and eligible to fire on schedule.
    #[default]
    Active,
    /// Temporarily disabled; retained but won't fire.
    Paused,
    /// Retired; appears only in the archived list and never fires.
    Archived,
}

/// A single past run of a scheduled task.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskRun {
    pub ran_at: SystemTime,
    pub ok: bool,
    /// Short note (e.g. an error summary or a result pointer).
    pub note: String,
}

/// A recurring agentic task: a prompt the agent runs on a schedule.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScheduledTask {
    pub id: String,
    /// Human title ("Weekly M&A digest").
    pub title: String,
    /// The natural-language instruction the agent executes each run.
    pub prompt: String,
    pub schedule: CronSchedule,
    pub lifecycle: TaskLifecycle,
    pub last_run: Option<SystemTime>,
    pub created_at: SystemTime,
    pub runs: Vec<TaskRun>,
}

impl ScheduledTask {
    pub fn new(
        id: impl Into<String>,
        title: impl Into<String>,
        prompt: impl Into<String>,
        schedule: CronSchedule,
        now: SystemTime,
    ) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            prompt: prompt.into(),
            schedule,
            lifecycle: TaskLifecycle::Active,
            last_run: None,
            created_at: now,
            runs: Vec::new(),
        }
    }

    pub fn is_active(&self) -> bool {
        self.lifecycle == TaskLifecycle::Active
    }

    pub fn is_archived(&self) -> bool {
        self.lifecycle == TaskLifecycle::Archived
    }

    /// Whether this task should fire at `now`. Only active tasks ever fire.
    /// Reuses the cron engine by projecting onto a transient [`CronJob`].
    pub fn should_fire(&self, now: SystemTime) -> bool {
        if !self.is_active() {
            return false;
        }
        let projected = CronJob {
            id: self.id.clone(),
            schedule: self.schedule.clone(),
            description: self.title.clone(),
            command: String::new(),
            enabled: true,
            last_run: self.last_run,
            created_at: self.created_at,
        };
        should_fire_cron(&projected, now)
    }

    /// Record a run result and advance `last_run`. Run history is bounded to
    /// the most recent [`Self::MAX_RUN_HISTORY`] entries so a high-frequency
    /// task can't grow the registry file without limit.
    pub fn record_run(&mut self, now: SystemTime, ok: bool, note: impl Into<String>) {
        self.last_run = Some(now);
        self.runs.push(TaskRun {
            ran_at: now,
            ok,
            note: note.into(),
        });
        let len = self.runs.len();
        if len > Self::MAX_RUN_HISTORY {
            self.runs.drain(0..len - Self::MAX_RUN_HISTORY);
        }
    }

    /// Maximum retained run-history entries per task.
    pub const MAX_RUN_HISTORY: usize = 50;

    /// Update the outcome of the most recent run (the one `due_and_advance`
    /// recorded as "fired"), e.g. once the headless process exits. No-op if
    /// there is no run history.
    pub fn record_outcome(&mut self, ok: bool, note: impl Into<String>) {
        if let Some(last) = self.runs.last_mut() {
            last.ok = ok;
            last.note = note.into();
        }
    }

    pub fn record_outcome_at(
        &mut self,
        ran_at: SystemTime,
        ok: bool,
        note: impl Into<String>,
    ) -> bool {
        let Some(run) = self.runs.iter_mut().find(|run| run.ran_at == ran_at) else {
            return false;
        };
        run.ok = ok;
        run.note = note.into();
        true
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledTaskSnapshot {
    pub id: String,
    pub title: String,
    pub prompt: String,
    pub lifecycle: TaskLifecycle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledTaskCreate {
    pub id: String,
    pub title: String,
    pub cron_expr: String,
    pub prompt: String,
}

pub trait ScheduledTaskManagementService: Send + Sync + 'static {
    fn list_scheduled_tasks(&self, archived: bool) -> Result<Vec<ScheduledTaskSnapshot>, String>;

    fn create_scheduled_task(&self, request: ScheduledTaskCreate) -> Result<String, String>;

    fn set_scheduled_task_lifecycle(
        &self,
        id: &str,
        lifecycle: TaskLifecycle,
    ) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct ScheduledTaskRegistryService {
    base_dir: PathBuf,
}

impl ScheduledTaskRegistryService {
    pub fn new(base_dir: impl Into<PathBuf>) -> Self {
        Self {
            base_dir: base_dir.into(),
        }
    }

    pub fn for_default_user() -> Self {
        Self::new(crate::state::DaemonPaths::default_user().base_dir)
    }

    fn registry_path(&self) -> PathBuf {
        ScheduledTaskRegistry::default_path(&self.base_dir)
    }

    fn load_registry(&self) -> Result<(ScheduledTaskRegistry, PathBuf), String> {
        let path = self.registry_path();
        let registry = ScheduledTaskRegistry::load(&path).map_err(|e| e.to_string())?;
        Ok((registry, path))
    }
}

impl ScheduledTaskManagementService for ScheduledTaskRegistryService {
    fn list_scheduled_tasks(&self, archived: bool) -> Result<Vec<ScheduledTaskSnapshot>, String> {
        let (registry, _) = self.load_registry()?;
        let tasks = if archived {
            registry.list_archived()
        } else {
            registry.list_scheduled()
        };
        Ok(tasks.into_iter().map(ScheduledTaskSnapshot::from).collect())
    }

    fn create_scheduled_task(&self, request: ScheduledTaskCreate) -> Result<String, String> {
        let (mut registry, path) = self.load_registry()?;
        let schedule = crate::cron::parse_schedule(&request.cron_expr)
            .map_err(|e| format!("bad cron: {e}"))?;
        let task = ScheduledTask::new(
            request.id.clone(),
            request.title,
            request.prompt,
            schedule,
            SystemTime::now(),
        );
        registry.create(task)?;
        registry
            .save(&path)
            .map_err(|e| format!("Created but failed to persist: {e}"))?;
        Ok(request.id)
    }

    fn set_scheduled_task_lifecycle(
        &self,
        id: &str,
        lifecycle: TaskLifecycle,
    ) -> Result<(), String> {
        let (mut registry, path) = self.load_registry()?;
        match lifecycle {
            TaskLifecycle::Active => registry.resume(id),
            TaskLifecycle::Paused => registry.pause(id),
            TaskLifecycle::Archived => registry.archive(id),
        }?;
        registry.save(&path).map_err(|e| e.to_string())
    }
}

impl From<&ScheduledTask> for ScheduledTaskSnapshot {
    fn from(task: &ScheduledTask) -> Self {
        Self {
            id: task.id.clone(),
            title: task.title.clone(),
            prompt: task.prompt.clone(),
            lifecycle: task.lifecycle,
        }
    }
}

/// A registry of scheduled agentic tasks. Provides the create / list-scheduled /
/// list-archived / pause / archive surface.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ScheduledTaskRegistry {
    tasks: Vec<ScheduledTask>,
}

impl ScheduledTaskRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a new task. Returns an error if the id already exists.
    pub fn create(&mut self, task: ScheduledTask) -> Result<(), String> {
        if self.tasks.iter().any(|t| t.id == task.id) {
            return Err(format!("scheduled task id already exists: {}", task.id));
        }
        self.tasks.push(task);
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&ScheduledTask> {
        self.tasks.iter().find(|t| t.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Option<&mut ScheduledTask> {
        self.tasks.iter_mut().find(|t| t.id == id)
    }

    /// List active (scheduled) tasks — mirrors `list_scheduled_computer_tasks`.
    pub fn list_scheduled(&self) -> Vec<&ScheduledTask> {
        self.tasks.iter().filter(|t| t.is_active()).collect()
    }

    /// List archived tasks — mirrors `list_archived_computer_tasks`.
    pub fn list_archived(&self) -> Vec<&ScheduledTask> {
        self.tasks.iter().filter(|t| t.is_archived()).collect()
    }

    /// List paused tasks.
    pub fn list_paused(&self) -> Vec<&ScheduledTask> {
        self.tasks
            .iter()
            .filter(|t| t.lifecycle == TaskLifecycle::Paused)
            .collect()
    }

    /// All tasks due to fire at `now` (active only). Read-only.
    pub fn due(&self, now: SystemTime) -> Vec<&ScheduledTask> {
        self.tasks.iter().filter(|t| t.should_fire(now)).collect()
    }

    /// Fire-side of the scheduler: return clones of all due active tasks AND
    /// advance their `last_run` (recording a run entry) so they don't re-fire
    /// on the next tick. Mirrors the daemon's `tick_cron` mutate-then-persist
    /// semantics. The caller persists the registry after firing.
    pub fn due_and_advance(&mut self, now: SystemTime) -> Vec<ScheduledTask> {
        let mut fired = Vec::new();
        for task in &mut self.tasks {
            if task.should_fire(now) {
                task.record_run(now, true, "fired");
                fired.push(task.clone());
            }
        }
        fired
    }

    fn set_lifecycle(&mut self, id: &str, lifecycle: TaskLifecycle) -> Result<(), String> {
        let task = self
            .get_mut(id)
            .ok_or_else(|| format!("unknown scheduled task: {id}"))?;
        task.lifecycle = lifecycle;
        Ok(())
    }

    pub fn pause(&mut self, id: &str) -> Result<(), String> {
        self.set_lifecycle(id, TaskLifecycle::Paused)
    }

    pub fn resume(&mut self, id: &str) -> Result<(), String> {
        self.set_lifecycle(id, TaskLifecycle::Active)
    }

    pub fn archive(&mut self, id: &str) -> Result<(), String> {
        self.set_lifecycle(id, TaskLifecycle::Archived)
    }

    /// Permanently remove a task.
    pub fn delete(&mut self, id: &str) -> Option<ScheduledTask> {
        if let Some(pos) = self.tasks.iter().position(|t| t.id == id) {
            Some(self.tasks.remove(pos))
        } else {
            None
        }
    }

    pub fn len(&self) -> usize {
        self.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    /// Default on-disk location for the registry under a config dir
    /// (`<config>/scheduled-tasks.json`).
    pub fn default_path(config_dir: &std::path::Path) -> std::path::PathBuf {
        config_dir.join("scheduled-tasks.json")
    }

    /// Load the registry from the DB row keyed by `path`. A missing row yields
    /// an empty registry; a malformed row is an error.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        let store = artifact_store(path)?;
        let key = artifact_key(path);
        if let Some(row) = jfc_knowledge::block_on_knowledge(async {
            store
                .get_session_artifact(SCHEDULED_TASKS_SESSION_ID, SCHEDULED_TASKS_KIND, &key)
                .await
        })
        .map_err(std::io::Error::other)?
        {
            return serde_json::from_str(&row.value_json)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e));
        }
        let legacy = match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str::<Self>(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Self::new()),
            Err(e) => return Err(e),
        };
        let json = serde_json::to_string(&legacy)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        jfc_knowledge::block_on_knowledge(async {
            store
                .upsert_session_artifact(
                    SCHEDULED_TASKS_SESSION_ID,
                    SCHEDULED_TASKS_KIND,
                    &key,
                    &json,
                )
                .await
        })
        .map_err(std::io::Error::other)?;
        Ok(legacy)
    }

    /// Record the completion outcome of a task's most recent run and persist
    /// the registry. Reloads from `path` first so a concurrent tick that fired
    /// other tasks isn't clobbered. No-op if the task no longer exists.
    pub fn record_run_outcome(
        path: &std::path::Path,
        task_id: &str,
        ok: bool,
        note: impl Into<String>,
    ) -> std::io::Result<()> {
        let mut registry = Self::load(path)?;
        if let Some(task) = registry.get_mut(task_id) {
            task.record_outcome(ok, note);
            registry.save(path)?;
        }
        Ok(())
    }

    pub fn record_run_outcome_at(
        path: &std::path::Path,
        task_id: &str,
        ran_at: SystemTime,
        ok: bool,
        note: impl Into<String>,
    ) -> std::io::Result<()> {
        let mut registry = Self::load(path)?;
        if let Some(task) = registry.get_mut(task_id)
            && task.record_outcome_at(ran_at, ok, note)
        {
            registry.save(path)?;
        }
        Ok(())
    }

    /// Persist the registry to the DB row keyed by `path`.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        let json = serde_json::to_string(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        let store = artifact_store(path)?;
        jfc_knowledge::block_on_knowledge(async {
            store
                .upsert_session_artifact(
                    SCHEDULED_TASKS_SESSION_ID,
                    SCHEDULED_TASKS_KIND,
                    &artifact_key(path),
                    &json,
                )
                .await
        })
        .map_err(std::io::Error::other)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(secs: u64) -> SystemTime {
        SystemTime::UNIX_EPOCH + Duration::from_secs(secs)
    }

    fn every(secs: u64) -> CronSchedule {
        CronSchedule::Every {
            period: Duration::from_secs(secs),
        }
    }

    fn task(id: &str, sched: CronSchedule, now: SystemTime) -> ScheduledTask {
        ScheduledTask::new(id, format!("title-{id}"), "do the thing", sched, now)
    }

    #[test]
    fn registry_service_creates_lists_and_mutates_via_daemon_pack_seam_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        let service = ScheduledTaskRegistryService::new(dir.path());

        let id = service
            .create_scheduled_task(ScheduledTaskCreate {
                id: "task-a".to_owned(),
                title: "Morning check".to_owned(),
                cron_expr: "* * * * *".to_owned(),
                prompt: "summarize overnight changes".to_owned(),
            })
            .unwrap();
        let scheduled = service.list_scheduled_tasks(false).unwrap();

        assert_eq!(id, "task-a");
        assert_eq!(scheduled.len(), 1);
        assert_eq!(scheduled[0].id, "task-a");
        assert_eq!(scheduled[0].lifecycle, TaskLifecycle::Active);

        service
            .set_scheduled_task_lifecycle("task-a", TaskLifecycle::Archived)
            .unwrap();
        assert!(service.list_scheduled_tasks(false).unwrap().is_empty());
        assert_eq!(service.list_scheduled_tasks(true).unwrap()[0].id, "task-a");
    }

    // ── CRUD ───────────────────────────────────────────────────────────────────

    #[test]
    fn create_and_get_normal() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("t1", every(3600), at(0))).unwrap();
        assert_eq!(reg.len(), 1);
        assert_eq!(reg.get("t1").unwrap().title, "title-t1");
    }

    #[test]
    fn create_duplicate_id_is_error_robust() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("t1", every(3600), at(0))).unwrap();
        let err = reg.create(task("t1", every(3600), at(0))).unwrap_err();
        assert!(err.contains("already exists"));
    }

    // ── Lifecycle + listing ──────────────────────────────────────────────────

    #[test]
    fn list_scheduled_vs_archived_normal() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("a", every(60), at(0))).unwrap();
        reg.create(task("b", every(60), at(0))).unwrap();
        reg.archive("b").unwrap();
        let scheduled: Vec<_> = reg.list_scheduled().iter().map(|t| t.id.clone()).collect();
        let archived: Vec<_> = reg.list_archived().iter().map(|t| t.id.clone()).collect();
        assert_eq!(scheduled, vec!["a"]);
        assert_eq!(archived, vec!["b"]);
    }

    #[test]
    fn pause_then_resume_moves_between_lists_normal() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("a", every(60), at(0))).unwrap();
        reg.pause("a").unwrap();
        assert!(reg.list_scheduled().is_empty());
        assert_eq!(reg.list_paused().len(), 1);
        reg.resume("a").unwrap();
        assert_eq!(reg.list_scheduled().len(), 1);
    }

    #[test]
    fn set_lifecycle_unknown_id_is_error_robust() {
        let mut reg = ScheduledTaskRegistry::new();
        assert!(reg.archive("nope").is_err());
        assert!(reg.pause("nope").is_err());
    }

    #[test]
    fn delete_removes_task_normal() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("a", every(60), at(0))).unwrap();
        assert!(reg.delete("a").is_some());
        assert!(reg.is_empty());
        assert!(reg.delete("a").is_none());
    }

    // ── Firing (schedule engine reuse) ─────────────────────────────────────────

    #[test]
    fn due_returns_tasks_past_their_interval_normal() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("hourly", every(3600), at(0))).unwrap();
        // Never run → fires immediately.
        assert_eq!(reg.due(at(0)).len(), 1);
        // After a run, not due until the interval elapses.
        reg.get_mut("hourly").unwrap().record_run(at(0), true, "ok");
        assert!(reg.due(at(1800)).is_empty());
        assert_eq!(reg.due(at(3600)).len(), 1);
    }

    #[test]
    fn due_and_advance_fires_then_does_not_refire_normal() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("hourly", every(3600), at(0))).unwrap();
        // First tick at/after creation: never-run → fires once, advances last_run.
        let fired = reg.due_and_advance(at(0));
        assert_eq!(fired.len(), 1);
        assert_eq!(fired[0].id, "hourly");
        // Immediately again: not due (just ran) → no re-fire.
        assert!(reg.due_and_advance(at(1)).is_empty());
        // After the interval: due again.
        assert_eq!(reg.due_and_advance(at(3600)).len(), 1);
        // Run history recorded.
        assert_eq!(reg.get("hourly").unwrap().runs.len(), 2);
    }

    #[test]
    fn due_and_advance_skips_paused_and_archived_robust() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("p", every(60), at(0))).unwrap();
        reg.create(task("z", every(60), at(0))).unwrap();
        reg.pause("p").unwrap();
        reg.archive("z").unwrap();
        assert!(reg.due_and_advance(at(100)).is_empty());
    }

    #[test]
    fn paused_and_archived_never_fire_robust() {
        let mut reg = ScheduledTaskRegistry::new();
        reg.create(task("p", every(60), at(0))).unwrap();
        reg.create(task("z", every(60), at(0))).unwrap();
        reg.pause("p").unwrap();
        reg.archive("z").unwrap();
        // Both would otherwise be due (never run), but neither is active.
        assert!(reg.due(at(100)).is_empty());
        assert!(!reg.get("p").unwrap().should_fire(at(100)));
        assert!(!reg.get("z").unwrap().should_fire(at(100)));
    }

    #[test]
    fn record_run_appends_history_and_advances_last_run_normal() {
        let mut t = task("a", every(60), at(0));
        t.record_run(at(60), true, "first");
        t.record_run(at(120), false, "boom");
        assert_eq!(t.runs.len(), 2);
        assert_eq!(t.last_run, Some(at(120)));
        assert!(!t.runs[1].ok);
        assert_eq!(t.runs[0].note, "first");
    }

    #[test]
    fn load_missing_file_is_empty_then_save_roundtrips_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = ScheduledTaskRegistry::default_path(dir.path());
        // Missing → empty.
        let mut reg = ScheduledTaskRegistry::load(&path).unwrap();
        assert!(reg.is_empty());
        reg.create(task("a", every(3600), at(0))).unwrap();
        reg.save(&path).unwrap();
        // Reload sees it.
        let back = ScheduledTaskRegistry::load(&path).unwrap();
        assert_eq!(back.len(), 1);
        assert!(back.get("a").is_some());
    }

    #[test]
    fn record_run_bounds_history_robust() {
        let mut t = task("a", every(60), at(0));
        let total = ScheduledTask::MAX_RUN_HISTORY + 20;
        for i in 0..total {
            t.record_run(at(i as u64), true, format!("run {i}"));
        }
        assert_eq!(t.runs.len(), ScheduledTask::MAX_RUN_HISTORY);
        // Oldest entries dropped — newest retained.
        assert_eq!(t.runs.last().unwrap().note, format!("run {}", total - 1));
    }

    #[test]
    fn record_outcome_updates_last_run_normal() {
        let mut t = task("a", every(60), at(0));
        t.record_run(at(1), true, "fired");
        t.record_outcome(false, "exit 1 → /tmp/x.log");
        let last = t.runs.last().unwrap();
        assert!(!last.ok);
        assert!(last.note.contains("exit 1"));
    }

    #[test]
    fn record_outcome_at_updates_exact_run_regression() {
        let mut t = task("a", every(60), at(0));
        t.record_run(at(60), true, "first fired");
        t.record_run(at(120), true, "second fired");

        assert!(t.record_outcome_at(at(60), false, "first failed"));

        assert!(!t.runs[0].ok);
        assert_eq!(t.runs[0].note, "first failed");
        assert!(t.runs[1].ok);
        assert_eq!(t.runs[1].note, "second fired");
    }

    #[test]
    fn record_run_outcome_persists_normal() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = ScheduledTaskRegistry::default_path(dir.path());
        let mut reg = ScheduledTaskRegistry::new();
        let mut t = task("a", every(60), at(0));
        t.record_run(at(1), true, "fired");
        reg.create(t).unwrap();
        reg.save(&path).unwrap();

        ScheduledTaskRegistry::record_run_outcome(&path, "a", true, "ok → /tmp/a.log").unwrap();
        let back = ScheduledTaskRegistry::load(&path).unwrap();
        let run = back.get("a").unwrap().runs.last().unwrap();
        assert!(run.ok);
        assert!(run.note.contains("ok →"));
    }

    #[test]
    fn record_run_outcome_at_does_not_update_newer_run_regression() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = ScheduledTaskRegistry::default_path(dir.path());
        let mut reg = ScheduledTaskRegistry::new();
        let mut t = task("a", every(60), at(0));
        t.record_run(at(60), true, "first fired");
        t.record_run(at(120), true, "second fired");
        reg.create(t).unwrap();
        reg.save(&path).unwrap();

        ScheduledTaskRegistry::record_run_outcome_at(&path, "a", at(60), false, "first failed")
            .unwrap();

        let back = ScheduledTaskRegistry::load(&path).unwrap();
        let task = back.get("a").unwrap();
        assert!(!task.runs[0].ok);
        assert_eq!(task.runs[0].note, "first failed");
        assert!(task.runs[1].ok);
        assert_eq!(task.runs[1].note, "second fired");
    }

    #[test]
    fn record_run_outcome_unknown_task_is_noop_robust() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = ScheduledTaskRegistry::default_path(dir.path());
        ScheduledTaskRegistry::new().save(&path).unwrap();
        // Should not error on a missing task id.
        ScheduledTaskRegistry::record_run_outcome(&path, "ghost", true, "x").unwrap();
    }

    #[test]
    fn registry_roundtrips_serde_robust() {
        let mut reg = ScheduledTaskRegistry::new();
        let mut t = task("a", every(60), at(0));
        t.record_run(at(60), true, "ran");
        reg.create(t).unwrap();
        reg.create(task("b", every(120), at(0))).unwrap();
        reg.archive("b").unwrap();

        let json = serde_json::to_string(&reg).unwrap();
        let back: ScheduledTaskRegistry = serde_json::from_str(&json).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back.list_scheduled().len(), 1);
        assert_eq!(back.list_archived().len(), 1);
        assert_eq!(back.get("a").unwrap().runs.len(), 1);
    }
}
