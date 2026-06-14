use std::borrow::Borrow;
use std::collections::BTreeSet;
use std::fmt;
use std::ops::Deref;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TodoTaskId(String);

impl TodoTaskId {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for TodoTaskId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for TodoTaskId {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl Deref for TodoTaskId {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for TodoTaskId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq<&str> for TodoTaskId {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<TodoTaskId> for &str {
    fn eq(&self, other: &TodoTaskId) -> bool {
        *self == other.as_str()
    }
}

impl From<String> for TodoTaskId {
    fn from(value: String) -> Self {
        Self(value)
    }
}

impl From<&str> for TodoTaskId {
    fn from(value: &str) -> Self {
        Self(value.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskError {
    UnknownTask { id: TodoTaskId },
    UnknownDependency { id: TodoTaskId },
    SelfCycle { id: TodoTaskId },
    DependencyCycle { path: Vec<TodoTaskId> },
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
                    .map(TodoTaskId::as_str)
                    .collect::<Vec<_>>()
                    .join(" -> ");
                write!(f, "blockedBy would create dependency cycle: {chain}")
            }
        }
    }
}

impl std::error::Error for TaskError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskRisk {
    Low,
    Medium,
    High,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Milestone,
    Task,
    Check,
    Decision,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    #[default]
    Pending,
    /// Accepted into a run queue but not yet started (e.g. waiting for a worker
    /// or a dependency window). Distinct from `Pending`, which is "unscheduled".
    Queued,
    InProgress,
    /// Cannot start because a `blocked_by` dependency is unmet. Non-terminal —
    /// transitions back to `Pending`/`Queued` once unblocked.
    Blocked,
    Completed,
    Failed,
    /// Explicitly cancelled by the user/system before completing (distinct from
    /// `Deleted`, which removes the task from the working set entirely).
    Cancelled,
    Deleted,
}

impl TaskStatus {
    /// Canonical single-cell status glyph. This is the single source of truth
    /// for the task-status vocabulary — UI call sites (task panel, `/task`
    /// listings, cascade summaries) must use this rather than re-deriving
    /// their own glyph sets, which previously drifted apart.
    pub fn glyph(self) -> &'static str {
        match self {
            TaskStatus::Pending => "□",
            TaskStatus::Queued => "▢",
            TaskStatus::InProgress => "▣",
            TaskStatus::Blocked => "◫",
            TaskStatus::Completed => "✓",
            TaskStatus::Failed | TaskStatus::Deleted | TaskStatus::Cancelled => "✗",
        }
    }

    /// Lowercase status word matching the serde representation
    /// (`pending`, `in_progress`, …). Used for status labels in listings.
    pub fn label(self) -> &'static str {
        match self {
            TaskStatus::Pending => "pending",
            TaskStatus::Queued => "queued",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Blocked => "blocked",
            TaskStatus::Completed => "completed",
            TaskStatus::Failed => "failed",
            TaskStatus::Cancelled => "cancelled",
            TaskStatus::Deleted => "deleted",
        }
    }

    /// Whether this status is terminal (no further work happens).
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            TaskStatus::Completed
                | TaskStatus::Failed
                | TaskStatus::Cancelled
                | TaskStatus::Deleted
        )
    }

    /// Whether the task is actively counted as "open work" (shows in the live
    /// working set and blocks turn-completion claims).
    pub fn is_open(self) -> bool {
        matches!(
            self,
            TaskStatus::Pending | TaskStatus::Queued | TaskStatus::InProgress | TaskStatus::Blocked
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: TodoTaskId,
    #[serde(default)]
    pub subject: String,
    #[serde(default)]
    pub description: String,
    #[serde(
        default,
        rename = "activeForm",
        alias = "active_form",
        skip_serializing_if = "Option::is_none"
    )]
    pub active_form: Option<String>,
    #[serde(default)]
    pub status: TaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub blocks: BTreeSet<TodoTaskId>,
    #[serde(default, rename = "blockedBy")]
    pub blocked_by: BTreeSet<TodoTaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
    #[serde(default)]
    pub created_at_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance_criteria: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification_command: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub risk: Option<TaskRisk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<TodoTaskId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<TaskKind>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<u8>,
    /// Per-task reasoning-effort override (e.g. "low", "medium", "high",
    /// "max"). When the factory auto-continues this task, this effort is
    /// applied for the turn. Precedence mirrors subagents: Task.effort >
    /// AgentDef.effort > global. `None` = inherit the session default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    /// Per-task model override (e.g. "claude-opus-4", "claude-haiku-4-5").
    /// When the factory auto-continues this task, it requests this model for
    /// the turn. `None` = use the session's active model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// How many times execution of this task has failed. Drives bounded
    /// retry (transient failures re-queue until this hits `max_attempts`)
    /// and the rework/attempts factory metrics. See
    /// [`crate::task_store`] recovery logic.
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub attempt_count: u32,
}

fn is_zero_u32(n: &u32) -> bool {
    *n == 0
}

