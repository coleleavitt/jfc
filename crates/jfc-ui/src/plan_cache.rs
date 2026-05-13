//! Plan reuse cache — stores successful task plans for future retrieval.
//!
//! When the task factory completes a full plan (all tasks done), the plan
//! is fingerprinted and cached. Future requests with similar fingerprints
//! can retrieve the cached plan DAG as a starting point.
//!
//! Storage: `~/.config/jfc/plans/` directory, one JSON file per plan entry.
//! Best-effort: failures are silent.

#![allow(dead_code)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::tasks::{Task, TaskKind, TaskRisk};

/// A cached plan entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanEntry {
    /// Fingerprint of the original request that produced this plan.
    pub request_fingerprint: String,
    /// Short summary of what the plan does.
    pub summary: String,
    /// The task DAG snapshot at completion.
    pub tasks: Vec<PlanTask>,
    /// Whether the plan succeeded (all tasks completed).
    pub outcome: PlanOutcome,
    /// When this plan was cached.
    pub created_at_ms: u64,
}

/// Lightweight task snapshot for the cache (no runtime state).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanTask {
    pub subject: String,
    pub description: String,
    pub blocked_by: Vec<String>,
    pub acceptance_criteria: Option<String>,
    pub verification_command: Option<String>,
    pub risk: Option<TaskRisk>,
    pub kind: Option<TaskKind>,
    pub parent_id: Option<String>,
}

/// Outcome of a cached plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanOutcome {
    Success,
    PartialSuccess,
    Failed,
}

/// Directory where plan caches are stored.
fn plans_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("plans")
}

/// Save a completed plan to the cache. Best-effort — failures are logged.
pub fn save_plan(summary: &str, tasks: &[Task], outcome: PlanOutcome) {
    let fingerprint = fingerprint_tasks(tasks);
    let entry = PlanEntry {
        request_fingerprint: fingerprint.clone(),
        summary: summary.to_owned(),
        tasks: tasks.iter().map(task_to_plan_task).collect(),
        outcome,
        created_at_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
    };

    let dir = plans_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let filename = format!("{}.json", &fingerprint[..fingerprint.len().min(16)]);
    let path = dir.join(filename);
    if let Ok(json) = serde_json::to_string_pretty(&entry) {
        let _ = std::fs::write(&path, json);
        tracing::debug!(
            target: "jfc::plan_cache",
            fingerprint = %entry.request_fingerprint,
            tasks = entry.tasks.len(),
            "saved plan to cache"
        );
    }
}

/// Find a similar cached plan by subject keywords. Returns None if no
/// match is found or the cache is empty. Uses a simple Jaccard coefficient
/// on whitespace-split subject words.
pub fn find_similar_plan(query: &str) -> Option<PlanEntry> {
    let dir = plans_dir();
    let entries = std::fs::read_dir(&dir).ok()?;
    let query_words: std::collections::HashSet<&str> =
        query.split_whitespace().map(|w| w.to_lowercase().leak() as &str).collect();

    if query_words.is_empty() {
        return None;
    }

    let mut best: Option<(f64, PlanEntry)> = None;

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let content = std::fs::read_to_string(&path).ok()?;
        let plan: PlanEntry = serde_json::from_str(&content).ok()?;

        // Only reuse successful plans
        if plan.outcome != PlanOutcome::Success {
            continue;
        }

        // Compute similarity between query and plan summary + task subjects
        let plan_text = format!(
            "{} {}",
            plan.summary,
            plan.tasks.iter().map(|t| t.subject.as_str()).collect::<Vec<_>>().join(" ")
        );
        let plan_words: std::collections::HashSet<&str> =
            plan_text.split_whitespace().collect();

        let intersection = query_words.intersection(&plan_words).count();
        let union = query_words.union(&plan_words).count();
        let jaccard = if union > 0 {
            intersection as f64 / union as f64
        } else {
            0.0
        };

        if jaccard > 0.3 {
            if best.as_ref().map_or(true, |(score, _)| jaccard > *score) {
                best = Some((jaccard, plan));
            }
        }
    }

    best.map(|(_, plan)| plan)
}

/// List all cached plans (most recent first).
pub fn list_plans() -> Vec<PlanEntry> {
    let dir = plans_dir();
    let entries = match std::fs::read_dir(&dir) {
        Ok(e) => e,
        Err(_) => return Vec::new(),
    };

    let mut plans: Vec<PlanEntry> = entries
        .flatten()
        .filter(|e| {
            e.path()
                .extension()
                .and_then(|ext| ext.to_str())
                == Some("json")
        })
        .filter_map(|e| {
            let content = std::fs::read_to_string(e.path()).ok()?;
            serde_json::from_str(&content).ok()
        })
        .collect();

    plans.sort_by(|a, b| b.created_at_ms.cmp(&a.created_at_ms));
    plans
}

/// Generate a fingerprint from a set of tasks (based on sorted subjects).
fn fingerprint_tasks(tasks: &[Task]) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    let mut subjects: Vec<&str> = tasks.iter().map(|t| t.subject.as_str()).collect();
    subjects.sort();

    let mut hasher = DefaultHasher::new();
    for s in &subjects {
        s.hash(&mut hasher);
    }
    format!("{:016x}", hasher.finish())
}

fn task_to_plan_task(task: &Task) -> PlanTask {
    PlanTask {
        subject: task.subject.clone(),
        description: task.description.clone(),
        blocked_by: task.blocked_by.iter().map(|id| id.as_str().to_owned()).collect(),
        acceptance_criteria: task.acceptance_criteria.clone(),
        verification_command: task.verification_command.clone(),
        risk: task.risk,
        kind: task.kind,
        parent_id: task.parent_id.as_ref().map(|id| id.as_str().to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStatus;

    #[test]
    fn fingerprint_is_stable_normal() {
        let tasks = vec![
            Task {
                id: crate::tasks::TaskId::new("t1"),
                subject: "Fix auth".into(),
                description: "".into(),
                active_form: None,
                status: TaskStatus::Completed,
                owner: None,
                blocks: Default::default(),
                blocked_by: Default::default(),
                metadata: None,
                created_at_ms: 0,
                acceptance_criteria: None,
                verification_command: None,
                risk: None,
                parent_id: None,
                kind: None,
            },
            Task {
                id: crate::tasks::TaskId::new("t2"),
                subject: "Add tests".into(),
                description: "".into(),
                active_form: None,
                status: TaskStatus::Completed,
                owner: None,
                blocks: Default::default(),
                blocked_by: Default::default(),
                metadata: None,
                created_at_ms: 0,
                acceptance_criteria: None,
                verification_command: None,
                risk: None,
                parent_id: None,
                kind: None,
            },
        ];
        let fp1 = fingerprint_tasks(&tasks);
        let fp2 = fingerprint_tasks(&tasks);
        assert_eq!(fp1, fp2);
    }
}
