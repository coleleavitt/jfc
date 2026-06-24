/// Process-global state: static singletons, registration functions, and
/// their snapshot/accessor counterparts. All callers in this module
/// should go through these helpers rather than touching `OnceLock`
/// directly.
use std::sync::{Arc, OnceLock};

// ---------------------------------------------------------------------------
// Market orchestrator + collusion detector
// ---------------------------------------------------------------------------

/// Process-global market orchestrator — task 14/15 from the
/// agent-economy plan. Holds bounty state, ledger, trust scores,
/// charter, collusion detector. One per process so consecutive
/// `post_bounty` / `market_status` calls see consistent state and
/// trust accumulates across bounties. Initialized lazily with the
/// charter's defaults; user-tunable via `JFC_MARKET_BUDGET` env var
/// (defaults to 100_000 tokens — the v131 auto-compact threshold).
pub fn market_orchestrator()
-> &'static tokio::sync::Mutex<jfc_economy::orchestrator::MarketOrchestrator> {
    // tokio::sync::Mutex (not std::sync::Mutex) so guards are Send
    // across .await — required because run_bounty_cycle holds the
    // lock across LLM calls.
    static M: OnceLock<tokio::sync::Mutex<jfc_economy::orchestrator::MarketOrchestrator>> =
        OnceLock::new();
    M.get_or_init(|| {
        let charter = jfc_economy::charter::Charter::default();
        let budget = std::env::var("JFC_MARKET_BUDGET")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(100_000);
        tokio::sync::Mutex::new(jfc_economy::orchestrator::MarketOrchestrator::with_budget(
            charter, budget,
        ))
    })
}

/// Companion collusion detector for the orchestrator. Kept separate
/// because `MarketReport::generate` takes them as distinct args.
pub fn collusion_detector() -> &'static std::sync::Mutex<jfc_economy::collusion::CollusionDetector>
{
    static C: OnceLock<std::sync::Mutex<jfc_economy::collusion::CollusionDetector>> =
        OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(jfc_economy::collusion::CollusionDetector::default()))
}

// ---------------------------------------------------------------------------
// Unified agent registry
// ---------------------------------------------------------------------------

/// Process-global unified [`AgentRegistry`](jfc_agent::AgentRegistry).
///
/// Single source of truth for every spawned agent's lifecycle — solo
/// subagents, teammates, council seats, and economy solvers/validators all
/// register here, so the UI roster, `wait`, and `abort` see one consistent
/// view instead of the previous three parallel trackers (`BackgroundTask`,
/// `BackgroundAgentInfo`, `InProcessTeammateState`).
///
/// Held as a singleton (mirroring `market_orchestrator` / `active_event_sender`)
/// so spawn paths don't have to thread a registry parameter through
/// `execute_tool`'s 20 callsites or bolt it onto the `EngineState` god-object.
pub fn agent_registry() -> &'static Arc<crate::agents::AgentRegistryImpl> {
    static R: OnceLock<Arc<crate::agents::AgentRegistryImpl>> = OnceLock::new();
    R.get_or_init(|| Arc::new(crate::agents::AgentRegistryImpl::new()))
}

// ---------------------------------------------------------------------------
// Active provider handle
// ---------------------------------------------------------------------------

/// Process-global handle to the active Provider + ModelId. Set
/// once at startup by `main.rs` after it constructs the provider
/// chain; consumed by the agent-economy `auto_dispatch` path which
/// needs to spin up sub-LLM calls without changing every signature
/// of `execute_tool`. RwLock so future model swaps can update it
/// without restarting the process.
type ActiveProvider = Option<(Arc<dyn jfc_provider::Provider>, jfc_provider::ModelId)>;
type ProviderRegistry = Vec<Arc<dyn jfc_provider::Provider>>;

fn active_provider_handle() -> &'static std::sync::RwLock<ActiveProvider> {
    static H: OnceLock<std::sync::RwLock<ActiveProvider>> = OnceLock::new();
    H.get_or_init(|| std::sync::RwLock::new(None))
}

fn provider_registry_handle() -> &'static std::sync::RwLock<ProviderRegistry> {
    static H: OnceLock<std::sync::RwLock<ProviderRegistry>> = OnceLock::new();
    H.get_or_init(|| std::sync::RwLock::new(Vec::new()))
}

/// Called by main.rs after the provider chain is built so
/// auto-dispatch market cycles can issue real LLM calls. Calling
/// this multiple times overwrites the previous handle, which is
/// the right behavior for a model-switch flow.
pub fn register_active_provider(
    provider: Arc<dyn jfc_provider::Provider>,
    model: jfc_provider::ModelId,
) {
    if let Ok(mut g) = active_provider_handle().write() {
        *g = Some((provider, model));
    }
}

