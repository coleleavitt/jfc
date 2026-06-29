//! Concrete `AgentRegistry` implementation for jfc-engine.
//!
//! [`AgentRegistryImpl`] is the live registry that the engine holds behind an
//! `Arc`. It stores every spawned agent's [`AgentState`] + an optional abort
//! handle. Concrete execution backends register/update entries here; direct
//! [`jfc_agent::AgentRegistry::spawn`] calls are rejected until backend launch
//! wiring is available through this trait method.
//!
//! The inner map lives behind a `tokio::sync::RwLock`: every accessor is async
//! (matching the [`AgentRegistry`] trait), so guards are acquired with `.await`
//! and never block the executor thread. Cross-await coordination for
//! [`AgentRegistry::wait`] uses a per-entry [`Notify`], which is independent of
//! the lock so a waiter never holds the map locked while parked.
//!
//! Execution backends are wired in the sibling modules:
//! - `in_process.rs` — foreground/background tokio tasks
//! - `team.rs`       — long-lived teammate loops (swarm)

use std::collections::HashMap;
use std::sync::Arc;

use jfc_agent::{
    AgentId, AgentRegistry, AgentResult, AgentState, AgentStatus, RegistryError, SpawnConfig,
};
use tokio::sync::{Notify, RwLock};

/// Internal per-agent record.
struct AgentEntry {
    state: AgentState,
    /// Signalled when the agent reaches a terminal status.
    done: Arc<Notify>,
    /// Last result (populated on complete/fail/cancel).
    result: Option<AgentResult>,
    /// Abort sender — `None` for daemon-backed agents (abort via signal file).
    abort_tx: Option<tokio::sync::watch::Sender<bool>>,
}

/// The live registry. Hold behind `Arc<AgentRegistryImpl>`.
pub struct AgentRegistryImpl {
    inner: RwLock<HashMap<AgentId, AgentEntry>>,
}

impl AgentRegistryImpl {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(HashMap::new()),
        }
    }

    /// Insert a pre-built entry. Returns the entry's `Notify` so a caller can
    /// await its terminal transition. Backends call this after minting the
    /// agent but before the work future resolves.
    pub async fn register(&self, state: AgentState) -> Arc<Notify> {
        let done = Arc::new(Notify::new());
        self.inner.write().await.insert(
            state.id.clone(),
            AgentEntry {
                state,
                done: done.clone(),
                result: None,
                abort_tx: None,
            },
        );
        done
    }

    /// Attach an abort handle to an existing entry (called by in-process backends).
    pub async fn set_abort_tx(&self, id: &AgentId, tx: tokio::sync::watch::Sender<bool>) {
        if let Some(entry) = self.inner.write().await.get_mut(id) {
            entry.abort_tx = Some(tx);
        }
    }

    /// Update just the status of a running agent (e.g. Running → Idle).
    pub async fn update_status(&self, id: &AgentId, status: AgentStatus) {
        if let Some(entry) = self.inner.write().await.get_mut(id) {
            entry.state.status = status;
            if status.is_terminal() {
                entry.done.notify_waiters();
            }
        }
    }

    /// Mark an agent idle with an optional reason (teammate waiting for input).
    pub async fn set_idle(&self, id: &AgentId, reason: Option<String>) {
        if let Some(entry) = self.inner.write().await.get_mut(id) {
            entry.state.status = AgentStatus::Idle;
            entry.state.idle_reason = reason;
        }
    }

    /// Mark an agent terminally failed and wake any waiters.
    pub async fn fail(&self, id: &AgentId, error: impl Into<String>) {
        if let Some(entry) = self.inner.write().await.get_mut(id) {
            entry.state.fail(error);
            entry.done.notify_waiters();
        }
    }

    /// Update token/tool progress fields for an in-flight agent.
    pub async fn update_progress(
        &self,
        id: &AgentId,
        token_count: u64,
        tool_use_count: u32,
        last_tool: Option<String>,
    ) {
        if let Some(entry) = self.inner.write().await.get_mut(id) {
            entry.state.token_count = token_count;
            entry.state.tool_use_count = tool_use_count;
            if last_tool.is_some() {
                entry.state.last_tool = last_tool;
            }
        }
    }

    /// Read the terminal result for `id` if it has reached a terminal status.
    /// `Ok(None)` = still active; `Err` = unknown id.
    async fn terminal_result(&self, id: &AgentId) -> Result<Option<AgentResult>, RegistryError> {
        let guard = self.inner.read().await;
        let entry = guard
            .get(id)
            .ok_or_else(|| RegistryError::UnknownAgent(id.clone()))?;
        if !entry.state.status.is_terminal() {
            return Ok(None);
        }
        let result = entry.result.clone().unwrap_or_else(|| AgentResult {
            id: id.clone(),
            output: entry.state.error.clone().unwrap_or_default(),
            tokens_used: entry.state.token_count,
            elapsed_ms: 0,
            patch: None,
        });
        Ok(Some(result))
    }
}

