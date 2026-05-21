//! Workflow task lifecycle + state tracking.

use std::path::PathBuf;
use tokio_util::sync::CancellationToken;

use super::meta::WorkflowMeta;

/// Status of a workflow run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowRunStatus {
    Running,
    Paused,
    Completed,
    Failed,
    Killed,
}

/// Progress entry for a single agent within the workflow.
#[derive(Debug, Clone)]
pub struct AgentProgress {
    pub index: u32,
    pub label: String,
    pub phase: Option<String>,
    pub status: AgentStatus,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentStatus {
    Queued,
    Running,
    Done,
    Failed,
    Skipped,
}

/// State of a running workflow task. Stored in the background task registry.
#[derive(Debug)]
pub struct WorkflowTaskState {
    pub run_id: String,
    pub script_path: Option<PathBuf>,
    pub meta: WorkflowMeta,
    pub status: WorkflowRunStatus,
    pub agent_count: u32,
    pub current_phase: Option<String>,
    pub progress: Vec<AgentProgress>,
    pub cancel: CancellationToken,
    pub started_at: std::time::Instant,
    pub ended_at: Option<std::time::Instant>,
    pub total_tokens: u64,
    pub total_tool_calls: u32,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub logs: Vec<String>,
}

impl WorkflowTaskState {
    pub fn new(run_id: String, meta: WorkflowMeta, cancel: CancellationToken) -> Self {
        Self {
            run_id,
            script_path: None,
            meta,
            status: WorkflowRunStatus::Running,
            agent_count: 0,
            current_phase: None,
            progress: Vec::new(),
            cancel,
            started_at: std::time::Instant::now(),
            ended_at: None,
            total_tokens: 0,
            total_tool_calls: 0,
            result: None,
            error: None,
            logs: Vec::new(),
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Killed
        )
    }
}

/// Generate a workflow run ID (wf_ + 8 random hex chars).
pub fn generate_run_id() -> String {
    let id = uuid::Uuid::new_v4();
    format!("wf_{}", &id.simple().to_string()[..8])
}
