//! Team execution backend bridge.
//!
//! The swarm's [`start_teammate`](crate::swarm::runner::start_teammate) already
//! spawns a long-lived teammate loop and reports its lifecycle through
//! [`TeammateEvent`]s on an mpsc channel. [`TeamBackend`] is the adapter that
//! consumes those events and projects them onto the unified
//! [`AgentRegistryImpl`], so a teammate shows up in the same roster as every
//! other agent (solo, council, solver) instead of in a separate `TeamContext`.
//!
//! It does NOT replace the teammate loop — it observes it. The mapping is:
//!
//! | TeammateEvent      | Registry effect                          |
//! |--------------------|------------------------------------------|
//! | `Progress`         | `update_progress` (tokens, tools)        |
//! | `Idle`             | `update_status(Idle)` + idle reason      |
//! | `Completed`        | `complete`                               |
//! | `Cancelled`        | `update_status(Cancelled)`               |
//! | `Failed`           | `fail`                                   |
//!
//! Messaging events (`MessageSent`, `TextDelta`) are left to the existing UI
//! path; the registry only tracks lifecycle + progress.

use std::sync::Arc;

use jfc_agent::{AgentId, AgentRegistry, AgentResult, AgentStatus};
use tokio::sync::mpsc;

use super::registry_impl::AgentRegistryImpl;
use crate::swarm::runner::TeammateEvent;

/// Bridges swarm `TeammateEvent`s onto the unified registry.
pub struct TeamBackend {
    registry: Arc<AgentRegistryImpl>,
}

impl TeamBackend {
    pub fn new(registry: Arc<AgentRegistryImpl>) -> Self {
        Self { registry }
    }

    /// Resolve a swarm `agent_id` string to the registry's [`AgentId`].
    ///
    /// Teammates are registered under a display name equal to their swarm
    /// agent id, so name resolution is exact. Logs + returns `None` for an
    /// event whose agent isn't tracked (so callers can skip it).
    async fn resolve(&self, agent_id: &str) -> Option<AgentId> {
        let resolved = self.registry.resolve_name(agent_id).await;
        if resolved.is_none() {
            tracing::trace!(
                target: "jfc::agent::team",
                agent_id,
                "teammate event for unregistered agent (ignored)"
            );
        }
        resolved
    }

    /// Apply one teammate event to the registry. Returns `true` if the event
    /// matched a tracked agent.
    pub async fn apply(&self, event: &TeammateEvent) -> bool {
        match event {
            TeammateEvent::Progress {
                agent_id,
                token_count,
                tool_use_count,
                last_tool,
                ..
            } => {
                let Some(id) = self.resolve(agent_id).await else {
                    return false;
                };
                self.registry
                    .update_progress(&id, *token_count, *tool_use_count as u32, last_tool.clone())
                    .await;
                // Progress implies the teammate is actively working again.
                self.registry.update_status(&id, AgentStatus::Running).await;
                true
            }
            TeammateEvent::Idle {
                agent_id, reason, ..
            } => {
                let Some(id) = self.resolve(agent_id).await else {
                    return false;
                };
                self.registry.set_idle(&id, reason.clone()).await;
                true
            }
            TeammateEvent::Completed { agent_id, .. } => {
                let Some(id) = self.resolve(agent_id).await else {
                    return false;
                };
                let tokens = self
                    .registry
                    .state(&id)
                    .await
                    .map(|s| s.token_count)
                    .unwrap_or(0);
                self.registry
                    .complete(
                        &id,
                        AgentResult {
                            id: id.clone(),
                            output: String::new(),
                            tokens_used: tokens,
                            elapsed_ms: 0,
                            patch: None,
                        },
                    )
                    .await;
                true
            }
            TeammateEvent::Cancelled { agent_id, .. } => {
                let Some(id) = self.resolve(agent_id).await else {
                    return false;
                };
                self.registry
                    .update_status(&id, AgentStatus::Cancelled)
                    .await;
                true
            }
            TeammateEvent::Failed {
                agent_id, error, ..
            } => {
                let Some(id) = self.resolve(agent_id).await else {
                    return false;
                };
                self.registry.fail(&id, error.clone()).await;
                true
            }
            // Messaging deltas are not lifecycle events.
            TeammateEvent::MessageSent { .. } | TeammateEvent::TextDelta { .. } => false,
        }
    }