impl Default for AgentRegistryImpl {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait::async_trait]
impl AgentRegistry for AgentRegistryImpl {
    async fn spawn(&self, config: SpawnConfig) -> Result<AgentId, RegistryError> {
        tracing::warn!(
            target: "jfc::agent::registry",
            role = config.role.label(),
            model = config.model.as_deref().unwrap_or("default"),
            detached = config.detached,
            "direct AgentRegistryImpl::spawn rejected: no execution backend is wired to this method"
        );

        Err(RegistryError::SpawnRejected(
            "AgentRegistryImpl::spawn is registry-only; launch through an in-process, team, daemon, or economy backend"
                .to_string(),
        ))
    }

    async fn status(&self, id: &AgentId) -> Option<AgentStatus> {
        self.inner.read().await.get(id).map(|e| e.state.status)
    }

    async fn state(&self, id: &AgentId) -> Option<AgentState> {
        self.inner.read().await.get(id).map(|e| e.state.clone())
    }

    async fn abort(&self, id: &AgentId) -> bool {
        let guard = self.inner.read().await;
        let Some(entry) = guard.get(id) else {
            return false;
        };
        if let Some(tx) = &entry.abort_tx {
            // SendError means the receiver is already gone (agent finished
            // before the abort arrived) — safe to ignore.
            let _send = tx.send(true);
            return true;
        }
        // Daemon agents abort via a cancel_requested flag (wired in the daemon
        // bridge); here we just report whether the agent is still alive.
        entry.state.status.is_active()
    }

    async fn complete(&self, id: &AgentId, result: AgentResult) {
        if let Some(entry) = self.inner.write().await.get_mut(id) {
            // Don't clobber a terminal status a racing abort already set.
            if entry.state.status.is_terminal() {
                return;
            }
            let summary = (!result.output.is_empty()).then(|| result.output.clone());
            entry.state.complete(summary);
            entry.state.token_count = result.tokens_used;
            entry.result = Some(result);
            entry.done.notify_waiters();
        }
    }

    async fn list(&self) -> Vec<AgentState> {
        self.inner
            .read()
            .await
            .values()
            .map(|e| e.state.clone())
            .collect()
    }

    async fn resolve_name(&self, name: &str) -> Option<AgentId> {
        // Match on display_name first, then description prefix.
        self.inner
            .read()
            .await
            .values()
            .find(|e| {
                e.state.id.display_name() == Some(name) || e.state.description.starts_with(name)
            })
            .map(|e| e.state.id.clone())
    }

