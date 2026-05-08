#![allow(dead_code)]

mod bash;
mod daemon;
mod defs;
mod economy;
mod filesystem;
mod lsp;
mod memory;
mod notebook;
mod notifications;
mod search;
mod subagent;
mod swarm;
mod tasks;
mod worktree;
#[cfg(test)]
mod tests;

// Re-exports from submodules
pub(crate) use defs::all_tool_defs;
pub(crate) use economy::{
    apply_winning_solution, market_report_string,
    EconomyAgentInvoker, EconomySwarmProvider,
};
pub(crate) use subagent::execute_task;
pub(crate) use tasks::execute_skill;


// Internal imports from submodules (used by execute_tool dispatcher)
use economy::strip_html_tags;
use bash::execute_bash;
use daemon::{execute_cron_create, execute_cron_delete, execute_cron_list, execute_monitor, execute_schedule_wakeup};
use filesystem::{execute_edit, execute_read, execute_write};
use lsp::execute_lsp;
use memory::{execute_memory_create, execute_memory_delete};
use notebook::{execute_notebook_edit, execute_notebook_read};
use notifications::{execute_push_notification, execute_remote_trigger};
use search::{execute_glob, execute_grep};
use swarm::{execute_send_message, execute_team_create, execute_team_delete, execute_team_member_mode};
use tasks::{execute_task_create, execute_task_done, execute_task_list, execute_task_update};
use worktree::{execute_enter_plan_mode, execute_enter_worktree, execute_exit_worktree};

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::sync::OnceLock;

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
fn graph_session_cache() -> &'static std::sync::Mutex<
    std::collections::HashMap<std::path::PathBuf, Arc<jfc_graph::session::GraphSession>>,
> {
    static CACHE: OnceLock<
        std::sync::Mutex<
            std::collections::HashMap<std::path::PathBuf, Arc<jfc_graph::session::GraphSession>>,
        >,
    > = OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Get-or-build a cached `GraphSession` for `cwd`. Cheap on cache hit
/// (one HashMap lookup); first call per workspace pays the full
/// tree-sitter parse cost.
///
/// Graph building and analysis (tarjan_scc, page_rank, etc.) can recurse
/// deeply on large codebases. We spawn the build on a dedicated thread
/// with a 64MB stack to avoid overflowing tokio's 8MB worker threads.
fn get_or_build_graph_session(cwd: &std::path::Path) -> Arc<jfc_graph::session::GraphSession> {
    let key = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let cache = graph_session_cache()
        .lock()
        .expect("graph cache mutex poisoned");
    if let Some(existing) = cache.get(&key) {
        return Arc::clone(existing);
    }
    // Drop the lock before spawning the build thread — the build can take
    // seconds on large workspaces and we don't want to hold the mutex.
    drop(cache);

    let key_clone = key.clone();
    let session = std::thread::Builder::new()
        .name("graph-build".into())
        .stack_size(64 * 1024 * 1024) // 64MB — handles 10K+ node graphs
        .spawn(move || Arc::new(jfc_graph::session::GraphSession::from_directory(&key_clone)))
        .expect("failed to spawn graph-build thread")
        .join()
        .expect("graph-build thread panicked");

    let mut cache = graph_session_cache()
        .lock()
        .expect("graph cache mutex poisoned");
    // Double-check: another thread may have built it while we were building.
    if let Some(existing) = cache.get(&key) {
        return Arc::clone(existing);
    }
    cache.insert(key, Arc::clone(&session));
    session
}

/// Process-global market orchestrator — task 14/15 from the
/// agent-economy plan. Holds bounty state, ledger, trust scores,
/// charter, collusion detector. One per process so consecutive
/// `post_bounty` / `market_status` calls see consistent state and
/// trust accumulates across bounties. Initialized lazily with the
/// charter's defaults; user-tunable via `JFC_MARKET_BUDGET` env var
/// (defaults to 100_000 tokens — the v131 auto-compact threshold).
fn market_orchestrator()
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
fn collusion_detector() -> &'static std::sync::Mutex<jfc_economy::collusion::CollusionDetector> {
    static C: OnceLock<std::sync::Mutex<jfc_economy::collusion::CollusionDetector>> =
        OnceLock::new();
    C.get_or_init(|| std::sync::Mutex::new(jfc_economy::collusion::CollusionDetector::default()))
}

/// Process-global handle to the active Provider + ModelId. Set
/// once at startup by `main.rs` after it constructs the provider
/// chain; consumed by the agent-economy `auto_dispatch` path which
/// needs to spin up sub-LLM calls without changing every signature
/// of `execute_tool`. RwLock so future model swaps can update it
/// without restarting the process.
fn active_provider_handle() -> &'static std::sync::RwLock<
    Option<(
        std::sync::Arc<dyn crate::provider::Provider>,
        crate::provider::ModelId,
    )>,
> {
    static H: OnceLock<
        std::sync::RwLock<
            Option<(
                std::sync::Arc<dyn crate::provider::Provider>,
                crate::provider::ModelId,
            )>,
        >,
    > = OnceLock::new();
    H.get_or_init(|| std::sync::RwLock::new(None))
}

/// Called by main.rs after the provider chain is built so
/// auto-dispatch market cycles can issue real LLM calls. Calling
/// this multiple times overwrites the previous handle, which is
/// the right behavior for a model-switch flow.
pub fn register_active_provider(
    provider: std::sync::Arc<dyn crate::provider::Provider>,
    model: crate::provider::ModelId,
) {
    if let Ok(mut g) = active_provider_handle().write() {
        *g = Some((provider, model));
    }
}

/// Snapshot the active provider + model. None when main.rs hasn't
/// registered one yet (early-boot tool calls, tests).
pub(crate) fn snapshot_active_provider() -> Option<(
    std::sync::Arc<dyn crate::provider::Provider>,
    crate::provider::ModelId,
)> {
    active_provider_handle().read().ok().and_then(|g| {
        g.as_ref()
            .map(|(p, m)| (std::sync::Arc::clone(p), m.clone()))
    })
}

/// Process-global handle to the AppEvent channel. Set by main.rs
/// once at startup so bounty solver/validator subagents can emit
/// the same `TaskStarted` / `AgentChunk` / `TaskCompleted` events
/// the regular Task tool's swarm does — without that, the fan UI
/// and ctrl+X subagent panel show nothing while a cycle is running.
fn active_event_sender_handle()
-> &'static std::sync::RwLock<Option<tokio::sync::mpsc::Sender<crate::app::AppEvent>>> {
    static H: OnceLock<std::sync::RwLock<Option<tokio::sync::mpsc::Sender<crate::app::AppEvent>>>> =
        OnceLock::new();
    H.get_or_init(|| std::sync::RwLock::new(None))
}

