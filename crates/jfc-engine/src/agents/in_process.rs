//! In-process execution backend.
//!
//! [`InProcessBackend`] runs an agent's work as a `tokio::spawn`ed task in the
//! current process and drives its [`AgentState`] lifecycle through the
//! [`AgentRegistryImpl`]: `Pending → Running → Completed/Failed/Cancelled`.
//!
//! This is the backend for solo subagents (the `Task` tool with
//! `run_in_background: false`) and for economy solvers/validators that run in
//! the same process. The work itself is supplied as a future by the caller, so
//! this module stays decoupled from the concrete agent loop (streaming,
//! tool dispatch) that lives in `stream.rs` / `swarm/`.

use std::future::Future;
use std::sync::Arc;

use jfc_agent::{AgentId, AgentRegistry, AgentResult, AgentStatus};
use tokio::sync::watch;

use super::registry_impl::AgentRegistryImpl;

/// Outcome a work future reports back to the backend.
pub enum WorkOutcome {
    /// Finished successfully with this result.
    Done(AgentResult),
    /// Failed with this error message.
    Failed(String),
}

/// Spawns agent work in-process and reports lifecycle to the registry.
pub struct InProcessBackend {
    registry: Arc<AgentRegistryImpl>,
}

impl InProcessBackend {
    pub fn new(registry: Arc<AgentRegistryImpl>) -> Self {
        Self { registry }
    }

