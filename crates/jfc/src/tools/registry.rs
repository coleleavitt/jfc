/// Process-global state: static singletons, registration functions, and
/// their snapshot/accessor counterparts. All callers in this module
/// should go through these helpers rather than touching `OnceLock`
/// directly.
use std::sync::{Arc, OnceLock};

// ---------------------------------------------------------------------------
// Graph session cache
// ---------------------------------------------------------------------------

const GRAPH_BUILD_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Process-global cache of code-graph sessions keyed by canonicalized
/// workspace root. Without this, every `graph_query` / `symbol_edit`
/// tool call rebuilt the graph from scratch by re-running tree-sitter
/// across every Rust file in the workspace — slow on a real codebase
/// and wasteful when the LLM chains 5 graph queries in one turn.
/// `invalidate_graph_session_cache()` is called after `symbol_edit`,
/// `Edit`, and `Write` modify a file so the next query reflects the
/// change. Uses `std::sync::Mutex` (NOT tokio's) because the critical
/// section is purely synchronous map insert/get — fully-qualified path
/// avoids colliding with `tokio::sync::Mutex` elsewhere in the file.
pub(super) fn graph_session_cache() -> &'static std::sync::Mutex<
    std::collections::HashMap<std::path::PathBuf, Arc<jfc_graph::session::GraphSession>>,
> {
    static CACHE: OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<std::path::PathBuf, Arc<jfc_graph::session::GraphSession>>,
        >,
    > = OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub(super) fn graph_session_cache_key(cwd: &std::path::Path) -> std::path::PathBuf {
    cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf())
}

pub(super) fn build_graph_session_for_key(
    key: std::path::PathBuf,
) -> Arc<jfc_graph::session::GraphSession> {
    // Try loading from persistent bincode cache.
    let cache_path = jfc_graph::data_dir::resolve_data_dir(&key).join("index.bin");

    if let Some(session) = try_load_cached_graph(&key, &cache_path) {
        return Arc::new(session);
    }

    // Cache miss or stale — full build on a larger-than-tokio stack.
    let key_clone = key.clone();
    let session = std::thread::Builder::new()
        .name("graph-build".into())
        .stack_size(GRAPH_BUILD_STACK_BYTES)
        .spawn(move || jfc_graph::session::GraphSession::from_directory(&key_clone))
        .expect("failed to spawn graph-build thread")
        .join()
        .expect("graph-build thread panicked");

    // Save to cache (non-blocking save would be nice but snapshot is
    // cheap relative to the full parse we just did).
    let _ = jfc_graph::overlay::save_snapshot_bincode(&cache_path, &session.graph, &key);
    tracing::info!(
        target: "jfc::graph",
        path = %cache_path.display(),
        "persisted graph snapshot to disk"
    );

    Arc::new(session)
}

/// Try to load a cached graph from disk. Returns None if:
/// - Cache file doesn't exist
/// - Cache is stale (any source file is newer than the cache)
/// - Deserialization fails
fn try_load_cached_graph(
    workspace_root: &std::path::Path,
    cache_path: &std::path::Path,
) -> Option<jfc_graph::session::GraphSession> {
    let cache_meta = std::fs::metadata(cache_path).ok()?;
    let cache_mtime = cache_meta.modified().ok()?;

    // Walk source files and find the newest mtime.
    let newest_source = newest_source_mtime(workspace_root)?;

    if newest_source > cache_mtime {
        tracing::debug!(
            target: "jfc::graph",
            "graph cache stale — source files newer than index"
        );
        return None;
    }

    let start = std::time::Instant::now();
    let loaded = jfc_graph::overlay::load_snapshot_bincode(cache_path).ok()?;
    let session = jfc_graph::session::GraphSession::from_snapshot(loaded.graph, workspace_root);
    let elapsed_ms = start.elapsed().as_millis();
    tracing::info!(
        target: "jfc::graph",
        elapsed_ms,
        nodes = session.graph.node_count(),
        "loaded graph from persistent cache"
    );
    Some(session)
}

/// Walk workspace for source files and return the newest modification time.
fn newest_source_mtime(workspace_root: &std::path::Path) -> Option<std::time::SystemTime> {
    use std::time::SystemTime;

    let extensions: &[&str] = &["rs"];
    let mut newest = SystemTime::UNIX_EPOCH;

    let walker = ignore::WalkBuilder::new(workspace_root)
        .hidden(true)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .follow_links(false)
        .max_depth(Some(32))
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if !extensions.contains(&ext) {
            continue;
        }
        if let Ok(meta) = path.metadata()
            && let Ok(mtime) = meta.modified()
            && mtime > newest
        {
            newest = mtime;
        }
    }

    if newest == SystemTime::UNIX_EPOCH {
        None
    } else {
        Some(newest)
    }
}