pub fn register_provider_registry(providers: Vec<Arc<dyn jfc_provider::Provider>>) {
    if let Ok(mut g) = provider_registry_handle().write() {
        *g = providers;
    }
}

/// Snapshot the active provider + model. None when main.rs hasn't
/// registered one yet (early-boot tool calls, tests).
pub fn snapshot_active_provider() -> Option<(Arc<dyn jfc_provider::Provider>, jfc_provider::ModelId)>
{
    active_provider_handle()
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|(p, m)| (Arc::clone(p), m.clone())))
}

pub fn snapshot_provider_registry() -> Vec<Arc<dyn jfc_provider::Provider>> {
    provider_registry_handle()
        .read()
        .map(|g| g.iter().cloned().collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// EngineEvent sender
// ---------------------------------------------------------------------------

/// Process-global handle to the EngineEvent channel. Set by main.rs
/// once at startup so bounty solver/validator subagents can emit
/// the same `TaskStarted` / `AgentChunk` / `TaskCompleted` events
/// the regular Task tool's swarm does — without that, the fan UI
/// and ctrl+X subagent panel show nothing while a cycle is running.
pub fn active_event_sender_handle()
-> &'static std::sync::RwLock<Option<tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>>> {
    static H: OnceLock<
        std::sync::RwLock<Option<tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>>>,
    > = OnceLock::new();
    H.get_or_init(|| std::sync::RwLock::new(None))
}

pub fn register_event_sender(tx: tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>) {
    if let Ok(mut g) = active_event_sender_handle().write() {
        *g = Some(tx);
    }
}

pub fn snapshot_event_sender() -> Option<tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>> {
    active_event_sender_handle()
        .read()
        .ok()
        .and_then(|g| g.clone())
}

// ---------------------------------------------------------------------------
// MCP registry
// ---------------------------------------------------------------------------

/// Process-global handle to the active MCP registry. Set once at
/// startup via `register_mcp_registry`, read by the dispatch arm in
/// `execute_tool` so MCP tool calls can route to the right server
/// without threading a registry parameter through every callsite.
///
/// Mirrors `active_event_sender_handle` exactly — the dispatcher is
/// already a process-global singleton via tokio tasks, and bolting a
/// registry parameter on would touch dozens of callsites for no
/// architectural win.
fn active_mcp_registry_handle() -> &'static std::sync::RwLock<Option<crate::mcp::McpRegistry>> {
    static H: OnceLock<std::sync::RwLock<Option<crate::mcp::McpRegistry>>> = OnceLock::new();
    H.get_or_init(|| std::sync::RwLock::new(None))
}

pub fn register_mcp_registry(registry: crate::mcp::McpRegistry) {
    if let Ok(mut g) = active_mcp_registry_handle().write() {
        *g = Some(registry);
    }
}

pub fn snapshot_mcp_registry() -> Option<crate::mcp::McpRegistry> {
    active_mcp_registry_handle()
        .read()
        .ok()
        .and_then(|g| g.clone())
}

// ---------------------------------------------------------------------------
// Undo history
// ---------------------------------------------------------------------------

/// Process-global FIFO of `/undo` entries. Tool dispatchers call
/// `push_undo_entry` *before* mutating the filesystem; the slash
/// command handler pops from this and applies the reversal. Stored
/// here (not on App) so per-tool dispatchers don't need a handle to
/// App threaded through the tool layer. Capped at 100 entries.
fn undo_history_handle()
-> &'static std::sync::RwLock<std::collections::VecDeque<crate::types::ToolUndoEntry>> {
    static H: OnceLock<std::sync::RwLock<std::collections::VecDeque<crate::types::ToolUndoEntry>>> =
        OnceLock::new();
    H.get_or_init(|| std::sync::RwLock::new(std::collections::VecDeque::new()))
}

/// Push an undo entry onto the per-session stack. Called from
/// `execute_edit` / `execute_write` / `execute_apply_patch` / etc.
/// before they mutate the filesystem.
pub fn push_undo_entry(file_path: &str, previous_content: Option<String>, op_label: &str) {
    let entry = crate::types::ToolUndoEntry {
        file_path: file_path.to_owned(),
        previous_content,
        op_label: op_label.to_owned(),
    };
    if let Ok(mut h) = undo_history_handle().write() {
        if h.len() >= 100 {
            h.pop_front();
        }
        h.push_back(entry);
    }
}

/// Drain the most recent undo entry.
pub fn pop_undo_entry() -> Option<crate::types::ToolUndoEntry> {
    undo_history_handle().write().ok()?.pop_back()
}

/// Push an entry back (used when /undo failed to apply).
pub fn restore_undo_entry(entry: crate::types::ToolUndoEntry) {
    if let Ok(mut h) = undo_history_handle().write() {
        h.push_back(entry);
    }
}