    /// Spawn a task that drains `rx`, applying every event to the registry until
    /// the channel closes. Returns the join handle.
    pub fn pump(
        self: Arc<Self>,
        mut rx: mpsc::UnboundedReceiver<TeammateEvent>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            while let Some(event) = rx.recv().await {
                self.apply(&event).await;
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_agent::{AgentRole, AgentState};

    async fn backend_with_teammate(swarm_id: &str) -> (Arc<TeamBackend>, AgentId) {
        let reg = Arc::new(AgentRegistryImpl::new());
        let id = AgentId::named(swarm_id);
        reg.register(AgentState::new(
            id.clone(),
            AgentRole::Teammate {
                team_name: "alpha".into(),
            },
            "teammate work",
        ))
        .await;
        (Arc::new(TeamBackend::new(reg)), id)
    }

    #[tokio::test]
    async fn progress_updates_tokens_and_runs_normal() {
        let (backend, id) = backend_with_teammate("researcher").await;
        let matched = backend
            .apply(&TeammateEvent::Progress {
                task_id: "teammate-researcher".into(),
                agent_id: "researcher".into(),
                token_count: 1234,
                tool_use_count: 7,
                last_tool: Some("Grep".into()),
                model_id: None,
                cost_usd: None,
            })
            .await;
        assert!(matched);
        let state = backend.registry.state(&id).await.unwrap();
        assert_eq!(state.token_count, 1234);
        assert_eq!(state.tool_use_count, 7);
        assert_eq!(state.status, AgentStatus::Running);
    }

    #[tokio::test]
    async fn idle_sets_idle_status_normal() {
        let (backend, id) = backend_with_teammate("researcher").await;
        backend
            .apply(&TeammateEvent::Idle {
                task_id: "teammate-researcher".into(),
                agent_id: "researcher".into(),
                agent_name: "researcher".into(),
                reason: Some("waiting for leader".into()),
                summary: None,
            })
            .await;
        assert_eq!(backend.registry.status(&id).await, Some(AgentStatus::Idle));
    }

    #[tokio::test]
    async fn completed_marks_terminal_normal() {
        let (backend, id) = backend_with_teammate("researcher").await;
        backend
            .apply(&TeammateEvent::Completed {
                task_id: "teammate-researcher".into(),
                agent_id: "researcher".into(),
            })
            .await;
        assert_eq!(
            backend.registry.status(&id).await,
            Some(AgentStatus::Completed)
        );
    }

    #[tokio::test]
    async fn failed_records_error_robust() {
        let (backend, id) = backend_with_teammate("researcher").await;
        backend
            .apply(&TeammateEvent::Failed {
                task_id: "teammate-researcher".into(),
                agent_id: "researcher".into(),
                error: "provider error".into(),
            })
            .await;
        assert_eq!(
            backend.registry.status(&id).await,
            Some(AgentStatus::Failed)
        );
        assert_eq!(
            backend.registry.state(&id).await.unwrap().error.as_deref(),
            Some("provider error")
        );
    }

    #[tokio::test]
    async fn unknown_agent_event_is_ignored_robust() {
        let (backend, _id) = backend_with_teammate("researcher").await;
        let matched = backend
            .apply(&TeammateEvent::Completed {
                task_id: "teammate-ghost".into(),
                agent_id: "ghost".into(),
            })
            .await;
        assert!(!matched);
    }

    #[tokio::test]
    async fn pump_drains_channel_until_closed_normal() {
        let (backend, id) = backend_with_teammate("researcher").await;
        let (tx, rx) = mpsc::unbounded_channel();
        let handle = backend.clone().pump(rx);

        tx.send(TeammateEvent::Completed {
            task_id: "teammate-researcher".into(),
            agent_id: "researcher".into(),
        })
        .unwrap();
        drop(tx); // close channel so pump exits

        handle.await.unwrap();
        assert_eq!(
            backend.registry.status(&id).await,
            Some(AgentStatus::Completed)
        );
    }
}