/// Mutate the cached graph session by taking sole ownership of the cached
/// `Arc`, running `f`, then reinserting it. If another reader is still holding
/// the session, mutation fails cleanly instead of manufacturing an aliased
/// `&mut GraphSession`.
pub(super) fn with_graph_session_mut<R>(
    cwd: &std::path::Path,
    f: impl FnOnce(&mut jfc_graph::session::GraphSession) -> R,
) -> Result<R, String> {
    let key = graph_session_cache_key(cwd);

    // Recover from poisoning rather than failing the mutation — see
    // `get_or_build_graph_session` for the rationale.
    let lock_recover = |g: std::sync::LockResult<_>| match g {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };

    let session = {
        let mut cache = lock_recover(graph_session_cache().lock());
        if let Some(session) = cache.remove(&key) {
            session
        } else {
            drop(cache);

            let built = build_graph_session_for_key(key.clone());
            let mut cache = lock_recover(graph_session_cache().lock());
            cache.remove(&key).unwrap_or(built)
        }
    };

    let mut session = match Arc::try_unwrap(session) {
        Ok(session) => session,
        Err(shared) => {
            lock_recover(graph_session_cache().lock()).insert(key, shared);
            return Err("graph session is currently in use; retry coverage after the active graph query finishes".to_string());
        }
    };

    let output = f(&mut session);
    lock_recover(graph_session_cache().lock()).insert(key, Arc::new(session));
    Ok(output)
}

/// Get-or-build a cached `GraphSession` for `cwd`. Cheap on cache hit
/// (one HashMap lookup); first call per workspace pays the full
/// tree-sitter parse cost.
///
/// Graph building and analysis (tarjan_scc, page_rank, etc.) can recurse
/// deeply on large codebases. We spawn the build on a dedicated thread
/// with a 16MB stack to avoid overflowing tokio's 8MB worker threads without
/// reserving a large virtual-memory range for every build.
pub(crate) fn get_or_build_graph_session(
    cwd: &std::path::Path,
) -> Arc<jfc_graph::session::GraphSession> {
    let key = graph_session_cache_key(cwd);
    // Recover from a poisoned mutex by taking the inner data and
    // continuing — a panic during graph build would otherwise wedge
    // every subsequent UI tick that touches this cache (every
    // `graph_query` tool call and every status-bar redraw that reads
    // graph state). The cache content is plain `HashMap<PathBuf, Arc<…>>`;
    // a poisoned inner is at worst a stale entry, which we tolerate.
    let cache = match graph_session_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(existing) = cache.get(&key) {
        return Arc::clone(existing);
    }
    // Drop the lock before spawning the build thread — the build can take
    // seconds on large workspaces and we don't want to hold the mutex.
    drop(cache);

    let session = build_graph_session_for_key(key.clone());

    let mut cache = match graph_session_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    // Double-check: another thread may have built it while we were building.
    if let Some(existing) = cache.get(&key) {
        return Arc::clone(existing);
    }
    cache.insert(key, Arc::clone(&session));
    session
}

/// Drop the cached graph for `cwd` (or every cached graph when `cwd` is
/// `None`). Called after writes so the next graph query re-parses the
/// affected file. Cheap — actual rebuild only happens on the next query.
/// Also invalidates the auto-context cache in `intent.rs` which holds a
/// parallel copy of the same session data.
pub fn invalidate_graph_session_cache(cwd: Option<&std::path::Path>) {
    // Same poisoning rationale as `get_or_build_graph_session`.
    let mut cache = match graph_session_cache().lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    match cwd {
        Some(c) => {
            let key = c.canonicalize().unwrap_or_else(|_| c.to_path_buf());
            cache.remove(&key);
        }
        None => cache.clear(),
    }
    // Keep the auto-context cache (intent.rs) in sync — it holds its own
    // Arc<GraphSession> which would otherwise serve stale data.
    #[cfg(feature = "intent-gate")]
    crate::intent::clear_auto_context_cache();
}

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
pub(super) fn market_orchestrator()
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
pub(super) fn collusion_detector()
-> &'static std::sync::Mutex<jfc_economy::collusion::CollusionDetector> {
    static C: OnceLock<std::sync::Mutex<jfc_economy::collusion::CollusionDetector>> =
        OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(jfc_economy::collusion::CollusionDetector::default()))
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

fn active_provider_handle() -> &'static std::sync::RwLock<ActiveProvider> {
    static H: OnceLock<std::sync::RwLock<ActiveProvider>> = OnceLock::new();
    H.get_or_init(|| std::sync::RwLock::new(None))
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

/// Snapshot the active provider + model. None when main.rs hasn't
/// registered one yet (early-boot tool calls, tests).
pub(crate) fn snapshot_active_provider()
-> Option<(Arc<dyn jfc_provider::Provider>, jfc_provider::ModelId)> {
    active_provider_handle()
        .read()
        .ok()
        .and_then(|g| g.as_ref().map(|(p, m)| (Arc::clone(p), m.clone())))
}

// ---------------------------------------------------------------------------
// EngineEvent sender
// ---------------------------------------------------------------------------

/// Process-global handle to the EngineEvent channel. Set by main.rs
/// once at startup so bounty solver/validator subagents can emit
/// the same `TaskStarted` / `AgentChunk` / `TaskCompleted` events
/// the regular Task tool's swarm does — without that, the fan UI
/// and ctrl+X subagent panel show nothing while a cycle is running.
pub(super) fn active_event_sender_handle()
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

