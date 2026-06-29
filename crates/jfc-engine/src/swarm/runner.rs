//! In-process teammate runner: lifecycle, spawn, and event types.
//!
//! This module owns the **lifecycle surface** for the in-process teammate
//! execution mode (v126's `teammateMode: "in-process"`):
//!
//! - `start_teammate()` — spawn the background tokio task; delegates the actual
//!   agent loop to [`super::coordinator::run_teammate_loop`].
//! - `TeammateRunnerConfig` — shared config (identity, provider, prompts, store).
//! - `PollResult`, `TeammateExit`, `TeammateEvent` — types exchanged between
//!   the coordinator's loop and the spawn wrapper.
//! - `assign_teammate_color`, `teammate_task_id` — small helpers used by the
//!   spawn site in `stream::tool_dispatch`.
//! - `poll_leader_inbox` — leader-side counterpart that drains the leader's
//!   inbox for teammate-originated messages, called by the main event loop.
//!
//! The agent loop itself (run-turn → idle → poll → repeat) lives in
//! [`super::coordinator`]; the single-turn streaming + tool execution lives in
//! [`super::executor`]. This file deliberately holds none of that body — only
//! the spawn boilerplate and the message types both halves share.

use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

use super::mailbox;
use super::types::*;

/// Teammate colors matching v126's palette. Cycled through on each spawn.
const TEAMMATE_COLORS: &[&str] = &[
    "#4FC3F7", "#81C784", "#FFB74D", "#BA68C8", "#F06292", "#4DD0E1", "#AED581", "#FFD54F",
    "#7986CB", "#A1887F",
];

static COLOR_INDEX: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Assign a color for a new teammate (cycles through palette).
pub fn assign_teammate_color() -> String {
    let idx = COLOR_INDEX.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    TEAMMATE_COLORS[idx % TEAMMATE_COLORS.len()].to_owned()
}

/// Configuration for starting an in-process teammate.
pub struct TeammateRunnerConfig {
    pub identity: TeammateIdentity,
    pub prompt: String,
    pub description: String,
    pub model: Option<String>,
    pub agent_type: Option<String>,
    /// Provider for API calls. Shared with the leader.
    pub provider: std::sync::Arc<dyn jfc_provider::Provider>,
    /// Model ID to use for this teammate's API calls.
    pub model_id: jfc_provider::ModelId,
    /// System prompt additions (agent-specific + teammate addendum).
    pub system_prompt: Option<String>,
    /// Shared task list used by TaskCreate/TaskUpdate/TaskList/TaskDone.
    pub task_store: Option<std::sync::Arc<jfc_session::TaskStore>>,
}

/// Message types the runner can receive from its poll loop.
#[derive(Debug)]
pub enum PollResult {
    /// New message from leader or another teammate.
    NewMessage {
        message: String,
        from: String,
        color: Option<String>,
        summary: Option<String>,
    },
    /// Shutdown request received.
    ShutdownRequest {
        request: Option<ShutdownRequest>,
        original_message: String,
    },
    /// A task from the task list is available to claim.
    TaskAvailable { task_id: String, prompt: String },
    /// The teammate was aborted (lifecycle abort signal).
    Aborted,
}

/// Start an in-process teammate. Spawns a background tokio task that runs
/// the agent loop. Returns abort handle and task ID.
///
/// This is the main entry point called after spawn validation.
/// Single source of truth for the public task id of an in-process
/// teammate. The runner uses this internally; the dispatcher uses it
/// to register the matching `BackgroundTask` so streaming events
/// route correctly.
pub fn teammate_task_id(agent_id: &str) -> String {
    format!("teammate-{agent_id}")
}

pub fn start_teammate(
    config: TeammateRunnerConfig,
    event_tx: mpsc::UnboundedSender<TeammateEvent>,
) -> (String, watch::Sender<bool>) {
    let (abort_tx, abort_rx) = watch::channel(false);
    let task_id = format!("teammate-{}", config.identity.agent_id);

    let identity = config.identity.clone();
    let task_id_clone = task_id.clone();

    tokio::spawn(async move {
        let result =
            super::coordinator::run_teammate_loop(config, abort_rx, event_tx.clone()).await;

        match result {
            Ok(TeammateExit::Completed) => {
                debug!(
                    "[InProcessRunner] Teammate {} completed normally",
                    identity.agent_name
                );
                let _ = event_tx.send(TeammateEvent::Completed {
                    task_id: task_id_clone,
                    agent_id: identity.agent_id,
                });
            }
            Ok(TeammateExit::Cancelled) => {
                // Abort signal — either explicit (.send(true)) or the
                // watch::Sender was dropped. The previous version silently
                // mapped this to Completed, which lit up every teammate as
                // ": Done" in the UI before they did any work, because the
                // spawn site at stream.rs:1962 dropped the abort handle.
                debug!(
                    "[InProcessRunner] Teammate {} cancelled",
                    identity.agent_name
                );
                let _ = event_tx.send(TeammateEvent::Cancelled {
                    task_id: task_id_clone,
                    agent_id: identity.agent_id,
                });
            }
            Err(e) => {
                warn!(
                    "[InProcessRunner] Teammate {} failed: {e}",
                    identity.agent_name
                );
                let _ = event_tx.send(TeammateEvent::Failed {
                    task_id: task_id_clone,
                    agent_id: identity.agent_id,
                    error: e.to_string(),
                });
            }
        }
    });

    (task_id, abort_tx)
}

