//! Lifecycle hook system with enum dispatch.
//!
//! Hooks fire at 29 defined points in the agent lifecycle, matching
//! Claude Code's hook surface area. All dispatch is via enum match —
//! no trait objects, no dynamic dispatch.
//!
//! Hook points are grouped into categories:
//! - **Tool lifecycle**: before/after tool dispatch, tool error, tool batch
//! - **Stream lifecycle**: before/after streaming, model response
//! - **Session lifecycle**: start, end, compact, heartbeat
//! - **Permission lifecycle**: request, granted, denied, mode change
//! - **File system**: file changed, cwd changed
//! - **Agent lifecycle**: spawned, terminated, idle, message sent/received
//! - **Configuration**: config changed, instructions loaded
//! - **Memory/Context**: memory created/deleted, context update
//! - **Tasks**: task created, task completed
//! - **Extensions**: skill invoked, bounty posted, bounty settled

/// Points in the lifecycle where hooks can fire.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HookPoint {
    // ── Tool lifecycle ──────────────────────────────────────────────────
    /// Before a tool is dispatched (can Skip, Replace, or Abort).
    BeforeToolDispatch,
    /// After a tool completes successfully.
    AfterToolDispatch,
    /// Before a batch of tools is dispatched (multi-tool turn).
    BeforeToolBatch,
    /// After a batch of tools completes.
    AfterToolBatch,
    /// When a tool execution fails.
    OnToolError,
    /// When a tool requires permission approval.
    OnToolApproval,

    // ── Stream lifecycle ────────────────────────────────────────────────
    /// Before streaming begins (prompt is about to be sent).
    BeforeStream,
    /// After streaming completes (full response received).
    AfterStream,
    /// When a model response chunk arrives (for real-time processing).
    OnModelResponse,

    // ── Session lifecycle ───────────────────────────────────────────────
    /// When a new session starts.
    OnSessionStart,
    /// When a session ends (user exit or programmatic).
    OnSessionEnd,
    /// Before context compaction happens.
    BeforeCompact,
    /// After context compaction completes.
    AfterCompact,
    /// Periodic heartbeat (for health monitoring / keep-alive).
    OnHeartbeat,

    // ── Permission lifecycle ────────────────────────────────────────────
    /// When a permission is requested from the user.
    OnPermissionRequest,
    /// When a permission is granted.
    OnPermissionGranted,
    /// When a permission is denied.
    OnPermissionDenied,

    // ── File system ────────────────────────────────────────────────────
    /// When a file is changed (written/edited/deleted).
    OnFileChanged,
    /// When the working directory changes.
    OnCwdChanged,

    // ── Agent lifecycle ─────────────────────────────────────────────────
    /// When a subagent/teammate is spawned.
    OnAgentSpawned,
    /// When a subagent/teammate terminates.
    OnAgentTerminated,
    /// When a teammate goes idle (waiting for work).
    OnTeammateIdle,
    /// When a message is sent (to teammate or leader).
    OnMessageSent,
    /// When a message is received (from teammate or leader).
    OnMessageReceived,

    // ── Configuration ──────────────────────────────────────────────────
    /// When configuration changes (settings, .jfc/ files, etc).
    OnConfigChanged,
    /// When instructions/system prompt is loaded or reloaded.
    OnInstructionsLoaded,
    /// When the user submits a prompt (before processing).
    OnUserPromptSubmit,

    // ── Memory/Context ─────────────────────────────────────────────────
    /// When a memory is created.
    OnMemoryCreated,
    /// When a memory is deleted.
    OnMemoryDeleted,

    // ── Tasks/Economy ──────────────────────────────────────────────────
    /// When a task is created.
    OnTaskCreated,
    /// When a task is completed.
    OnTaskCompleted,
}

/// Action a hook can take.
#[derive(Debug, Clone)]
pub enum HookAction {
    /// Continue to next hook / proceed with operation.
    Continue,
    /// Skip the operation (tool not executed, no error).
    Skip,
    /// Replace the tool input with a different one.
    Replace(String),
    /// Abort with an error message.
    Abort(String),
    /// Emit metadata (non-blocking, for telemetry/logging).
    Emit(HookMetadata),
}