pub(crate) fn snapshot_event_sender() -> Option<tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>>
{
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

pub(crate) fn snapshot_mcp_registry() -> Option<crate::mcp::McpRegistry> {
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

// ---------------------------------------------------------------------------
// Graph history
// ---------------------------------------------------------------------------

/// Process-global graph-query history — task 27 from the
/// graph-context-engine plan. Stores the last 50 query / result
/// pairs so the user can inspect what the model has been asking
/// the graph and re-issue any of them via `/graph-history`. The
/// graph crate provides the underlying ring-buffer; we just keep
/// one handle per process and route inserts through it.
pub(super) fn graph_history() -> &'static std::sync::Mutex<jfc_graph::history::GraphHistory> {
    static HISTORY: OnceLock<std::sync::Mutex<jfc_graph::history::GraphHistory>> = OnceLock::new();
    HISTORY.get_or_init(|| std::sync::Mutex::new(jfc_graph::history::GraphHistory::new(50)))
}

/// Snapshot of recent graph-query records, most recent last. Used
/// by the `/graph-history` slash command and any UI panel that
/// wants to render the history without holding the lock.
pub fn graph_history_snapshot() -> Vec<jfc_graph::history::QueryRecord> {
    match graph_history().lock() {
        Ok(g) => g.all().iter().cloned().collect(),
        Err(_) => Vec::new(),
    }
}

pub(super) fn record_graph_query(query: &str, result: &jfc_graph::dsl::QueryResult) {
    if let Ok(mut g) = graph_history().lock() {
        g.record(query, result);
    }
}

// ---------------------------------------------------------------------------
// Auto-context queue
// ---------------------------------------------------------------------------

/// Queue of files modified by recent Edit/Write/symbol_edit calls,
/// awaiting auto-context injection at the next stream call. Mirrors
/// v131 Claude Code's behavior of surfacing affected callers to the
/// model after a function edit so it doesn't have to grep them
/// itself. Drained by `render_pending_auto_context()`; the renderer
/// runs `fn(name) | callers | depth 1` against the cached graph for
/// each modified file's functions and returns a single block to
/// splice into the next system prompt.
pub(super) fn auto_context_queue() -> &'static std::sync::Mutex<Vec<std::path::PathBuf>> {
    static QUEUE: OnceLock<std::sync::Mutex<Vec<std::path::PathBuf>>> = OnceLock::new();
    QUEUE.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Record that `path` was edited. Called from the Edit / Write /
/// symbol_edit tool handlers after a successful write. Cheap — just
/// appends to a Vec under a Mutex. The actual graph query runs
/// lazily inside `render_pending_auto_context()` at the next stream
/// boundary.
pub(crate) fn record_edited_file(path: &std::path::Path) {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if let Ok(mut q) = auto_context_queue().lock()
        && !q.contains(&canonical)
    {
        q.push(canonical);
    }
}

/// Drain the auto-context queue and render a single Graph Context
/// block describing callers of any function that lives in a
/// recently-edited file. Returns `None` when the queue is empty,
/// the graph isn't built, or no callers were found. Output is hard
/// capped at ~500 chars to honor the v131 token-budget convention
/// (auto-context is a hint, not a substitute for the model running
/// its own queries).
pub fn render_pending_auto_context(cwd: &std::path::Path) -> Option<String> {
    const MAX_CHARS: usize = 500;
    let edited: Vec<std::path::PathBuf> = match auto_context_queue().lock() {
        Ok(mut q) => std::mem::take(&mut *q),
        Err(_) => return None,
    };
    if edited.is_empty() {
        return None;
    }
    let session = get_or_build_graph_session(cwd);

    let mut out = String::new();
    out.push_str(
        "\n\n## Graph Context\nCallers of recently-edited functions \
        (auto-generated; ignore if unrelated to your next move):\n",
    );
    let mut any_callers = false;
    'outer: for file in &edited {
        // Function nodes whose `file_path` matches the edited file.
        let fns: Vec<_> = session
            .graph
            .nodes_by_kind(jfc_graph::nodes::NodeKind::Function)
            .into_iter()
            .filter(|n| n.file_path == *file)
            .collect();
        for f in fns {
            let q = format!("fn(\"{}\") | callers | depth 1", f.name);
            // Per-function budget keeps any one fn from filling the block.
            let budget = MAX_CHARS / 4;
            if let Ok(result) = session.query(&q, budget)
                && result.nodes_total > 0
            {
                any_callers = true;
                out.push_str(&format!(
                    "\n- `{}` ({}): {} caller(s)\n  {}\n",
                    f.name,
                    file.display(),
                    result.nodes_total,
                    result.text.lines().take(4).collect::<Vec<_>>().join("  ")
                ));
                if out.len() >= MAX_CHARS {
                    out.truncate(MAX_CHARS);
                    out.push('…');
                    break 'outer;
                }
            }
        }
    }
    if !any_callers {
        return None;
    }
    Some(out)
}