pub fn register_event_sender(tx: tokio::sync::mpsc::Sender<crate::app::AppEvent>) {
    if let Ok(mut g) = active_event_sender_handle().write() {
        *g = Some(tx);
    }
}

pub(crate) fn snapshot_event_sender() -> Option<tokio::sync::mpsc::Sender<crate::app::AppEvent>> {
    active_event_sender_handle()
        .read()
        .ok()
        .and_then(|g| g.clone())
}

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

/// Process-global queue of attachments staged for the next outgoing
/// request. The Read tool pushes to this queue when it ingests a
/// `.pdf` (or, in future, an image) so the file lands in the
/// upcoming `tool_result` message as a `document` / `image` content
/// block instead of being squashed into a base64 text blob the
/// model can't usefully read.
///
/// Drained by `stream::build_provider_messages_with_tool_results`
/// just before serialization.
fn pending_tool_attachments_handle()
-> &'static std::sync::Mutex<Vec<crate::attachments::Attachment>> {
    static H: OnceLock<std::sync::Mutex<Vec<crate::attachments::Attachment>>> =
        OnceLock::new();
    H.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

/// Stash an attachment for the next outgoing tool_result message.
/// Called by tools (currently the Read tool when handed a `.pdf`)
/// that need to surface a binary blob via Anthropic's `document` or
/// `image` content block.
pub(crate) fn push_pending_tool_attachment(att: crate::attachments::Attachment) {
    if let Ok(mut g) = pending_tool_attachments_handle().lock() {
        tracing::debug!(
            target: "jfc::tools::attach",
            kind = att.kind.mime_type(),
            bytes = att.bytes.len(),
            queued = g.len() + 1,
            "queued attachment for next request"
        );
        g.push(att);
    }
}

/// Drain every staged attachment. Called from
/// `stream::build_provider_messages_with_tool_results` so the next
/// request includes the attachments and the queue resets to empty.
pub fn take_pending_tool_attachments() -> Vec<crate::attachments::Attachment> {
    pending_tool_attachments_handle()
        .lock()
        .map(|mut g| std::mem::take(&mut *g))
        .unwrap_or_default()
}

/// Drop the cached graph for `cwd` (or every cached graph when `cwd` is
/// `None`). Called after writes so the next graph query re-parses the
/// affected file. Cheap — actual rebuild only happens on the next query.
pub fn invalidate_graph_session_cache(cwd: Option<&std::path::Path>) {
    let mut cache = graph_session_cache()
        .lock()
        .expect("graph cache mutex poisoned");
    match cwd {
        Some(c) => {
            let key = c.canonicalize().unwrap_or_else(|_| c.to_path_buf());
            cache.remove(&key);
        }
        None => cache.clear(),
    }
}

/// Process-global graph-query history — task 27 from the
/// graph-context-engine plan. Stores the last 50 query / result
/// pairs so the user can inspect what the model has been asking
/// the graph and re-issue any of them via `/graph-history`. The
/// graph crate provides the underlying ring-buffer; we just keep
/// one handle per process and route inserts through it.
fn graph_history() -> &'static std::sync::Mutex<jfc_graph::history::GraphHistory> {
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

fn record_graph_query(query: &str, result: &jfc_graph::dsl::QueryResult) {
    if let Ok(mut g) = graph_history().lock() {
        g.record(query, result);
    }
}

/// Queue of files modified by recent Edit/Write/symbol_edit calls,
/// awaiting auto-context injection at the next stream call. Mirrors
/// v131 Claude Code's behavior of surfacing affected callers to the
/// model after a function edit so it doesn't have to grep them
/// itself. Drained by `render_pending_auto_context()`; the renderer
/// runs `fn(name) | callers | depth 1` against the cached graph for
/// each modified file's functions and returns a single block to
/// splice into the next system prompt.
fn auto_context_queue() -> &'static std::sync::Mutex<Vec<std::path::PathBuf>> {
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
    if let Ok(mut q) = auto_context_queue().lock() {
        if !q.contains(&canonical) {
            q.push(canonical);
        }
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
                    out.push_str("…");
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

use tokio::process::Command;
use tokio::sync::Mutex;

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
}


use crate::context::ReadDedupCache;
use crate::provider::ToolDef;
use crate::tasks::TaskStore;
use crate::types::{ToolInput, ToolKind};

pub async fn all_tool_defs_with_mcp() -> Vec<ToolDef> {
    let mut tools = all_tool_defs();
    if let Some(registry) = snapshot_mcp_registry() {
        tools.extend(registry.all_advertised_tool_defs().await);
    }
    tools
}

#[derive(Debug, Clone)]
pub struct ExecutionResult {
    pub output: String,
    pub outcome: ToolOutcome,
    pub diagnostics: Vec<ToolDiagnostic>,
    pub provenance: Option<ToolProvenance>,
    /// When set, the renderer prefers this structured diff over
    /// `output`/`Text`. Used by Edit (and Write-as-overwrite) to surface
    /// a colorized diff in the transcript instead of a flat
    /// "file updated successfully" string.
    pub diff: Option<crate::types::DiffView>,
}

impl ExecutionResult {
    pub fn success(output: impl Into<String>) -> Self {
        Self {
            output: output.into(),
            outcome: ToolOutcome::Success,
            diagnostics: Vec::new(),
            provenance: None,
            diff: None,
        }
    }

    pub fn failure(output: impl Into<String>) -> Self {
        let output = output.into();
        Self {
            diagnostics: vec![ToolDiagnostic::error(output.clone())],
            output,
            outcome: ToolOutcome::Failed,
            provenance: None,
            diff: None,
        }
    }

    pub fn with_provenance(mut self, provenance: ToolProvenance) -> Self {
        self.provenance = Some(provenance);
        self
    }

    pub fn with_diff(mut self, diff: crate::types::DiffView) -> Self {
        self.diff = Some(diff);
        self
    }

    pub fn is_error(&self) -> bool {
        matches!(self.outcome, ToolOutcome::Failed)
    }
}

