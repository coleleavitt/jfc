//! Unified agent state, status, and role.
//!
//! Consolidates four structs that each tracked "a running agent" with 70–90%
//! field overlap (`BackgroundTask`, `BackgroundAgentInfo`,
//! `InProcessTeammateState`, `TeammateInfo`) and four near-identical status
//! enums into a single [`AgentState`] keyed by [`AgentStatus`] + [`AgentRole`].
//!
//! Role-specific data (team name, solver worktree, trust score) lives in the
//! [`AgentRole`] enum variant rather than as scattered `Option` fields, so the
//! type system enforces which data is meaningful for which kind of agent.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

use crate::id::AgentId;

mod trace;

/// Lifecycle status shared by every agent, regardless of backend.
///
/// Merges `AgentStatus` (engine), `BackgroundAgentStatus` (daemon),
/// `TeammateStatus` (swarm), and `TaskLifecycle` (engine types). `Idle` is the
/// teammate-specific "waiting for a message" state; every other variant is
/// common to all backends.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    /// Spawned but not yet started work.
    #[default]
    Pending,
    /// Actively streaming / executing tools.
    Running,
    /// Alive but waiting for input (teammates between turns).
    Idle,
    /// Finished successfully.
    Completed,
    /// Finished with an error.
    Failed,
    /// Aborted by the user or a takeover.
    Cancelled,
}

impl AgentStatus {
    /// Whether this is a terminal state (no further transitions expected).
    pub fn is_terminal(self) -> bool {
        let terminal = matches!(
            self,
            AgentStatus::Completed | AgentStatus::Failed | AgentStatus::Cancelled
        );
        trace::status_classification("agent.status.is_terminal", self, terminal);
        terminal
    }

    /// Whether the agent is still alive (consuming resources).
    pub fn is_active(self) -> bool {
        let active = matches!(
            self,
            AgentStatus::Pending | AgentStatus::Running | AgentStatus::Idle
        );
        trace::status_classification("agent.status.is_active", self, active);
        active
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentStatus::Pending => "pending",
            AgentStatus::Running => "running",
            AgentStatus::Idle => "idle",
            AgentStatus::Completed => "completed",
            AgentStatus::Failed => "failed",
            AgentStatus::Cancelled => "cancelled",
        }
    }
}

/// What *kind* of agent this is, plus the data that only that kind needs.
///
/// This is the key consolidation: instead of separate structs per backend, the
/// role enum carries the backend-specific payload. A `Solo` agent has no extra
/// data; a `Teammate` carries its team name; a `Solver` carries its bounty and
/// worktree; and so on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentRole {
    /// A one-shot subagent (the `Task` tool), in-process or detached.
    Solo,
    /// A long-lived named teammate in a team/swarm.
    Teammate { team_name: String },
    /// An economy solver competing on a bounty in an isolated worktree.
    Solver {
        bounty_id: String,
        worktree: Option<PathBuf>,
    },
    /// An economy validator adversarially checking a solver's solution.
    Validator { bounty_id: String },
    /// A council seat: one model in a parallel multi-model deliberation.
    Council { council_id: String },
}

impl AgentRole {
    /// Short label for UI grouping (e.g. the roster panel headers).
    pub fn label(&self) -> &'static str {
        let label = match self {
            AgentRole::Solo => "solo",
            AgentRole::Teammate { .. } => "teammate",
            AgentRole::Solver { .. } => "solver",
            AgentRole::Validator { .. } => "validator",
            AgentRole::Council { .. } => "council",
        };
        linkscope::record_items(trace::role_metric_label(label), 1);
        label
    }

    /// The team this agent belongs to, if any.
    pub fn team_name(&self) -> Option<&str> {
        let value = match self {
            AgentRole::Teammate { team_name } => Some(team_name.as_str()),
            _ => None,
        };
        trace::role_accessor("agent.role.team_name", self, value.map(str::len));
        value
    }

    /// The bounty this agent is working on, if any.
    pub fn bounty_id(&self) -> Option<&str> {
        let value = match self {
            AgentRole::Solver { bounty_id, .. } | AgentRole::Validator { bounty_id } => {
                Some(bounty_id.as_str())
            }
            _ => None,
        };
        trace::role_accessor("agent.role.bounty_id", self, value.map(str::len));
        value
    }
}

/// The single record describing one agent's full state.
///
/// UI-only data that used to live on `BackgroundTask` (rendered message parts,
/// chat history) is intentionally *not* here — the render layer keeps that
/// keyed by `AgentId`. This struct is the canonical, serializable truth about
/// an agent's identity, lifecycle, and progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub id: AgentId,
    pub status: AgentStatus,
    pub role: AgentRole,
    pub description: String,

    // ── Progress (shared by all backends) ───────────────────────────────
    #[serde(default)]
    pub token_count: u64,
    #[serde(default)]
    pub tool_use_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,

    // ── Lifecycle ───────────────────────────────────────────────────────
    pub started_at: SystemTime,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<SystemTime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,

    // ── Optional role-flavored fields ───────────────────────────────────
    /// Trust score for economy agents (solvers/validators). `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust_score: Option<i8>,
    /// Why a teammate is idle (e.g. "waiting for leader"). `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idle_reason: Option<String>,
}

impl AgentState {
    /// Create a freshly-spawned agent in [`AgentStatus::Pending`].
    pub fn new(id: AgentId, role: AgentRole, description: impl Into<String>) -> Self {
        let _linkscope_state = linkscope::phase("agent.state.new");
        let description = description.into();
        let state = Self {
            id,
            status: AgentStatus::Pending,
            role,
            description,
            token_count: 0,
            tool_use_count: 0,
            last_tool: None,
            model: None,
            started_at: SystemTime::now(),
            completed_at: None,
            error: None,
            summary: None,
            trust_score: None,
            idle_reason: None,
        };
        trace::state("agent.state.new.detail", &state);
        state
    }

    /// Mark the agent terminally completed with an optional summary.
    pub fn complete(&mut self, summary: Option<String>) {
        let _linkscope_complete = linkscope::phase("agent.state.complete");
        self.status = AgentStatus::Completed;
        self.completed_at = Some(SystemTime::now());
        self.summary = summary;
        trace::state("agent.state.complete.detail", self);
    }

    /// Mark the agent terminally failed with an error message.
    pub fn fail(&mut self, error: impl Into<String>) {
        let _linkscope_fail = linkscope::phase("agent.state.fail");
        self.status = AgentStatus::Failed;
        self.completed_at = Some(SystemTime::now());
        self.error = Some(error.into());
        trace::state("agent.state.fail.detail", self);
    }

    /// Mark the agent cancelled (user abort / takeover).
    pub fn cancel(&mut self) {
        let _linkscope_cancel = linkscope::phase("agent.state.cancel");
        self.status = AgentStatus::Cancelled;
        self.completed_at = Some(SystemTime::now());
        trace::state("agent.state.cancel.detail", self);
    }
}

/// The result an agent reports when it finishes.
///
/// Replaces the engine's `AgentResult` and the economy's per-solver token
/// reporting with one shape consumed by `AgentRegistry::complete`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub id: AgentId,
    pub output: String,
    #[serde(default)]
    pub tokens_used: u64,
    #[serde(default)]
    pub elapsed_ms: u64,
    /// Unified diff produced by a solver, if any (drives bounty settlement).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub patch: Option<String>,
}

#[cfg(test)]
mod tests;
