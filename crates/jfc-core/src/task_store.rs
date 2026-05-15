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
    InProgress,
    Completed,
    Failed,
    Deleted,
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
}

#[derive(Debug, Default, Clone, Copy)]
pub struct TaskCounts {
    pub pending: usize,
    pub in_progress: usize,
    pub completed: usize,
}