    async fn wait(&self, id: &AgentId) -> Result<AgentResult, RegistryError> {
        let done = self
            .inner
            .read()
            .await
            .get(id)
            .map(|e| e.done.clone())
            .ok_or_else(|| RegistryError::UnknownAgent(id.clone()))?;

        loop {
            // Register interest BEFORE re-checking status so a completion that
            // fires between the check and the await is not missed.
            let notified = done.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();

            if let Some(result) = self.terminal_result(id).await? {
                return Ok(result);
            }
            notified.await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_agent::AgentRole;

    #[tokio::test]
    async fn register_and_status_normal() {
        let reg = AgentRegistryImpl::new();
        let id = AgentId::named("test-agent");
        let state = AgentState::new(id.clone(), AgentRole::Solo, "test");
        reg.register(state).await;
        assert_eq!(reg.status(&id).await, Some(AgentStatus::Pending));
    }

    #[tokio::test]
    async fn complete_updates_status_normal() {
        let reg = AgentRegistryImpl::new();
        let id = AgentId::named("worker");
        let state = AgentState::new(id.clone(), AgentRole::Solo, "work");
        reg.register(state).await;

        reg.complete(
            &id,
            AgentResult {
                id: id.clone(),
                output: "done".into(),
                tokens_used: 42,
                elapsed_ms: 100,
                patch: None,
            },
        )
        .await;
        assert_eq!(reg.status(&id).await, Some(AgentStatus::Completed));
    }

    #[tokio::test]
    async fn complete_does_not_clobber_cancelled_robust() {
        let reg = AgentRegistryImpl::new();
        let id = AgentId::named("racer");
        reg.register(AgentState::new(id.clone(), AgentRole::Solo, "race"))
            .await;

        // Abort wins first.
        reg.update_status(&id, AgentStatus::Cancelled).await;
        // A late completion must not overwrite the terminal Cancelled state.
        reg.complete(
            &id,
            AgentResult {
                id: id.clone(),
                output: "late".into(),
                tokens_used: 1,
                elapsed_ms: 1,
                patch: None,
            },
        )
        .await;
        assert_eq!(reg.status(&id).await, Some(AgentStatus::Cancelled));
    }

    #[tokio::test]
    async fn list_returns_all_agents_normal() {
        let reg = AgentRegistryImpl::new();
        for i in 0..3u32 {
            let id = AgentId::named(format!("agent-{i}"));
            reg.register(AgentState::new(id, AgentRole::Solo, "task"))
                .await;
        }
        assert_eq!(reg.list().await.len(), 3);
    }

    #[tokio::test]
    async fn resolve_name_finds_by_display_name_normal() {
        let reg = AgentRegistryImpl::new();
        let id = AgentId::named("researcher");
        reg.register(AgentState::new(
            id.clone(),
            AgentRole::Teammate {
                team_name: "alpha".into(),
            },
            "research task",
        ))
        .await;
        assert_eq!(reg.resolve_name("researcher").await, Some(id));
    }

    #[tokio::test]
    async fn direct_spawn_rejects_instead_of_pending_forever_robust() {
        let reg = AgentRegistryImpl::new();
        let err = reg
            .spawn(SpawnConfig::solo("unwired work", "/tmp"))
            .await
            .unwrap_err();

        assert!(matches!(err, RegistryError::SpawnRejected(_)));
        assert!(reg.list().await.is_empty());
    }

    #[tokio::test]
    async fn wait_resolves_after_complete_normal() {
        let reg = Arc::new(AgentRegistryImpl::new());
        let id = AgentId::named("async-worker");
        reg.register(AgentState::new(id.clone(), AgentRole::Solo, "async work"))
            .await;

        let reg2 = reg.clone();
        let id2 = id.clone();
        tokio::spawn(async move {
            tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
            reg2.complete(
                &id2,
                AgentResult {
                    id: id2.clone(),
                    output: "result".into(),
                    tokens_used: 5,
                    elapsed_ms: 10,
                    patch: None,
                },
            )
            .await;
        });

        let result = reg.wait(&id).await.unwrap();
        assert_eq!(result.output, "result");
        assert_eq!(result.tokens_used, 5);
    }
}
