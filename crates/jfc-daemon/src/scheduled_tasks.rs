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

use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::cron::{CronJob, CronSchedule, should_fire_cron};

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

    /// Record a run result and advance `last_run`.
    pub fn record_run(&mut self, now: SystemTime, ok: bool, note: impl Into<String>) {
        self.last_run = Some(now);
        self.runs.push(TaskRun {
            ran_at: now,
            ok,
            note: note.into(),
        });
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

    /// All tasks due to fire at `now` (active only).
    pub fn due(&self, now: SystemTime) -> Vec<&ScheduledTask> {
        self.tasks.iter().filter(|t| t.should_fire(now)).collect()
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

    /// Load the registry from `path`. A missing file yields an empty registry
    /// (first run); a malformed file is an error.
    pub fn load(path: &std::path::Path) -> std::io::Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(s) => serde_json::from_str(&s)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::new()),
            Err(e) => Err(e),
        }
    }

    /// Persist the registry to `path` (pretty JSON), creating parent dirs.
    pub fn save(&self, path: &std::path::Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(path, json)
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