/// Metadata emitted by a hook (non-blocking).
#[derive(Debug, Clone)]
pub struct HookMetadata {
    pub key: String,
    pub value: String,
}

/// Context passed to hooks — expanded with richer lifecycle data.
#[derive(Debug, Clone)]
pub struct HookContext {
    pub tool_name: String,
    pub tool_input: String,
    pub session_id: String,
    pub intent: Option<String>,
    /// Name of file affected (for OnFileChanged, etc).
    pub file_path: Option<String>,
    /// Agent/teammate name (for agent lifecycle hooks).
    pub agent_name: Option<String>,
    /// Additional key-value metadata.
    pub extra: Vec<(String, String)>,
}

impl HookContext {
    /// Create a minimal context for tool-related hooks.
    pub fn for_tool(tool_name: &str, tool_input: &str, session_id: impl AsRef<str>) -> Self {
        Self {
            tool_name: tool_name.to_string(),
            tool_input: tool_input.to_string(),
            session_id: session_id.as_ref().to_string(),
            intent: None,
            file_path: None,
            agent_name: None,
            extra: Vec::new(),
        }
    }

    /// Create a context for file-related hooks.
    pub fn for_file(file_path: &str, session_id: impl AsRef<str>) -> Self {
        Self {
            tool_name: String::new(),
            tool_input: String::new(),
            session_id: session_id.as_ref().to_string(),
            intent: None,
            file_path: Some(file_path.to_string()),
            agent_name: None,
            extra: Vec::new(),
        }
    }

    /// Create a context for agent lifecycle hooks.
    pub fn for_agent(agent_name: &str, session_id: impl AsRef<str>) -> Self {
        Self {
            tool_name: String::new(),
            tool_input: String::new(),
            session_id: session_id.as_ref().to_string(),
            intent: None,
            file_path: None,
            agent_name: Some(agent_name.to_string()),
            extra: Vec::new(),
        }
    }

    /// Create a context for session-level hooks.
    ///
    /// Accepts anything `AsRef<str>` (string literal, `String`, or
    /// `SessionId`) so call sites don't have to thread the typed id
    /// through `.as_str()` at every fire.
    pub fn for_session(session_id: impl AsRef<str>) -> Self {
        Self {
            tool_name: String::new(),
            tool_input: String::new(),
            session_id: session_id.as_ref().to_string(),
            intent: None,
            file_path: None,
            agent_name: None,
            extra: Vec::new(),
        }
    }

    /// Add extra metadata.
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((key.into(), value.into()));
        self
    }
}

/// Concrete hook handlers — enum dispatch, no dyn.
#[derive(Debug, Clone)]
pub enum HookHandler {
    /// Logs the hook invocation (for debugging).
    Logger,
    /// Permission check (delegates to permission system).
    PermissionCheck,
    /// Intent enrichment (adds intent to context).
    IntentEnricher,
    /// Comment/slop checker.
    CommentChecker,
    /// Shell command executor (runs a user-defined command).
    ///
    /// **Limitation — fire-and-forget semantics.** `ShellCommand`
    /// handlers spawn a child process and return [`HookAction::Continue`]
    /// immediately. They **cannot veto a tool call** (no Skip / Abort /
    /// Replace), their stdout/stderr is **not collected**, and their
    /// exit code is **not checked**. The spawn failure itself is
    /// silently dropped.
    ///
    /// Use a Rust handler ([`HookHandler::Custom`] or a new variant) for
    /// any blocking pre-tool veto behavior. `ShellCommand` is suitable
    /// only for informational side effects (notifications, log shipping,
    /// metrics).
    ShellCommand { command: String },
    /// Custom function (for testing and extensibility).
    Custom { name: String, action: HookAction },
}