impl Task {
    pub fn spinner_text(&self) -> &str {
        self.active_form.as_deref().unwrap_or(&self.subject)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskValidation {
    pub orphaned_tasks: Vec<TodoTaskId>,
    pub blocked_forever: Vec<TodoTaskId>,
    pub no_verification_path: Vec<TodoTaskId>,
    pub duplicate_subjects: Vec<String>,
    pub parallelization_opportunities: Vec<String>,
    /// Dependency cycles in the `blocked_by` graph, each inner `Vec` a Tarjan
    /// strongly-connected component (size > 1, or a single task that blocks
    /// itself). The per-edge guard in `update()` rejects *new* cycles, but a
    /// graph loaded from disk or hand-edited JSON can still contain one — this
    /// surfaces it. Mirrors Terraform `internal/dag/tarjan.go`.
    #[serde(default)]
    pub dependency_cycles: Vec<Vec<TodoTaskId>>,
    /// Tasks transitively blocked by a Failed/Deleted (or missing) task —
    /// Terraform's `upstreamFailed`. They are stuck through no fault of their
    /// own, so a scheduler must not report them as their *own* failures. This
    /// is the transitive superset of `blocked_forever`.
    #[serde(default)]
    pub upstream_failed: Vec<TodoTaskId>,
    /// The ready frontier: pending, unowned tasks whose blockers are all
    /// completed — the set safe to dispatch in parallel right now. Mirrors the
    /// Terraform ready-set DAG walk.
    #[serde(default)]
    pub ready: Vec<TodoTaskId>,
    pub total_tasks: usize,
    pub pending_count: usize,
    pub in_progress_count: usize,
    pub completed_count: usize,
    pub failed_count: usize,
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
    pub acceptance_criteria: Option<String>,
    pub verification_command: Option<String>,
    pub risk: Option<TaskRisk>,
    pub parent_id: Option<TodoTaskId>,
    pub kind: Option<TaskKind>,
    pub tags: Option<Vec<String>>,
    pub priority: Option<u8>,
    pub effort: Option<String>,
    pub model: Option<String>,
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TaskCounts {
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
}

/// Factory throughput + quality telemetry (Morescient GAI, arXiv:2406.04710):
/// measured feedback the scheduler and the user can read to reason about the
/// production line — not just raw counts, but rates and rework.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct FactoryMetrics {
    /// Tasks in each terminal/active state.
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
    pub failed: usize,
    /// Replan tasks (tagged `replan`) — the rework backlog.
    pub replan_tasks: usize,
    /// Tasks that were retried at least once (`attempt_count > 0`).
    pub retried_tasks: usize,
    /// Sum of all `attempt_count`s across tasks — total wasted/extra runs.
    pub total_attempts: u32,
    /// Tasks that needed more than one attempt to complete (or are still
    /// trying). A proxy for "hard" tasks the planner under-decomposed.
    pub multi_attempt_tasks: usize,
}

impl FactoryMetrics {
    /// Total tasks ever created (excluding deleted), = the denominator for
    /// success/rework rates.
    pub fn total(&self) -> usize {
        self.pending + self.in_progress + self.completed + self.failed
    }

    /// Completed / (completed + failed). 1.0 when nothing has failed; `None`
    /// when nothing has reached a terminal state yet.
    pub fn success_rate(&self) -> Option<f64> {
        let terminal = self.completed + self.failed;
        (terminal > 0).then(|| self.completed as f64 / terminal as f64)
    }

    /// Rework ratio: replan tasks / total. High means the plan keeps needing
    /// revision — a signal the planner should decompose more carefully.
    pub fn rework_ratio(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        self.replan_tasks as f64 / total as f64
    }

    /// Average attempts across all non-deleted tasks (≥1.0 once any task has
    /// been worked). Rises with flakiness / under-decomposition.
    pub fn avg_attempts(&self) -> f64 {
        let total = self.total();
        if total == 0 {
            return 0.0;
        }
        // Each task is attempted at least conceptually once; attempt_count
        // counts *extra* tries, so average extra retries per task.
        self.total_attempts as f64 / total as f64
    }
}

#[cfg(test)]
mod status_tests {
    use super::TaskStatus;

    // Normal: the CC 2.1.170-parity statuses serialize to snake_case wire form.
    #[test]
    fn new_statuses_serde_roundtrip_normal() {
        for (status, wire) in [
            (TaskStatus::Queued, "\"queued\""),
            (TaskStatus::Blocked, "\"blocked\""),
            (TaskStatus::Cancelled, "\"cancelled\""),
        ] {
            let json = serde_json::to_string(&status).unwrap();
            assert_eq!(json, wire, "{status:?} wire form");
            let back: TaskStatus = serde_json::from_str(wire).unwrap();
            assert_eq!(back, status, "{wire} round-trip");
            assert_eq!(status.label(), wire.trim_matches('"'), "{status:?} label");
        }
    }

    // Robust: terminal vs open classification covers every variant.
    #[test]
    fn terminal_and_open_partition_robust() {
        use TaskStatus::*;
        for s in [Pending, Queued, InProgress, Blocked] {
            assert!(s.is_open(), "{s:?} should be open");
            assert!(!s.is_terminal(), "{s:?} should not be terminal");
        }
        for s in [Completed, Failed, Cancelled, Deleted] {
            assert!(s.is_terminal(), "{s:?} should be terminal");
            assert!(!s.is_open(), "{s:?} should not be open");
        }
    }
}