/// Why a teammate's run loop ended. Aborted vs naturally exhausted is a
/// meaningful distinction for the UI — aborted means "not done yet",
/// completed means "the agent decided it was finished".
#[derive(Debug)]
pub enum TeammateExit {
    Completed,
    Cancelled,
}

/// Events emitted by the teammate runner back to the leader.
#[derive(Debug, Clone)]
pub enum TeammateEvent {
    /// Teammate has gone idle (finished processing, waiting for next message).
    Idle {
        task_id: String,
        agent_id: String,
        agent_name: String,
        reason: Option<String>,
        summary: Option<String>,
    },
    /// Teammate is actively processing (status update for UI).
    Progress {
        task_id: String,
        agent_id: String,
        token_count: u64,
        tool_use_count: u64,
        last_tool: Option<String>,
        /// Model being used, for cost aggregation in the status bar.
        model_id: Option<String>,
        /// Incremental cost in USD since last progress event.
        cost_usd: Option<f64>,
    },
    /// Teammate completed and exited its loop.
    Completed { task_id: String, agent_id: String },
    /// Teammate's run loop was cancelled before natural completion —
    /// either by an explicit abort signal (ESC×2, kill button) or by the
    /// abort_tx watch::Sender being dropped. Distinct from Completed so
    /// the UI can label the row "Cancelled" instead of "Done".
    Cancelled { task_id: String, agent_id: String },
    /// Teammate encountered a fatal error.
    Failed {
        task_id: String,
        agent_id: String,
        error: String,
    },
    MessageSent {
        task_id: String,
        agent_id: String,
        from: String,
        to: String,
        text: String,
        summary: Option<String>,
    },
    /// One streaming-text delta from the teammate's current turn.
    /// The main loop translates this into `TaskEvent::AgentChunk` so
    /// the task panel fills live as the teammate streams. Without
    /// it, drilling into a running teammate showed "No messages yet"
    /// until the entire turn finished.
    TextDelta {
        task_id: String,
        agent_id: String,
        delta: String,
    },
}

// ─── Leader inbox polling ────────────────────────────────────────────────────

/// Check the leader's inbox for new messages from teammates.
/// Returns formatted messages ready to be injected into the conversation.
/// Called periodically by the main event loop (every LEADER_POLL_INTERVAL_MS).
pub async fn poll_leader_inbox(team_name: &str) -> Vec<IncomingTeammateMessage> {
    let messages = mailbox::read_mailbox(super::TEAM_LEAD_NAME, team_name).await;
    let mut incoming = Vec::new();

    for (idx, msg) in messages.iter().enumerate() {
        if msg.read {
            continue;
        }

        // Skip idle notifications — they're informational, not conversation content
        if mailbox::is_idle_notification(&msg.text) {
            // Mark as read so we don't re-process
            let _ = mailbox::mark_message_read(super::TEAM_LEAD_NAME, team_name, idx).await;
            continue;
        }

        // Format as teammate-message for conversation injection
        let formatted = format_teammate_message(
            &msg.from,
            &msg.text,
            msg.color.as_deref(),
            msg.summary.as_deref(),
        );

        incoming.push(IncomingTeammateMessage {
            from: msg.from.clone(),
            text: msg.text.clone(),
            formatted,
            color: msg.color.clone(),
            summary: msg.summary.clone(),
        });

        // Mark as read
        let _ = mailbox::mark_message_read(super::TEAM_LEAD_NAME, team_name, idx).await;
    }

    incoming
}

/// A message from a teammate ready for delivery to the leader's conversation.
#[derive(Debug, Clone)]
pub struct IncomingTeammateMessage {
    pub from: String,
    pub text: String,
    /// Pre-formatted `<teammate-message>` XML for conversation injection.
    pub formatted: String,
    pub color: Option<String>,
    pub summary: Option<String>,
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