impl HookHandler {
    pub fn execute(&self, point: HookPoint, ctx: &HookContext) -> HookAction {
        match self {
            Self::Logger => {
                tracing::debug!(
                    point = ?point,
                    tool = %ctx.tool_name,
                    file = ?ctx.file_path,
                    agent = ?ctx.agent_name,
                    "hook fired"
                );
                HookAction::Continue
            }
            Self::PermissionCheck => {
                // Placeholder — actual integration via permission system
                HookAction::Continue
            }
            Self::IntentEnricher => {
                #[cfg(feature = "intent-gate")]
                {
                    tracing::debug!("intent enricher hook fired");
                }
                HookAction::Continue
            }
            Self::CommentChecker => {
                let slop_patterns = [
                    "// This function",
                    "// This method",
                    "// TODO: implement",
                    "#[allow(unused)]",
                    "/* eslint-disable */",
                ];
                let has_slop = slop_patterns
                    .iter()
                    .any(|pattern| ctx.tool_input.contains(pattern));
                if has_slop {
                    tracing::warn!(
                        target: "jfc::hooks::comment_check",
                        tool = %ctx.tool_name,
                        "AI-slop pattern detected in tool output"
                    );
                }
                HookAction::Continue
            }
            Self::ShellCommand { command } => {
                // Fire-and-forget: spawn and return immediately. See the
                // doc comment on `HookHandler::ShellCommand` for the full
                // limitation list (no veto, no output capture, no exit
                // status). The trace below is the only observability hook
                // available.
                tracing::debug!(
                    target: "jfc::hooks::shell",
                    point = ?point,
                    tool = %ctx.tool_name,
                    session_id = %ctx.session_id,
                    command = %command,
                    "spawning ShellCommand hook (fire-and-forget)"
                );
                let _ = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .env("JFC_HOOK_POINT", format!("{point:?}"))
                    .env("JFC_TOOL_NAME", &ctx.tool_name)
                    .env("JFC_SESSION_ID", &ctx.session_id)
                    .env("JFC_FILE_PATH", ctx.file_path.as_deref().unwrap_or(""))
                    .env("JFC_AGENT_NAME", ctx.agent_name.as_deref().unwrap_or(""))
                    .spawn();
                HookAction::Continue
            }
            Self::Custom { action, .. } => action.clone(),
        }
    }
}

/// Registry of hooks, fired in registration order (FIFO).
pub struct HookRegistry {
    hooks: Vec<(HookPoint, HookHandler)>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self { hooks: Vec::new() }
    }

    pub fn register(&mut self, point: HookPoint, handler: HookHandler) {
        self.hooks.push((point, handler));
    }

    /// Register a handler for multiple hook points at once.
    pub fn register_multi(&mut self, points: &[HookPoint], handler: HookHandler) {
        for &point in points {
            self.hooks.push((point, handler.clone()));
        }
    }

    /// Fire all hooks registered for the given point.
    /// Short-circuits on first Skip or Abort.
    pub fn fire(&self, point: HookPoint, ctx: &HookContext) -> HookAction {
        for (hook_point, handler) in &self.hooks {
            if *hook_point == point {
                let action = handler.execute(point, ctx);
                match &action {
                    HookAction::Continue | HookAction::Emit(_) => continue,
                    HookAction::Skip | HookAction::Abort(_) | HookAction::Replace(_) => {
                        return action;
                    }
                }
            }
        }
        HookAction::Continue
    }

    /// Fire hooks for the given point in registration order, ignoring all
    /// returned actions (no Skip/Replace/Abort short-circuit). For
    /// informational hooks where we don't need the result (heartbeat,
    /// telemetry, etc).
    ///
    /// **WARNING**: This is misnamed — it runs **synchronously** on the
    /// caller's thread. The "async" in the name refers to the
    /// fire-and-forget intent, not to any async runtime semantics. Handler
    /// errors and veto actions are dropped (best-effort). If a handler
    /// blocks (e.g. a `ShellCommand` spawn that contends on a slow
    /// subprocess setup), the caller blocks too.
    ///
    /// Prefer [`HookRegistry::fire`] when veto behavior is required.
    pub fn fire_async(&self, point: HookPoint, ctx: &HookContext) {
        for (hook_point, handler) in &self.hooks {
            if *hook_point == point {
                let _ = handler.execute(point, ctx);
            }
        }
    }

    /// Number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty()
    }

    /// Get all registered hook points (unique).
    pub fn registered_points(&self) -> Vec<HookPoint> {
        let mut points: Vec<HookPoint> = self.hooks.iter().map(|(p, _)| *p).collect();
        points.dedup();
        points
    }

    /// Remove all hooks for a specific point.
    pub fn clear_point(&mut self, point: HookPoint) {
        self.hooks.retain(|(p, _)| *p != point);
    }

    /// Remove all hooks.
    pub fn clear_all(&mut self) {
        self.hooks.clear();
    }
}