    /// Launch `work` for an already-registered agent `id`.
    ///
    /// Transitions the agent to `Running`, installs an abort handle, then spawns
    /// a tokio task that awaits `work` and records the terminal result. The work
    /// future receives an abort receiver so cooperative cancellation is possible;
    /// if the abort fires first, the agent is marked `Cancelled`.
    ///
    /// Returns immediately with the spawned task's `JoinHandle`.
    pub fn launch<F, Fut>(&self, id: AgentId, work: F) -> tokio::task::JoinHandle<()>
    where
        F: FnOnce(watch::Receiver<bool>) -> Fut + Send + 'static,
        Fut: Future<Output = WorkOutcome> + Send + 'static,
    {
        let (abort_tx, abort_rx) = watch::channel(false);
        let registry = self.registry.clone();
        tokio::spawn(async move {
            registry.set_abort_tx(&id, abort_tx).await;
            registry.update_status(&id, AgentStatus::Running).await;

            let mut abort_probe = abort_rx.clone();
            let outcome = tokio::select! {
                // Bias the select so a pending abort is observed before a work
                // future that is also ready in the same poll — cancellation
                // takes precedence over a racing completion.
                biased;
                _ = abort_probe.changed() => {
                    registry.update_status(&id, AgentStatus::Cancelled).await;
                    tracing::debug!(
                        target: "jfc::agent::in_process",
                        agent = %id,
                        "agent cancelled via abort signal"
                    );
                    return;
                }
                outcome = work(abort_rx) => outcome,
            };

            // `complete`/`fail` no-op if a terminal status (e.g. a racing
            // Cancelled) was already recorded, so a late finish can't resurrect
            // a cancelled agent.
            match outcome {
                WorkOutcome::Done(result) => registry.complete(&id, result).await,
                WorkOutcome::Failed(error) => registry.fail(&id, error).await,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_agent::{AgentRegistry, AgentRole, AgentState};

    async fn registry_with_agent(name: &str) -> (Arc<AgentRegistryImpl>, AgentId) {
        let reg = Arc::new(AgentRegistryImpl::new());
        let id = AgentId::named(name);
        reg.register(AgentState::new(id.clone(), AgentRole::Solo, "work"))
            .await;
        (reg, id)
    }

    #[tokio::test]
    async fn launch_runs_to_completion_normal() {
        let (reg, id) = registry_with_agent("worker").await;
        let backend = InProcessBackend::new(reg.clone());

        let id2 = id.clone();
        backend.launch(id.clone(), move |_abort| async move {
            WorkOutcome::Done(AgentResult {
                id: id2,
                output: "finished".into(),
                tokens_used: 100,
                elapsed_ms: 5,
                patch: None,
            })
        });

        let result = reg.wait(&id).await.unwrap();
        assert_eq!(result.output, "finished");
        assert_eq!(reg.status(&id).await, Some(AgentStatus::Completed));
    }

    #[tokio::test]
    async fn launch_records_failure_robust() {
        let (reg, id) = registry_with_agent("failer").await;
        let backend = InProcessBackend::new(reg.clone());

        backend.launch(id.clone(), move |_abort| async move {
            WorkOutcome::Failed("boom".into())
        });

        reg.wait(&id).await.unwrap();
        assert_eq!(reg.status(&id).await, Some(AgentStatus::Failed));
        assert_eq!(reg.state(&id).await.unwrap().error.as_deref(), Some("boom"));
    }

    #[tokio::test]
    async fn launch_transitions_to_running_normal() {
        let (reg, id) = registry_with_agent("runner").await;
        let backend = InProcessBackend::new(reg.clone());

        // Work that blocks until we drop the gate, so we can observe Running.
        let (gate_tx, gate_rx) = tokio::sync::oneshot::channel::<()>();
        let id2 = id.clone();
        backend.launch(id.clone(), move |_abort| async move {
            let _ = gate_rx.await;
            WorkOutcome::Done(AgentResult {
                id: id2,
                output: "ok".into(),
                tokens_used: 0,
                elapsed_ms: 0,
                patch: None,
            })
        });

        // Spin until the spawned task has run up to the gate (set Running).
        for _ in 0..50 {
            tokio::task::yield_now().await;
            if reg.status(&id).await == Some(AgentStatus::Running) {
                break;
            }
        }
        assert_eq!(reg.status(&id).await, Some(AgentStatus::Running));
        let _ = gate_tx.send(());
        reg.wait(&id).await.unwrap();
    }

    /// Spin the runtime until `id` reaches a terminal status (bounded), so the
    /// `select!` arm that observes the abort has a chance to run. Returns the
    /// final observed status.
    async fn settle_status(reg: &AgentRegistryImpl, id: &AgentId) -> Option<AgentStatus> {
        for _ in 0..50 {
            tokio::task::yield_now().await;
            if reg.status(id).await.is_some_and(AgentStatus::is_terminal) {
                break;
            }
        }
        reg.status(id).await
    }

    #[tokio::test]
    async fn abort_reports_true_for_live_agent_robust() {
        let (reg, id) = registry_with_agent("cancellable").await;
        let backend = InProcessBackend::new(reg.clone());

        // Work that never completes on its own — only the backend's abort arm
        // can end it. This keeps cancellation unambiguous (no self-complete race).
        backend.launch(id.clone(), move |_abort| async move {
            std::future::pending::<WorkOutcome>().await
        });

        // Wait until the backend has installed the abort handle (Running).
        for _ in 0..50 {
            tokio::task::yield_now().await;
            if reg.status(&id).await == Some(AgentStatus::Running) {
                break;
            }
        }
        assert!(reg.abort(&id).await);
    }

    #[tokio::test]
    async fn abort_marks_cancelled_robust() {
        let (reg, id) = registry_with_agent("cancellable").await;
        let backend = InProcessBackend::new(reg.clone());

        backend.launch(id.clone(), move |_abort| async move {
            std::future::pending::<WorkOutcome>().await
        });

        // Wait until the backend installed the abort handle before aborting.
        for _ in 0..50 {
            tokio::task::yield_now().await;
            if reg.status(&id).await == Some(AgentStatus::Running) {
                break;
            }
        }
        reg.abort(&id).await;
        assert_eq!(settle_status(&reg, &id).await, Some(AgentStatus::Cancelled));
    }
}
