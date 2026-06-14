//! Durable scheduled-task persistence for `.claude/scheduled_tasks.json`.
//!
//! CC 2.1.167 persists `CronCreate(durable: true)` jobs to this file so they
//! survive restarts. JFC mirrors this: on startup the engine loads any durable
//! tasks and re-registers them in the cron scheduler; on creation/deletion the
//! file is updated atomically.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::atomic_write::write_atomic_sync;

/// Canonical path to the scheduled-tasks persistence file.
pub fn scheduled_tasks_path(project_root: &Path) -> PathBuf {
    project_root.join(".claude").join("scheduled_tasks.json")
}

/// A single durable scheduled task, mirroring CC's persisted shape.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScheduledTask {
    /// Unique identifier for this task (UUID string).
    pub id: String,
    /// Cron expression (e.g. `"0 9 * * *"`) or an ISO-8601 one-shot timestamp.
    pub schedule: String,
    /// Whether this is a recurring (`cron`) or one-shot (`once`) job.
    #[serde(default = "default_kind")]
    pub kind: ScheduledTaskKind,
    /// The prompt/command to execute when the job fires.
    pub prompt: String,
    /// Creation time in milliseconds since Unix epoch.
    pub created_at_ms: u64,
    /// Whether this task was created with `durable: true`.
    #[serde(default = "default_true")]
    pub durable: bool,
    /// Optional human-readable label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Whether the task fires on a recurring schedule or just once.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ScheduledTaskKind {
    #[default]
    Cron,
    Once,
}

fn default_kind() -> ScheduledTaskKind {
    ScheduledTaskKind::Cron
}

fn default_true() -> bool {
    true
}

/// Load all durable scheduled tasks from `<project_root>/.claude/scheduled_tasks.json`.
///
/// Returns an empty `Vec` when the file is absent. Emits `tracing::warn` on
/// parse errors so misconfigured files are visible without crashing.
pub fn load_scheduled_tasks(project_root: &Path) -> Vec<ScheduledTask> {
    let path = scheduled_tasks_path(project_root);
    let raw = match std::fs::read_to_string(&path) {
        Ok(r) => r,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(err) => {
            tracing::warn!(
                target: "jfc::config::scheduled_tasks",
                path = %path.display(),
                error = %err,
                "failed to read scheduled_tasks.json"
            );
            return Vec::new();
        }
    };
    match serde_json::from_str::<Vec<ScheduledTask>>(&raw) {
        Ok(tasks) => tasks,
        Err(err) => {
            tracing::warn!(
                target: "jfc::config::scheduled_tasks",
                path = %path.display(),
                error = %err,
                "failed to parse scheduled_tasks.json — returning empty task list"
            );
            Vec::new()
        }
    }
}

/// Persist the scheduled-task list to `<project_root>/.claude/scheduled_tasks.json`.
///
/// Uses an atomic write (write to a temp file, then rename) to avoid
/// corruption on crash. Creates the `.claude/` directory if needed.
pub fn save_scheduled_tasks(project_root: &Path, tasks: &[ScheduledTask]) -> std::io::Result<()> {
    let path = scheduled_tasks_path(project_root);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(tasks)
        .map_err(|err| std::io::Error::new(std::io::ErrorKind::InvalidData, err))?;
    write_atomic_sync(&path, json.as_bytes())
}

/// Add a task to the persisted list (idempotent by `id`).
pub fn upsert_scheduled_task(project_root: &Path, task: ScheduledTask) -> std::io::Result<()> {
    let mut tasks = load_scheduled_tasks(project_root);
    if let Some(pos) = tasks.iter().position(|t| t.id == task.id) {
        tasks[pos] = task;
    } else {
        tasks.push(task);
    }
    save_scheduled_tasks(project_root, &tasks)
}

/// Remove a task from the persisted list by `id`. Returns `Ok(())` whether or not the id existed.
pub fn remove_scheduled_task(project_root: &Path, id: &str) -> std::io::Result<()> {
    let mut tasks = load_scheduled_tasks(project_root);
    let before = tasks.len();
    tasks.retain(|t| t.id != id);
    if tasks.len() != before {
        save_scheduled_tasks(project_root, &tasks)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_task(id: &str) -> ScheduledTask {
        ScheduledTask {
            id: id.to_owned(),
            schedule: "0 9 * * *".to_owned(),
            kind: ScheduledTaskKind::Cron,
            prompt: "catch-up".to_owned(),
            created_at_ms: 0,
            durable: true,
            label: None,
        }
    }

    #[test]
    fn roundtrip_save_load_normal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let tasks = vec![make_task("task-1"), make_task("task-2")];
        save_scheduled_tasks(root, &tasks).unwrap();
        let loaded = load_scheduled_tasks(root);
        assert_eq!(loaded, tasks);
    }

    #[test]
    fn upsert_adds_and_replaces_normal() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        upsert_scheduled_task(root, make_task("a")).unwrap();
        upsert_scheduled_task(root, make_task("b")).unwrap();
        let mut updated = make_task("a");
        updated.prompt = "updated".to_owned();
        upsert_scheduled_task(root, updated).unwrap();
        let loaded = load_scheduled_tasks(root);
        assert_eq!(loaded.len(), 2);
        assert_eq!(
            loaded.iter().find(|t| t.id == "a").unwrap().prompt,
            "updated"
        );
    }

    #[test]
    fn remove_nonexistent_is_noop_robust() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        upsert_scheduled_task(root, make_task("keep")).unwrap();
        remove_scheduled_task(root, "does-not-exist").unwrap();
        let loaded = load_scheduled_tasks(root);
        assert_eq!(loaded.len(), 1);
    }

    #[test]
    fn load_missing_file_returns_empty_robust() {
        let dir = tempfile::tempdir().unwrap();
        let loaded = load_scheduled_tasks(dir.path());
        assert!(loaded.is_empty());
    }
}