impl Default for HookRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Convenience: default hook setup with standard handlers.
pub fn default_registry() -> HookRegistry {
    let mut registry = HookRegistry::new();
    registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
    registry.register(HookPoint::BeforeToolDispatch, HookHandler::PermissionCheck);
    registry.register(HookPoint::AfterToolDispatch, HookHandler::CommentChecker);
    registry.register(HookPoint::OnSessionStart, HookHandler::Logger);
    registry.register(HookPoint::OnSessionEnd, HookHandler::Logger);
    registry.register(HookPoint::OnAgentSpawned, HookHandler::Logger);
    registry.register(HookPoint::OnAgentTerminated, HookHandler::Logger);
    registry.register(HookPoint::OnFileChanged, HookHandler::Logger);
    registry.register(HookPoint::OnUserPromptSubmit, HookHandler::Logger);
    registry.register(HookPoint::BeforeStream, HookHandler::Logger);
    registry.register(HookPoint::AfterStream, HookHandler::Logger);
    registry
}

// ─── Process-global registry ────────────────────────────────────────────────

use std::sync::OnceLock;

static GLOBAL_REGISTRY: OnceLock<HookRegistry> = OnceLock::new();

/// Initialize the process-global registry (idempotent, first call wins).
/// Call once from `main.rs` after settings are loaded.
pub fn init_global(registry: HookRegistry) {
    let _ = GLOBAL_REGISTRY.set(registry);
}

/// Fire a hook against the process-global registry. No-op if no registry
/// has been initialized — keeps the hook surface zero-cost when disabled.
pub fn fire(point: HookPoint, ctx: &HookContext) -> HookAction {
    if let Some(reg) = GLOBAL_REGISTRY.get() {
        reg.fire(point, ctx)
    } else {
        HookAction::Continue
    }
}