fn configure_tool_command(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SUDO_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false");

    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

/// Process-global FIFO of `/undo` entries. Tool dispatchers call
/// `push_undo_entry` *before* mutating the filesystem; the slash
/// command handler pops from this and applies the reversal. Stored
/// here (not on App) so per-tool dispatchers don't need a handle to
/// App threaded through the tool layer. Capped at 100 entries.
fn undo_history_handle()
-> &'static std::sync::RwLock<std::collections::VecDeque<crate::types::ToolUndoEntry>> {
    use std::sync::OnceLock;
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

fn terminal_safe_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut previous_was_esc = false;
                    for c in chars.by_ref() {
                        if c == '\u{7}' || (previous_was_esc && c == '\\') {
                            break;
                        }
                        previous_was_esc = c == '\u{1b}';
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\t' | '\n' | '\r' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }

    out
}

fn non_interactive_shell_command(command: &str) -> String {
    let trimmed = command.trim_start();
    let leading_len = command.len() - trimmed.len();

    if trimmed == "sudo" {
        return format!("{}sudo -n", &command[..leading_len]);
    }

    let Some(rest) = trimmed.strip_prefix("sudo ") else {
        return command.to_string();
    };

    if rest.starts_with("-n ") || rest == "-n" || rest.starts_with("--non-interactive ") {
        command.to_string()
    } else {
        format!("{}sudo -n {}", &command[..leading_len], rest)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolOutcome {
    Success,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolDiagnostic {
    pub level: DiagnosticLevel,
    pub message: String,
    pub help: Option<String>,
}

impl ToolDiagnostic {
    fn error(message: impl Into<String>) -> Self {
        Self {
            level: DiagnosticLevel::Error,
            message: message.into(),
            help: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DiagnosticLevel {
    Error,
    Warning,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolProvenance {
    pub cwd: PathBuf,
    pub source: ToolSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)]
pub enum ToolSource {
    ModelRequested,
    LocalExecutor,
}

#[cfg(feature = "permission-automation")]
fn tool_permission_path(input: &ToolInput) -> Option<&str> {
    match input {
        ToolInput::Edit { file_path, .. }
        | ToolInput::Write { file_path, .. }
        | ToolInput::Read { file_path, .. } => Some(file_path.as_str()),
        ToolInput::Bash {
            workdir: Some(workdir),
            ..
        }
        | ToolInput::Glob {
            path: Some(workdir),
            ..
        }
        | ToolInput::Grep {
            path: Some(workdir),
            ..
        }
        | ToolInput::Search {
            path: Some(workdir),
            ..
        } => Some(workdir.as_str()),
        ToolInput::MemoryDelete { path } => Some(path.as_str()),
        _ => None,
    }
}

/// REQ-TOOLS-002: Tool executors — bash/read/write/edit/glob/grep/task via tokio + fs.
#[tracing::instrument(target = "jfc::tools", skip(input, cwd, dedup, task_store), fields(kind = ?kind))]
pub async fn execute_tool(
    kind: ToolKind,
    input: ToolInput,
    cwd: std::path::PathBuf,
    dedup: Option<Arc<Mutex<ReadDedupCache>>>,
    task_store: Option<Arc<TaskStore>>,
    active_team_name: Option<&str>,
) -> ExecutionResult {
    #[cfg(feature = "hooks")]
    {
        // Hook integration point: BeforeToolDispatch
        // When fully wired, this will:
        // 1. Build HookContext from tool name + input
        // 2. Fire BeforeToolDispatch hooks
        // 3. If Abort → return error
        // 4. If Skip → return empty result
        // 5. If Replace → use replacement input
        tracing::trace!(target: "jfc::hooks", "hook integration point: BeforeToolDispatch");
    }

    #[cfg(feature = "permission-automation")]
    {
        use crate::permissions::{PermissionAction, check_tool_permission};

        let config = crate::config::feature_config::FeatureConfig::load(&cwd);
        let rules = crate::permissions::RuleSet::from_config(&config);
        let decision = check_tool_permission(&rules, kind.api_name(), tool_permission_path(&input));

        if matches!(decision.action, PermissionAction::Deny) {
            let reason = decision
                .reason
                .as_deref()
                .unwrap_or("permission rule denied tool invocation");
            return ExecutionResult::failure(format!(
                "Permission denied for {}: {reason}",
                kind.api_name()
            ));
        }
    }

    match (kind, input) {
        (
            ToolKind::Bash,
            ToolInput::Bash {
                command, timeout, ..
            },
        ) => execute_bash(&command, timeout, &cwd).await,
        (
            ToolKind::Read,
            ToolInput::Read {
                file_path,
                offset,
                limit,
            },
        ) => execute_read(&file_path, offset, limit, dedup.as_ref()).await,
        (ToolKind::Write, ToolInput::Write { file_path, content }) => {
            let result = execute_write(&file_path, &content).await;
            if !result.is_error() {
                if let Some(cache) = &dedup {
                    cache.lock().await.invalidate(Path::new(&file_path));
                }
                // Drop the cached graph for this workspace so the next
                // graph_query reflects the new file content.
                invalidate_graph_session_cache(Some(&cwd));
                record_edited_file(Path::new(&file_path));
            }
            result
        }
        (
            ToolKind::Edit,
            ToolInput::Edit {
                file_path,
                old_string,
                new_string,
                replacement,
            },
        ) => {
            let result = execute_edit(&file_path, &old_string, &new_string, replacement).await;
            if !result.is_error() {
                if let Some(cache) = &dedup {
                    cache.lock().await.invalidate(Path::new(&file_path));
                }
                invalidate_graph_session_cache(Some(&cwd));
                record_edited_file(Path::new(&file_path));
            }
            result
        }
        (ToolKind::Glob, ToolInput::Glob { pattern, path }) => {
            execute_glob(&pattern, path.as_deref(), &cwd).await
        }
        (
            ToolKind::Grep,
            ToolInput::Grep {
                pattern,
                path,
                glob,
                output_mode,
            },
        ) => {
            execute_grep(
                &pattern,
                path.as_deref(),
                glob.as_deref(),
                output_mode.as_deref(),
                &cwd,
            )
            .await
        }
        (
            ToolKind::TaskCreate,
            ToolInput::TaskCreate {
                subject,
                description,
                active_form,
                blocked_by,
            },
        ) => execute_task_create(task_store, subject, description, active_form, blocked_by),
        (
            ToolKind::TaskUpdate,
            ToolInput::TaskUpdate {
                task_id,
                status,
                subject,
                description,
                owner,
            },
        ) => execute_task_update(task_store, &task_id, status, subject, description, owner),
        (
            ToolKind::TaskList,
            ToolInput::TaskList {
                status_filter,
                owner_filter,
            },
        ) => execute_task_list(
            task_store,
            status_filter.as_deref(),
            owner_filter.as_deref(),
        ),
        (ToolKind::TaskDone, ToolInput::TaskDone { task_id }) => {
            execute_task_done(task_store, &task_id)
        }
        (ToolKind::Task, ToolInput::Task(_)) => {
            ExecutionResult::failure("Task tool must be dispatched via the streaming executor")
        }
        (ToolKind::Skill, ToolInput::Skill { name, args }) => {
            execute_skill(&name, args.as_deref()).await
        }
        (
            ToolKind::MemoryCreate,
            ToolInput::MemoryCreate {
                level,
                memory_type,
                scope,
                body,
            },
        ) => execute_memory_create(&level, &memory_type, &scope, &body, &cwd),
        (ToolKind::MemoryDelete, ToolInput::MemoryDelete { path }) => execute_memory_delete(&path),
        (
            ToolKind::TeamCreate,
            ToolInput::TeamCreate {
                team_name,
                description,
            },
        ) => execute_team_create(&team_name, description.as_deref(), &cwd).await,
        (ToolKind::TeamDelete, ToolInput::TeamDelete) => {
            execute_team_delete(active_team_name).await
        }
        (
            ToolKind::SendMessage,
            ToolInput::SendMessage {
                to,
                message,
                summary,
            },
        ) => execute_send_message(&to, &message, summary.as_deref(), active_team_name).await,
        (ToolKind::TeamMemberMode, ToolInput::TeamMemberMode { member_name, mode }) => {
            execute_team_member_mode(&member_name, &mode, active_team_name).await
        }
        (
            ToolKind::GraphQuery,
            ToolInput::GraphQuery {
                query,
                max_tokens,
                include_handles,
            },
        ) => {
            let budget = max_tokens.unwrap_or(4000);
            let want_handles = include_handles.unwrap_or(true);
            let session = get_or_build_graph_session(&cwd);
            // Run twice: once raw (so we can record the structured
            // QueryResult to history *and* extract chain-able handles)
            // and once formatted with the budget. The raw call is
            // cheap — same parse, just skips the formatting pass —
            // and the alternative (changing format_query_result to
            // also expose the QueryResult) would touch the jfc-graph
            // public API.
            let raw_for_predicates = session.query_raw(&query).ok();
            if let Some(ref raw) = raw_for_predicates {
                record_graph_query(&query, raw);
            }
            match session.query(&query, budget) {
                Ok(output) => {
                    let mut text = output.text.clone();
                    // Magic's path-dependent analysis: when the
                    // query asked for `preconditions`, append the
                    // enclosing if/match/while predicate at every
                    // outgoing call site of each caller. The model
                    // sees "to call X you must have passed (a > 0)"
                    // without having to grep for callers manually.
                    if query.contains("preconditions")
                        && let Some(ref raw) = raw_for_predicates
                    {
                        let mut preds_block = String::new();
                        for node_id in raw.nodes.iter().take(10) {
                            let preds = jfc_graph::predicates::outgoing_call_predicates(
                                &session.graph,
                                node_id,
                            );
                            if preds.is_empty() {
                                continue;
                            }
                            if let Some(node) = session.graph.get_node(node_id) {
                                preds_block.push_str(&format!(
                                    "\n  • {} ({}):\n",
                                    node.name,
                                    node.file_path.display()
                                ));
                            }
                            for (target, ps) in preds.iter().take(3) {
                                let chain = ps
                                    .iter()
                                    .map(|p| p.text.as_str())
                                    .collect::<Vec<_>>()
                                    .join(" → ");
                                preds_block.push_str(&format!("      → {target}: {chain}\n"));
                            }
                        }
                        if !preds_block.is_empty() {
                            text.push_str("\n\n--- preconditions ---");
                            text.push_str(&preds_block);
                        }
                    }
                    // Append a machine-parseable handle footer so the
                    // model can pipe this query's matches into the
                    // next turn (e.g. `path fn:foo → fn:bar`). Bounded
                    // at 50 entries to keep the budget bite small even
                    // when a query returns hundreds of nodes.
                    if want_handles
                        && let Some(ref raw) = raw_for_predicates
                    {
                        let handles = raw.handles(&session.graph);
                        if !handles.is_empty() {
                            text.push_str("\n\n--- handles ---");
                            const HANDLE_CAP: usize = 50;
                            let total = handles.len();
                            for h in handles.iter().take(HANDLE_CAP) {
                                text.push('\n');
                                text.push_str(h);
                            }
                            if total > HANDLE_CAP {
                                text.push_str(&format!(
                                    "\n... and {} more (use a tighter query to see all)",
                                    total - HANDLE_CAP
                                ));
                            }
                        }
                    }
                    if output.was_truncated {
                        ExecutionResult::success(format!(
                            "{text}\n\n[Showing {}/{} nodes]",
                            output.nodes_shown, output.nodes_total
                        ))
                    } else {
                        ExecutionResult::success(text)
                    }
                }
                Err(e) => ExecutionResult::failure(format!("Graph query error: {e}")),
            }
        }
        (
            ToolKind::SymbolEdit,
            ToolInput::SymbolEdit {
                handle,
                new_content,
                validate,
                dispatch_cascade,
            },
        ) => {
            let session = get_or_build_graph_session(&cwd);
            let entry = match session.symbols().resolve(&handle) {
                Some(e) => e.clone(),
                None => {
                    let fuzzy = session.symbols().resolve_fuzzy(&handle);
                    if fuzzy.is_empty() {
                        return ExecutionResult::failure(format!(
                            "Symbol not found: '{}'. Use graph_query to discover handles.",
                            handle
                        ));
                    }
                    return ExecutionResult::failure(format!(
                        "Symbol '{}' not found. Did you mean: {}?",
                        handle,
                        fuzzy
                            .iter()
                            .take(5)
                            .map(|e| e.handle.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
            };

            // v131-style cascade: when the edit changes a function
            // signature, the surrounding call sites likely need
            // updating too. Generate per-file CascadeTask descriptors
            // and surface them in the tool's success string so the
            // model knows what it needs to fix next without having
            // to grep for callers itself. Validation runs first so
            // an obviously-broken edit blocks before we touch disk.
            let mut cascade_summary = String::new();
            if validate {
                let cascade = jfc_graph::cascade::generate_cascade(
                    &session.graph,
                    &entry.node_id,
                    new_content.lines().next().unwrap_or("").trim(),
                    &format!("symbol_edit on '{handle}'"),
                );
                if !cascade.is_empty() {
                    let total_sites: usize = cascade.iter().map(|t| t.call_sites.len()).sum();
                    let mut summary = format!(
                        "\n\n--- cascade ---\n{} call site{} across {} file{} may need updating:",
                        total_sites,
                        if total_sites == 1 { "" } else { "s" },
                        cascade.len(),
                        if cascade.len() == 1 { "" } else { "s" }
                    );
                    for task in &cascade {
                        summary.push_str(&format!(
                            "\n  - {} ({} site{}): {}",
                            task.call_sites
                                .first()
                                .map(|s| s.file_path.display().to_string())
                                .unwrap_or_default(),
                            task.call_sites.len(),
                            if task.call_sites.len() == 1 { "" } else { "s" },
                            task.call_sites
                                .iter()
                                .map(|s| s.caller_name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ));
                    }
                    summary
                        .push_str("\nDispatch the Task tool per file to update them in parallel.");
                    cascade_summary = summary;
                    tracing::info!(
                        target: "jfc::tools",
                        sites = total_sites,
                        files = cascade.len(),
                        "symbol_edit produced cascade"
                    );
                    // Optional auto-queue: when the caller passed
                    // `dispatch_cascade=true` AND a TaskStore is
                    // available, drop one entry per file into the
                    // store so the user (and the model, via /tasks)
                    // sees the cascade plan as concrete trackable
                    // work. metadata.kind = "cascade" lets the UI
                    // and `/cascade` filter for these specifically.
                    if dispatch_cascade && let Some(ts) = task_store.as_ref() {
                        let mut queued_ids: Vec<String> = Vec::new();
                        for ct in &cascade {
                            let file_disp = ct
                                .call_sites
                                .first()
                                .map(|s| s.file_path.display().to_string())
                                .unwrap_or_else(|| "<unknown>".to_owned());
                            let subject = format!(
                                "Update {} call site{} in {}",
                                ct.call_sites.len(),
                                if ct.call_sites.len() == 1 { "" } else { "s" },
                                file_disp,
                            );
                            let active = format!("Updating call sites in {file_disp}");
                            match ts.create::<crate::tasks::TaskId>(
                                subject,
                                ct.instruction.clone(),
                                Some(active),
                                Vec::new(),
                            ) {
                                Ok(t) => {
                                    let metadata = serde_json::json!({
                                        "kind": "cascade",
                                        "source_handle": handle,
                                        "file": file_disp,
                                        "callers": ct
                                            .call_sites
                                            .iter()
                                            .map(|s| s.caller_name.clone())
                                            .collect::<Vec<_>>(),
                                        "new_signature": ct.new_signature,
                                    });
                                    let _ = ts.update(
                                        t.id.as_str(),
                                        crate::tasks::TaskPatch {
                                            metadata: Some(metadata),
                                            ..Default::default()
                                        },
                                    );
                                    queued_ids.push(t.id.to_string());
                                }
                                Err(e) => {
                                    tracing::warn!(
                                        target: "jfc::tools",
                                        error = %e,
                                        "cascade task create failed"
                                    );
                                }
                            }
                        }
                        if !queued_ids.is_empty() {
                            cascade_summary.push_str(&format!(
                                "\n\nQueued {} cascade task{} ({}). Use the Task tool with the \
                                 task IDs above as descriptions, or run /cascade to view them.",
                                queued_ids.len(),
                                if queued_ids.len() == 1 { "" } else { "s" },
                                queued_ids.join(", "),
                            ));
                        }
                    }
                }
            }

            let file_content = match std::fs::read_to_string(&entry.file_path) {
                Ok(c) => c,
                Err(e) => return ExecutionResult::failure(format!("Read failed: {e}")),
            };

            let start = entry.span.byte_range.start;
            let end = entry.span.byte_range.end;
            if end > file_content.len() {
                return ExecutionResult::failure(
                    "Span out of bounds — file changed since graph was built",
                );
            }

            let new_file = format!(
                "{}{}{}",
                &file_content[..start],
                new_content,
                &file_content[end..]
            );
            if let Err(e) = std::fs::write(&entry.file_path, &new_file) {
                return ExecutionResult::failure(format!("Write failed: {e}"));
            }
            // Invalidate the cached graph session for this workspace so
            // the next graph_query re-parses the modified file and the
            // user sees the symbol's new shape. Also queue the file
            // for auto-context injection on the next stream call.
            invalidate_graph_session_cache(Some(&cwd));
            record_edited_file(&entry.file_path);

            ExecutionResult::success(format!(
                "Edited symbol '{}' in {}{}",
                handle,
                entry.file_path.display(),
                cascade_summary
            ))
        }
        (
            ToolKind::PostBounty,
            ToolInput::PostBounty {
                description,
                budget,
                acceptance_criteria,
                max_solvers,
                auto_dispatch,
            },
        ) => {
            // The orchestrator's lock is process-global; only one
            // post_bounty runs at a time. That's fine — bounties are
            // posted in the LLM's main loop, not from concurrent
            // subagents. If two tool calls race, the second waits.
            //
            // Posting always succeeds first. If `auto_dispatch=true`,
            // we then drop the lock, run the cycle (which spawns
            // real subagent LLM calls and can take minutes), and
            // re-acquire the lock to read the settlement. Holding
            // the orchestrator mutex across the network round-trips
            // would block /market and concurrent post_bounty calls.
            let bounty_id = {
                let mut orch = market_orchestrator().lock().await;
                match orch.post_bounty(description, budget, acceptance_criteria, max_solvers) {
                    Ok(id) => id,
                    Err(e) => {
                        return ExecutionResult::failure(format!("post_bounty failed: {e}"));
                    }
                }
            };
            let max_solvers_text = match max_solvers {
                Some(n) => n.to_string(),
                None => {
                    let orch = market_orchestrator().lock().await;
                    orch.charter().max_solvers.to_string()
                }
            };
            if !auto_dispatch {
                return ExecutionResult::success(format!(
                    "Bounty `{bounty_id}` registered. State=Open, budget={budget} tok, \
                     max_solvers={max_solvers_text}. Solvers and validators have NOT \
                     run yet — the post step only registers the bounty in the market. \
                     To execute the full Post→Solve→Validate→Settle cycle (real LLM \
                     subagents compete + cross-validate), call run_bounty with \
                     bounty_id=\"{bounty_id}\". Or repost with auto_dispatch=true to \
                     register and run in one shot."
                ));
            }
            // Drive the real cycle. The orchestrator mutex is
            // dropped before the await so /market and concurrent
            // post_bounty calls aren't blocked across the network
            // round-trips.
            let Some((provider, model)) = snapshot_active_provider() else {
                return ExecutionResult::success(format!(
                    "Bounty `{bounty_id}` registered (budget {budget} tok, \
                     max_solvers={max_solvers_text}, State=Open). \
                     auto_dispatch=true was requested but the tool layer \
                     has no active provider registered, so the cycle did \
                     not run. The bounty stays Open — call run_bounty \
                     once the provider is wired."
                ));
            };
            let invoker = EconomyAgentInvoker::new(provider, model);
            let swarm = EconomySwarmProvider::new(cwd.clone());
            // Solver + validator counts: respect the bounty's
            // max_solvers, default to 2 to keep the per-bounty
            // round-trip count predictable. One validator per
            // surviving solution — sealed validation gives one
            // independent verdict per solver.
            let n_solvers = max_solvers.unwrap_or(2).clamp(1, 5);
            tracing::info!(
                target: "jfc::ui::bounty",
                bounty_id = %bounty_id,
                n_solvers = n_solvers,
                cwd = %cwd.display(),
                "post_bounty auto_dispatch: kicking off cycle"
            );
            let cycle_result = {
                let mut orch = market_orchestrator().lock().await;
                orch.run_bounty_cycle(&bounty_id, &invoker, &swarm, n_solvers, 1)
                    .await
            };
            match cycle_result {
                Ok(outcome) => {
                    let written =
                        apply_winning_solution(&cwd, &bounty_id, outcome.winning_solution.as_ref());
                    tracing::info!(
                        target: "jfc::ui::bounty",
                        bounty_id = %bounty_id,
                        winner = outcome.settlement.winner.as_ref().map(|a| a.0.as_str()).unwrap_or("(none)"),
                        files_written = written.files.len(),
                        "post_bounty auto_dispatch settled"
                    );
                    ExecutionResult::success(format!(
                        "Bounty `{bounty_id}` settled.\n\
                         Winner: {}\n\
                         Total cost: {} tok\n\
                         Payouts: {}\n\
                         Trust updates: {}\n\
                         {}\n\
                         Run /market to see updated trust + budget.",
                        outcome
                            .settlement
                            .winner
                            .as_ref()
                            .map(|a| a.0.as_str())
                            .unwrap_or("(no winning solution)"),
                        outcome.settlement.total_cost,
                        outcome.settlement.payouts.len(),
                        outcome.settlement.trust_updates.len(),
                        written.summary,
                    ))
                }
                Err(e) => ExecutionResult::failure(format!(
                    "auto_dispatch cycle for `{bounty_id}` failed: {e}"
                )),
            }
        }
        (
            ToolKind::RunBounty,
            ToolInput::RunBounty {
                bounty_id,
                max_solvers,
            },
        ) => {
            // Drive an already-posted Open bounty through the full
            // Solve→Validate→Settle cycle. Same code path as
            // PostBounty's auto_dispatch=true, just without the
            // post step. Lets the model post first (cheap registration)
            // and dispatch later when ready, instead of all-or-nothing.
            let Some((provider, model)) = snapshot_active_provider() else {
                return ExecutionResult::failure(
                    "run_bounty: no active provider registered with the \
                     tool layer. main.rs must call \
                     tools::register_active_provider during startup.",
                );
            };
            // Verify the bounty exists and is in Open state before
            // we go through all the worktree + LLM-call setup.
            let state = {
                let orch = market_orchestrator().lock().await;
                orch.bounty_state(&bounty_id)
            };
            let Some(state) = state else {
                return ExecutionResult::failure(format!(
                    "run_bounty: bounty `{bounty_id}` not found"
                ));
            };
            if !matches!(state, jfc_economy::types::MarketState::Open) {
                return ExecutionResult::failure(format!(
                    "run_bounty: bounty `{bounty_id}` is in state {state:?}, \
                     not Open — only Open bounties can be dispatched"
                ));
            }
            let invoker = EconomyAgentInvoker::new(provider, model);
            let swarm = EconomySwarmProvider::new(cwd.clone());
            let n_solvers = max_solvers.unwrap_or(2).clamp(1, 5);
            tracing::info!(
                target: "jfc::ui::bounty",
                bounty_id = %bounty_id,
                n_solvers = n_solvers,
                cwd = %cwd.display(),
                "run_bounty: kicking off cycle"
            );
            let cycle_result = {
                let mut orch = market_orchestrator().lock().await;
                orch.run_bounty_cycle(&bounty_id, &invoker, &swarm, n_solvers, 1)
                    .await
            };
            match cycle_result {
                Ok(outcome) => {
                    let written =
                        apply_winning_solution(&cwd, &bounty_id, outcome.winning_solution.as_ref());
                    tracing::info!(
                        target: "jfc::ui::bounty",
                        bounty_id = %bounty_id,
                        winner = outcome.settlement.winner.as_ref().map(|a| a.0.as_str()).unwrap_or("(none)"),
                        files_written = written.files.len(),
                        "run_bounty settled"
                    );
                    ExecutionResult::success(format!(
                        "Bounty `{bounty_id}` settled.\n\
                         Winner: {}\n\
                         Total cost: {} tok\n\
                         Payouts: {}\n\
                         Trust updates: {}\n\
                         {}\n\
                         Run /market or market_status to see updated trust + budget.",
                        outcome
                            .settlement
                            .winner
                            .as_ref()
                            .map(|a| a.0.as_str())
                            .unwrap_or("(no winning solution)"),
                        outcome.settlement.total_cost,
                        outcome.settlement.payouts.len(),
                        outcome.settlement.trust_updates.len(),
                        written.summary,
                    ))
                }
                Err(e) => ExecutionResult::failure(format!(
                    "run_bounty cycle for `{bounty_id}` failed: {e}"
                )),
            }
        }
        (ToolKind::MarketStatus, ToolInput::MarketStatus { bounty_id }) => {
            let orch = market_orchestrator().lock().await;
            let detector = match collusion_detector().lock() {
                Ok(g) => g,
                Err(e) => {
                    return ExecutionResult::failure(format!(
                        "collusion detector mutex poisoned: {e}"
                    ));
                }
            };
            let report = jfc_economy::reporting::MarketReport::generate(&orch, &detector, 0, 0);
            let critical = report.health.is_critical();
            let mut body = format!(
                "Market: {} bounties total ({} active) · spent {} / remaining {} tok\n\
                 Health: composite={:.2} (eff={:.2}, fair={:.2}, trust={:.2}, budget={:.2})",
                report.total_bounties,
                report.active_bounties,
                report.total_spent,
                report.remaining_budget,
                report.health.composite,
                report.health.efficiency,
                report.health.fairness,
                report.health.trust,
                report.health.budget_adherence,
            );
            if critical {
                body.push_str(" [CRITICAL]");
            }
            if !report.flagged_agents.is_empty() {
                body.push_str("\nFlagged agents:");
                for f in &report.flagged_agents {
                    body.push_str(&format!("\n  - {f}"));
                }
            }
            if let Some(id) = bounty_id
                && let Some(state) = orch.bounty_state(&id)
            {
                body.push_str(&format!("\nBounty `{id}` state: {state:?}"));
                if matches!(state, jfc_economy::types::MarketState::Open) {
                    body.push_str(" — call run_bounty to drive Solve→Validate→Settle.");
                }
            }
            ExecutionResult::success(body)
        }
        (
            ToolKind::MultiEdit,
            ToolInput::MultiEdit { file_path, edits },
        ) => {
            // Apply each edit in order. Each edit sees the previous
            // edit's output, so later edits can reference text that
            // earlier edits introduced. Bails on the first edit that
            // doesn't match — partial application would leave the
            // file in a half-edited state the model has to recover
            // from. Same contract as v132.
            let path = std::path::PathBuf::from(&file_path);
            let mut content = match tokio::fs::read_to_string(&path).await {
                Ok(s) => s,
                Err(e) => {
                    return ExecutionResult::failure(format!(
                        "MultiEdit: cannot read {file_path}: {e}"
                    ));
                }
            };
            let edit_array = match edits.as_array() {
                Some(a) => a,
                None => return ExecutionResult::failure(
                    "MultiEdit: `edits` must be an array of {old_string, new_string} objects".to_string(),
                ),
            };
            let mut applied = 0usize;
            for (i, edit) in edit_array.iter().enumerate() {
                let old = edit.get("old_string").and_then(|v| v.as_str()).unwrap_or("");
                let new_s = edit.get("new_string").and_then(|v| v.as_str()).unwrap_or("");
                let replace_all = edit
                    .get("replace_all")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                if old.is_empty() {
                    return ExecutionResult::failure(format!(
                        "MultiEdit: edit {} has empty old_string",
                        i + 1
                    ));
                }
                if !content.contains(old) {
                    return ExecutionResult::failure(format!(
                        "MultiEdit: edit {} of {} — old_string not found. \
                         Earlier edits applied: {applied}. \
                         Read the file and retry with the current contents.",
                        i + 1,
                        edit_array.len()
                    ));
                }
                content = if replace_all {
                    content.replace(old, new_s)
                } else {
                    let occurrences = content.matches(old).count();
                    if occurrences > 1 {
                        return ExecutionResult::failure(format!(
                            "MultiEdit: edit {} matched {occurrences} times — \
                             pass `replace_all: true` or include more context to disambiguate.",
                            i + 1
                        ));
                    }
                    content.replacen(old, new_s, 1)
                };
                applied += 1;
            }
            if let Err(e) = tokio::fs::write(&path, &content).await {
                return ExecutionResult::failure(format!("MultiEdit: write {file_path}: {e}"));
            }
            tracing::info!(
                target: "jfc::tools::multi_edit",
                file_path = %file_path,
                applied,
                bytes = content.len(),
                "MultiEdit applied"
            );
            ExecutionResult::success(format!("Applied {applied} edits to {file_path}."))
        }
        (
            ToolKind::AskUserQuestion,
            ToolInput::AskUserQuestion {
                question,
                options,
                multi_select,
            },
        ) => {
            // Surface the prompt to the user as a special transcript
            // entry. The user replies with text that the next turn
            // sees as the tool result. We don't block here because
            // jfc has no modal-prompt UI yet — the entry pattern is
            // "post the question, return immediately, treat the next
            // user message as the answer."
            let opts_repr: Vec<String> = options
                .as_array()
                .map(|arr| {
                    arr.iter()
                        .filter_map(|opt| {
                            let label = opt.get("label").and_then(|v| v.as_str())?;
                            let desc = opt.get("description").and_then(|v| v.as_str()).unwrap_or("");
                            if desc.is_empty() {
                                Some(format!("- {label}"))
                            } else {
                                Some(format!("- {label} — {desc}"))
                            }
                        })
                        .collect()
                })
                .unwrap_or_default();
            let body = format!(
                "**Question for you:** {question}\n\n{}\n\n_(Reply with your choice{} as your next message.)_",
                opts_repr.join("\n"),
                if multi_select { "(s)" } else { "" }
            );
            // The transcript itself surfaces the question; we don't
            // fire a toast since AppEvent::Toast's exact shape varies
            // across builds and the transcript line is enough for the
            // user to act on.
            tracing::info!(
                target: "jfc::tools::ask",
                question = %question.chars().take(80).collect::<String>(),
                option_count = opts_repr.len(),
                multi = multi_select,
                "AskUserQuestion surfaced"
            );
            ExecutionResult::success(format!(
                "{body}\n\n(The user's next message is your tool result.)"
            ))
        }
        (ToolKind::WebFetch, ToolInput::WebFetch { url, prompt }) => {
            // v132 caches WebFetch results per-URL with a 15-minute TTL so
            // the model can iterate on a document it just fetched without
            // re-downloading. Cache HIT returns immediately with a
            // `<system-reminder>` flag so the model knows the body is from
            // a previous fetch (matters if the URL was a live endpoint).
            if let Some(cached) = crate::web_cache::get(&url) {
                let prompt_hint = prompt
                    .as_ref()
                    .map(|p| format!("Focus: {p}\n\n"))
                    .unwrap_or_default();
                tracing::debug!(
                    target: "jfc::tools::webfetch",
                    %url,
                    cached_bytes = cached.len(),
                    "WebFetch cache HIT"
                );
                return ExecutionResult::success(format!(
                    "{}\n\nGET {url} → 200 (cached)\n\n{prompt_hint}{cached}",
                    crate::system_reminder::format(
                        "WebFetch result served from cache (last fetch <15min ago). \
                         If you need fresh content, re-issue with a cache-busting query \
                         parameter."
                    ),
                ));
            }

            // Use reqwest with a short timeout. Strips HTML to text
            // when content-type indicates HTML; otherwise returns
            // the body as-is. The optional `prompt` is *not* applied
            // here (we don't run a second LLM pass) — it's surfaced
            // verbatim in the tool result so the model sees its own
            // intent and can summarize during the next turn.
            let client = match reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .user_agent("jfc/0.1 (https://github.com/anthropics/jfc)")
                .build()
            {
                Ok(c) => c,
                Err(e) => return ExecutionResult::failure(format!("WebFetch: client init: {e}")),
            };
            let resp = match client.get(&url).send().await {
                Ok(r) => r,
                Err(e) => return ExecutionResult::failure(format!("WebFetch: {url}: {e}")),
            };
            let status = resp.status();
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_owned();
            let body = resp.text().await.unwrap_or_default();
            let body = if content_type.contains("html") {
                // Cheap HTML→text: strip tags. A real impl would use
                // scraper/html5ever; this is an MVP.
                strip_html_tags(&body)
            } else {
                body
            };
            // Cap to 50 KB so the tool result doesn't blow context.
            let truncated = if body.len() > 50_000 {
                format!("{}\n\n[...truncated, full {} bytes]", &body[..50_000], body.len())
            } else {
                body
            };
            // Cache successful 2xx responses only — caching errors would
            // mask transient outages on retry.
            if status.is_success() {
                crate::web_cache::put(&url, truncated.clone());
            }
            let prompt_hint = prompt
                .as_ref()
                .map(|p| format!("Focus: {p}\n\n"))
                .unwrap_or_default();
            ExecutionResult::success(format!(
                "GET {url} → {status}\n\n{prompt_hint}{truncated}"
            ))
        }
        (ToolKind::WebSearch, ToolInput::WebSearch { query, max_results: _ }) => {
            // jfc doesn't ship a search backend — this is a stub that
            // tells the model to fall back to manual search. v132's
            // WebSearch goes through Anthropic's hosted search API
            // which we don't have access to from this client.
            ExecutionResult::failure(format!(
                "WebSearch not yet wired in jfc. As a workaround, suggest the \
                 user run a search themselves and paste results, OR use WebFetch \
                 against a known URL. Query was: {query}"
            ))
        }
        (ToolKind::ExitPlanMode, ToolInput::ExitPlanMode { plan }) => {
            // Hand the plan off to the UI thread so all permission-mode
            // mutations stay on a single task. The model's tool result
            // is the success acknowledgment — the actual mode flip
            // happens when the main loop drains AppEvent::ExitPlanModeRequested.
            if let Some(tx) = snapshot_event_sender() {
                let _ = tx
                    .send(crate::app::AppEvent::ExitPlanModeRequested { plan: plan.clone() })
                    .await;
                tracing::info!(
                    target: "jfc::tools::plan_mode",
                    plan_bytes = plan.len(),
                    "ExitPlanMode dispatched to UI thread"
                );
                ExecutionResult::success(
                    "Plan presented to user. Permission mode transitions \
                     from Plan to AcceptEdits — you may now perform the \
                     destructive operations described in the plan.".to_string(),
                )
            } else {
                tracing::warn!(
                    target: "jfc::tools::plan_mode",
                    "ExitPlanMode called but no AppEvent sender registered"
                );
                ExecutionResult::failure(
                    "ExitPlanMode failed: UI event channel unavailable.".to_string(),
                )
            }
        }
        (ToolKind::Mcp(advertised_name), ToolInput::Mcp { arguments, .. }) => {
            // Route through the global MCP registry. The registry is
            // populated at startup from `[mcp.<name>]` config blocks;
            // if it's missing, MCP isn't wired in this build (e.g.
            // headless test) — surface a clean failure so the model
            // can recover rather than thinking the call hung.
            let Some(registry) = snapshot_mcp_registry() else {
                return ExecutionResult::failure(
                    "MCP registry not initialized — restart jfc with the MCP module enabled."
                        .to_string(),
                );
            };
            match crate::mcp::dispatch_tool(&registry, &advertised_name, arguments).await {
                Ok(outcome) if outcome.is_error => ExecutionResult::failure(outcome.text),
                Ok(outcome) => ExecutionResult::success(outcome.text),
                Err(e) => ExecutionResult::failure(format!("MCP dispatch failed: {e}")),
            }
        }
        (
            ToolKind::CronCreate,
            ToolInput::CronCreate {
                schedule,
                command,
                description,
            },
        ) => execute_cron_create(&schedule, &command, &description),
        (ToolKind::CronList, ToolInput::CronList) => execute_cron_list(),
        (ToolKind::CronDelete, ToolInput::CronDelete { id }) => execute_cron_delete(&id),
        (
            ToolKind::ScheduleWakeup,
            ToolInput::ScheduleWakeup {
                delay_seconds,
                prompt,
                reason,
            },
        ) => execute_schedule_wakeup(delay_seconds, &prompt, &reason),
        (ToolKind::Monitor, ToolInput::Monitor { command, until }) => {
            execute_monitor(&command, &until, &cwd).await
        }
        (
            ToolKind::Lsp,
            ToolInput::Lsp {
                kind: req_kind,
                file,
                line,
                column,
            },
        ) => execute_lsp(&req_kind, &file, line, column, &cwd).await,
        (ToolKind::PushNotification, ToolInput::PushNotification { message, title }) => {
            execute_push_notification(&message, title.as_deref())
        }
        (
            ToolKind::RemoteTrigger,
            ToolInput::RemoteTrigger {
                trigger_id,
                payload,
            },
        ) => execute_remote_trigger(&trigger_id, payload.as_ref()).await,
        (ToolKind::EnterPlanMode, ToolInput::EnterPlanMode { reason }) => {
            execute_enter_plan_mode(&reason).await
        }
        (ToolKind::EnterWorktree, ToolInput::EnterWorktree { name, branch }) => {
            execute_enter_worktree(&name, branch.as_deref(), &cwd).await
        }
        (ToolKind::ExitWorktree, ToolInput::ExitWorktree) => execute_exit_worktree(&cwd).await,
        (ToolKind::NotebookRead, ToolInput::NotebookRead { path }) => {
            execute_notebook_read(&path).await
        }
        (
            ToolKind::NotebookEdit,
            ToolInput::NotebookEdit {
                path,
                cell_id,
                new_source,
                edit_mode,
            },
        ) => execute_notebook_edit(&path, &cell_id, &new_source, edit_mode.as_deref()).await,
        (kind, _) => ExecutionResult::failure(format!("Tool {:?} not yet implemented", kind)),
    }
}

