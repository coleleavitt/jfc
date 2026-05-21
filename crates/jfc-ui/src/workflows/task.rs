//! Workflow task lifecycle + state tracking.
//!
//! `WorkflowTaskState` is the canonical in-memory record for a running or
//! completed workflow.  It is stored on `BackgroundTask.workflow_progress` and
//! updated by `AppEvent::WorkflowProgress` handlers in the event loop.
//!
//! The types here are fully implemented but some are not yet wired into the
//! event loop (pending t154/t156). Suppress dead_code until the wiring lands.
#![allow(dead_code)]

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

impl WorkflowRunStatus {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            WorkflowRunStatus::Completed | WorkflowRunStatus::Failed | WorkflowRunStatus::Killed
        )
    }
}

impl std::fmt::Display for WorkflowRunStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Running => f.write_str("running"),
            Self::Paused => f.write_str("paused"),
            Self::Completed => f.write_str("completed"),
            Self::Failed => f.write_str("failed"),
            Self::Killed => f.write_str("killed"),
        }
    }
}

/// Per-agent progress row inside a workflow run.
#[derive(Debug, Clone)]
pub struct AgentProgress {
    /// 1-based monotonic index (matches `AgentRequest::index`).
    pub index: u32,
    /// Short display label (prompt prefix or explicit `label` opt).
    pub label: String,
    /// Phase this agent was launched in, if `phase()` was called before it.
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

impl std::fmt::Display for AgentStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Queued => f.write_str("queued"),
            Self::Running => f.write_str("running"),
            Self::Done => f.write_str("done"),
            Self::Failed => f.write_str("failed"),
            Self::Skipped => f.write_str("skipped"),
        }
    }
}

/// Live progress snapshot for a workflow background task.
///
/// Stored on `BackgroundTask::workflow_progress` and updated incrementally
/// via `AppEvent::WorkflowProgress` events emitted by the runner.
#[derive(Debug, Clone)]
pub struct WorkflowTaskProgress {
    pub run_id: String,
    pub script_path: Option<PathBuf>,
    pub meta: WorkflowMeta,
    pub status: WorkflowRunStatus,
    /// Current phase title (last `phase()` call from the script).
    pub current_phase: Option<String>,
    /// All agent rows seen so far, in dispatch order.
    pub agents: Vec<AgentProgress>,
    /// Log messages emitted by `log()` and internal phase/progress signals.
    pub logs: Vec<String>,
    /// Total unique agents dispatched (excludes cache hits).
    pub total_dispatched: u32,
    /// Cache hits (agents replayed from the resume journal).
    pub cache_hits: u32,
    pub started_at: std::time::Instant,
    pub ended_at: Option<std::time::Instant>,
    /// Accumulated output tokens across all sub-agents.
    pub total_tokens: u64,
}

impl WorkflowTaskProgress {
    pub fn new(run_id: String, meta: WorkflowMeta) -> Self {
        Self {
            run_id,
            script_path: None,
            meta,
            status: WorkflowRunStatus::Running,
            current_phase: None,
            agents: Vec::new(),
            logs: Vec::new(),
            total_dispatched: 0,
            cache_hits: 0,
            started_at: std::time::Instant::now(),
            ended_at: None,
            total_tokens: 0,
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }

    /// How many agents are currently `Running`.
    pub fn running_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| a.status == AgentStatus::Running)
            .count()
    }

    /// How many agents have finished (Done or Failed).
    pub fn finished_count(&self) -> usize {
        self.agents
            .iter()
            .filter(|a| matches!(a.status, AgentStatus::Done | AgentStatus::Failed))
            .count()
    }
}

/// Full state record for a running workflow (kept alive for the Cancel path).
pub struct WorkflowTaskState {
    pub progress: WorkflowTaskProgress,
    pub cancel: CancellationToken,
}

impl WorkflowTaskState {
    pub fn new(run_id: String, meta: WorkflowMeta, cancel: CancellationToken) -> Self {
        Self {
            progress: WorkflowTaskProgress::new(run_id, meta),
            cancel,
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.progress.is_terminal()
    }
}

/// Generate a workflow run ID: `wf_` + 8 random hex chars.
pub fn generate_run_id() -> String {
    let id = uuid::Uuid::new_v4();
    format!("wf_{}", &id.simple().to_string()[..8])
}