/// Fire a hook on the global registry without short-circuit logic. Used
/// at high-frequency sites (heartbeat, model-response chunks) where we
/// don't want per-call overhead from veto handling.
///
/// **WARNING**: Misnamed — runs synchronously on the caller's thread
/// (see [`HookRegistry::fire_async`] for the underlying behavior).
/// Handler veto actions are ignored.
pub fn fire_async(point: HookPoint, ctx: &HookContext) {
    if let Some(reg) = GLOBAL_REGISTRY.get() {
        reg.fire_async(point, ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context() -> HookContext {
        HookContext::for_tool("bash", "cargo test", "session-1")
    }

    fn assert_continue(action: HookAction) {
        assert!(matches!(action, HookAction::Continue));
    }

    #[test]
    fn test_all_hook_points_compile() {
        // Ensure exhaustive match compiles — if you add a variant, this breaks.
        let points = [
            HookPoint::BeforeToolDispatch,
            HookPoint::AfterToolDispatch,
            HookPoint::BeforeToolBatch,
            HookPoint::AfterToolBatch,
            HookPoint::OnToolError,
            HookPoint::OnToolApproval,
            HookPoint::BeforeStream,
            HookPoint::AfterStream,
            HookPoint::OnModelResponse,
            HookPoint::OnSessionStart,
            HookPoint::OnSessionEnd,
            HookPoint::BeforeCompact,
            HookPoint::AfterCompact,
            HookPoint::OnHeartbeat,
            HookPoint::OnPermissionRequest,
            HookPoint::OnPermissionGranted,
            HookPoint::OnPermissionDenied,
            HookPoint::OnFileChanged,
            HookPoint::OnCwdChanged,
            HookPoint::OnAgentSpawned,
            HookPoint::OnAgentTerminated,
            HookPoint::OnTeammateIdle,
            HookPoint::OnMessageSent,
            HookPoint::OnMessageReceived,
            HookPoint::OnConfigChanged,
            HookPoint::OnInstructionsLoaded,
            HookPoint::OnUserPromptSubmit,
            HookPoint::OnMemoryCreated,
            HookPoint::OnMemoryDeleted,
            HookPoint::OnTaskCreated,
            HookPoint::OnTaskCompleted,
        ];
        assert_eq!(points.len(), 31);
    }

    #[test]
    fn test_fire_continues_through_loggers() {
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        assert_continue(registry.fire(HookPoint::BeforeToolDispatch, &context()));
    }

    #[test]
    fn test_fire_short_circuits_on_abort() {
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(
            HookPoint::BeforeToolDispatch,
            HookHandler::Custom {
                name: "abort".to_string(),
                action: HookAction::Abort("blocked".to_string()),
            },
        );
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        match registry.fire(HookPoint::BeforeToolDispatch, &context()) {
            HookAction::Abort(message) => assert_eq!(message, "blocked"),
            action => panic!("expected abort, got {action:?}"),
        }
    }

    #[test]
    fn test_fire_short_circuits_on_skip() {
        let mut registry = HookRegistry::new();
        registry.register(
            HookPoint::BeforeToolDispatch,
            HookHandler::Custom {
                name: "skip".to_string(),
                action: HookAction::Skip,
            },
        );
        assert!(matches!(
            registry.fire(HookPoint::BeforeToolDispatch, &context()),
            HookAction::Skip
        ));
    }

    #[test]
    fn test_fire_only_matching_point() {
        let mut registry = HookRegistry::new();
        registry.register(
            HookPoint::AfterToolDispatch,
            HookHandler::Custom {
                name: "abort".to_string(),
                action: HookAction::Abort("wrong-point".to_string()),
            },
        );
        assert_continue(registry.fire(HookPoint::BeforeToolDispatch, &context()));
    }

    #[test]
    fn test_register_multi() {
        let mut registry = HookRegistry::new();
        registry.register_multi(
            &[
                HookPoint::OnSessionStart,
                HookPoint::OnSessionEnd,
                HookPoint::OnHeartbeat,
            ],
            HookHandler::Logger,
        );
        assert_eq!(registry.len(), 3);
    }

    #[test]
    fn test_context_constructors() {
        let tool_ctx = HookContext::for_tool("bash", "ls", "s1");
        assert_eq!(tool_ctx.tool_name, "bash");

        let file_ctx = HookContext::for_file("/tmp/foo.rs", "s1");
        assert_eq!(file_ctx.file_path.as_deref(), Some("/tmp/foo.rs"));

        let agent_ctx = HookContext::for_agent("solver-1", "s1");
        assert_eq!(agent_ctx.agent_name.as_deref(), Some("solver-1"));

        let session_ctx = HookContext::for_session("s1").with_extra("reason", "user-exit");
        assert_eq!(
            session_ctx.extra[0],
            ("reason".to_string(), "user-exit".to_string())
        );
    }

    #[test]
    fn test_clear_point() {
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        registry.register(HookPoint::AfterToolDispatch, HookHandler::Logger);
        registry.register(HookPoint::BeforeToolDispatch, HookHandler::Logger);
        assert_eq!(registry.len(), 3);
        registry.clear_point(HookPoint::BeforeToolDispatch);
        assert_eq!(registry.len(), 1);
    }

    #[test]
    fn test_default_registry() {
        let registry = default_registry();
        assert!(!registry.is_empty());
        assert!(registry.len() >= 6);
    }

    #[test]
    fn test_comment_checker_detects_slop() {
        let ctx = HookContext::for_tool(
            "write",
            "// This function updates state\nfn update() {}",
            "s1",
        );
        assert_continue(HookHandler::CommentChecker.execute(HookPoint::AfterToolDispatch, &ctx));
    }

    #[test]
    fn test_emit_action_continues() {
        let mut registry = HookRegistry::new();
        registry.register(
            HookPoint::OnHeartbeat,
            HookHandler::Custom {
                name: "emit".to_string(),
                action: HookAction::Emit(HookMetadata {
                    key: "uptime".to_string(),
                    value: "3600".to_string(),
                }),
            },
        );
        registry.register(HookPoint::OnHeartbeat, HookHandler::Logger);
        // Emit doesn't short-circuit — logger still fires
        assert_continue(registry.fire(HookPoint::OnHeartbeat, &HookContext::for_session("s1")));
    }
}
