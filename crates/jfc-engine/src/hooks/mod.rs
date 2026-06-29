//! Lifecycle hook system with enum dispatch.
//!
//! Hooks fire at 45+ defined points in the agent lifecycle, matching
//! Claude Code 2.1.167's hook surface area. All dispatch is via enum match —
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

use std::sync::{Arc, Mutex, OnceLock};

use jfc_plugin_host::{HookValue, PluginHost, PluginHostError, PluginRegistration};
use jfc_plugin_sdk::{HookName, PluginId, PluginManifest, PluginSource, PluginVersion};
use serde_json::{Value, json};

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
    /// Maps to CC's `PreCompact` hook event.
    BeforeCompact,
    /// After context compaction completes.
    /// Maps to CC's `PostCompact` hook event.
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

    // ── Additional agent lifecycle ──────────────────────────────────────
    /// When a subagent stops/terminates.
    SubagentStop,
    /// When the model's response turn ends (before user sees output).
    Stop,
    /// When a tool execution fails with an error.
    PostToolUseFailure,

    // ── CC 2.1.167 additions ────────────────────────────────────────────
    /// Fires before the first model turn; output injected as additional context.
    /// Maps to CC's `Setup` hook event.
    OnSetup,
    /// Fires when a slash-command expands before prompt submission.
    /// Maps to CC's `UserPromptExpansion` hook event.
    OnUserPromptExpansion,
    /// Fires as assistant text streams; hook can rewrite displayed content.
    /// Maps to CC's `MessageDisplay` hook event.
    OnMessageDisplay,
    /// Fires when an MCP server requests structured user input (elicitation/create).
    /// Maps to CC's `Elicitation` hook event.
    OnElicitation,
    /// Fires after the user responds to an elicitation.
    /// Maps to CC's `ElicitationResult` hook event.
    OnElicitationResult,
    /// Fires after a batch of tools completes.
    /// Maps to CC's `PostToolBatch` hook event.
    PostToolBatch,
    /// Fires after context compaction completes.
    /// Maps to CC's `PostCompact` hook event.
    PostCompact,
    /// Fires when a subagent starts.
    /// Maps to CC's `SubagentStart` hook event.
    SubagentStart,
    /// Fires when a worktree is created.
    /// Maps to CC's `WorktreeCreate` hook event.
    WorktreeCreate,
    /// Fires when a worktree is removed.
    /// Maps to CC's `WorktreeRemove` hook event.
    WorktreeRemove,
    /// Fires when configuration changes.
    /// Maps to CC's `ConfigChange` hook event.
    ConfigChange,
    /// Fires when a stop operation fails.
    /// Maps to CC's `StopFailure` hook event.
    StopFailure,
    /// Fires when the user interrupts a running turn (Ctrl-C / Esc-Esc).
    OnUserInterrupt,
    /// Fires just before the engine blocks on interactive user input
    /// (permission modal, AskUserQuestion, elicitation, etc.).
    OnUserInputRequired,
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
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    /// Environment variables to inject into shell hook commands.
    pub env_vars: Vec<(String, String)>,
}

fn hook_name_for_point(point: HookPoint) -> Option<HookName> {
    match point {
        HookPoint::BeforeToolDispatch => Some(HookName::PreToolUse),
        HookPoint::AfterToolDispatch => Some(HookName::PostToolUse),
        HookPoint::PostToolUseFailure => Some(HookName::PostToolUseFailure),
        HookPoint::OnUserPromptSubmit => Some(HookName::UserPromptSubmit),
        HookPoint::OnSessionStart => Some(HookName::SessionStart),
        HookPoint::OnSessionEnd => Some(HookName::SessionEnd),
        HookPoint::Stop => Some(HookName::Stop),
        HookPoint::OnSetup => Some(HookName::Setup),
        HookPoint::OnUserPromptExpansion => Some(HookName::UserPromptExpansion),
        HookPoint::OnFileChanged => Some(HookName::FileChanged),
        HookPoint::OnCwdChanged => Some(HookName::CwdChanged),
        HookPoint::SubagentStart => Some(HookName::SubagentStart),
        HookPoint::SubagentStop => Some(HookName::SubagentStop),
        HookPoint::OnUserInterrupt => Some(HookName::UserInterrupt),
        HookPoint::OnModelResponse => Some(HookName::ModelResponseChunk),
        HookPoint::OnUserInputRequired => Some(HookName::UserInputRequired),
        HookPoint::PostToolBatch | HookPoint::AfterToolBatch => Some(HookName::PostToolBatch),
        HookPoint::BeforeCompact => Some(HookName::BeforeCompact),
        HookPoint::PostCompact => Some(HookName::PostCompact),
        HookPoint::AfterCompact => Some(HookName::AfterCompact),
        HookPoint::OnPermissionRequest => Some(HookName::OnPermissionRequest),
        HookPoint::OnPermissionDenied => Some(HookName::OnPermissionDenied),
        HookPoint::OnMessageDisplay => Some(HookName::OnMessageDisplay),
        HookPoint::OnElicitation => Some(HookName::OnElicitation),
        HookPoint::OnElicitationResult => Some(HookName::OnElicitationResult),
        HookPoint::OnTaskCreated => Some(HookName::OnTaskCreated),
        HookPoint::OnTaskCompleted => Some(HookName::OnTaskCompleted),
        HookPoint::WorktreeCreate => Some(HookName::WorktreeCreate),
        HookPoint::WorktreeRemove => Some(HookName::WorktreeRemove),
        HookPoint::ConfigChange | HookPoint::OnConfigChanged => Some(HookName::ConfigChange),
        HookPoint::OnInstructionsLoaded => Some(HookName::OnInstructionsLoaded),
        HookPoint::OnTeammateIdle => Some(HookName::OnTeammateIdle),
        HookPoint::StopFailure => Some(HookName::StopFailure),
        HookPoint::BeforeStream => Some(HookName::BeforeStream),
        HookPoint::AfterStream => Some(HookName::AfterStream),
        HookPoint::BeforeToolBatch
        | HookPoint::OnToolError
        | HookPoint::OnToolApproval
        | HookPoint::OnPermissionGranted
        | HookPoint::OnAgentSpawned
        | HookPoint::OnAgentTerminated
        | HookPoint::OnMessageSent
        | HookPoint::OnMessageReceived
        | HookPoint::OnMemoryCreated
        | HookPoint::OnMemoryDeleted
        | HookPoint::OnHeartbeat => None,
    }
}

fn point_for_hook_name(name: HookName) -> Option<HookPoint> {
    match name {
        HookName::PreToolUse => Some(HookPoint::BeforeToolDispatch),
        HookName::PostToolUse => Some(HookPoint::AfterToolDispatch),
        HookName::PostToolUseFailure => Some(HookPoint::PostToolUseFailure),
        HookName::UserPromptSubmit => Some(HookPoint::OnUserPromptSubmit),
        HookName::SessionStart => Some(HookPoint::OnSessionStart),
        HookName::SessionEnd => Some(HookPoint::OnSessionEnd),
        HookName::Stop => Some(HookPoint::Stop),
        HookName::Setup => Some(HookPoint::OnSetup),
        HookName::UserPromptExpansion => Some(HookPoint::OnUserPromptExpansion),
        HookName::FileChanged => Some(HookPoint::OnFileChanged),
        HookName::CwdChanged => Some(HookPoint::OnCwdChanged),
        HookName::SubagentStart => Some(HookPoint::SubagentStart),
        HookName::SubagentStop => Some(HookPoint::SubagentStop),
        HookName::UserInterrupt => Some(HookPoint::OnUserInterrupt),
        HookName::ModelResponseChunk => Some(HookPoint::OnModelResponse),
        HookName::UserInputRequired => Some(HookPoint::OnUserInputRequired),
        HookName::PostToolBatch => Some(HookPoint::PostToolBatch),
        HookName::BeforeCompact => Some(HookPoint::BeforeCompact),
        HookName::PostCompact => Some(HookPoint::PostCompact),
        HookName::AfterCompact => Some(HookPoint::AfterCompact),
        HookName::OnPermissionRequest => Some(HookPoint::OnPermissionRequest),
        HookName::OnPermissionDenied => Some(HookPoint::OnPermissionDenied),
        HookName::OnMessageDisplay => Some(HookPoint::OnMessageDisplay),
        HookName::OnElicitation => Some(HookPoint::OnElicitation),
        HookName::OnElicitationResult => Some(HookPoint::OnElicitationResult),
        HookName::OnTaskCreated => Some(HookPoint::OnTaskCreated),
        HookName::OnTaskCompleted => Some(HookPoint::OnTaskCompleted),
        HookName::WorktreeCreate => Some(HookPoint::WorktreeCreate),
        HookName::WorktreeRemove => Some(HookPoint::WorktreeRemove),
        HookName::ConfigChange => Some(HookPoint::ConfigChange),
        HookName::OnInstructionsLoaded => Some(HookPoint::OnInstructionsLoaded),
        HookName::OnTeammateIdle => Some(HookPoint::OnTeammateIdle),
        HookName::StopFailure => Some(HookPoint::StopFailure),
        HookName::BeforeStream => Some(HookPoint::BeforeStream),
        HookName::AfterStream => Some(HookPoint::AfterStream),
        HookName::Notification | HookName::CommandExecuteBefore | HookName::ToolDefinition => None,
    }
}

fn hook_context_value(ctx: &HookContext) -> Value {
    json!({
        "tool_name": ctx.tool_name,
        "tool_input": ctx.tool_input,
        "session_id": ctx.session_id,
        "intent": ctx.intent,
        "file_path": ctx.file_path,
        "agent_name": ctx.agent_name,
        "extra": ctx.extra,
        "env_vars": ctx.env_vars,
    })
}

fn hook_value_for_action(action: HookAction, context: Value) -> HookValue {
    let payload = match action {
        HookAction::Continue => json!({ "action": "continue", "context": context }),
        HookAction::Skip => json!({ "action": "skip", "context": context }),
        HookAction::Replace(value) => {
            json!({ "action": "replace", "value": value, "context": context })
        }
        HookAction::Abort(message) => {
            json!({ "action": "abort", "message": message, "context": context })
        }
        HookAction::Emit(metadata) => json!({
            "action": "emit",
            "metadata": {
                "key": metadata.key,
                "value": metadata.value,
            },
            "context": context,
        }),
    };
    HookValue::json(payload)
}

fn action_from_hook_value(value: &HookValue) -> HookAction {
    let payload = value.payload();
    match payload.get("action").and_then(Value::as_str) {
        Some("skip") => HookAction::Skip,
        Some("replace") => HookAction::Replace(
            payload
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
        ),
        Some("abort") => HookAction::Abort(
            payload
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Plugin hook aborted")
                .to_owned(),
        ),
        Some("emit") => {
            let metadata = payload.get("metadata").unwrap_or(&Value::Null);
            HookAction::Emit(HookMetadata {
                key: metadata
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                value: metadata
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
            })
        }
        _ => HookAction::Continue,
    }
}

fn terminal_hook_value(value: &HookValue) -> bool {
    matches!(
        value.payload().get("action").and_then(Value::as_str),
        Some("skip" | "replace" | "abort")
    )
}

fn context_from_hook_value(value: &HookValue) -> Result<HookContext, PluginHostError> {
    let context = value
        .payload()
        .get("context")
        .ok_or_else(|| PluginHostError::plugin("missing hook context"))?;
    serde_json::from_value(context.clone())
        .map_err(|error| PluginHostError::plugin(format!("invalid hook context: {error}")))
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
            env_vars: Vec::new(),
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
            env_vars: Vec::new(),
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
            env_vars: Vec::new(),
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
            env_vars: Vec::new(),
        }
    }

    /// Add extra metadata.
    pub fn with_extra(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra.push((key.into(), value.into()));
        self
    }
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeHookSpecificOutput {
    permission_decision: Option<String>,
    permission_decision_reason: Option<String>,
}

/// Claude-compatible hook JSON output.
///
/// Expected schema fragment from Claude Code 2.1.177:
/// `suppressOutput -> boolean (optional)`.
#[allow(dead_code)]
#[derive(Debug, Clone, Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ClaudeHookOutput {
    #[serde(rename = "continue")]
    continue_run: Option<bool>,
    stop_reason: Option<String>,
    suppress_output: Option<bool>,
    decision: Option<String>,
    reason: Option<String>,
    system_message: Option<String>,
    terminal_sequence: Option<String>,
    permission_decision: Option<String>,
    hook_specific_output: Option<ClaudeHookSpecificOutput>,
}

impl ClaudeHookOutput {
    fn wants_abort(&self) -> bool {
        self.continue_run == Some(false)
            || self.decision.as_deref().is_some_and(is_block_decision)
            || self
                .permission_decision
                .as_deref()
                .is_some_and(is_deny_decision)
            || self
                .hook_specific_output
                .as_ref()
                .and_then(|output| output.permission_decision.as_deref())
                .is_some_and(is_deny_decision)
    }

    fn abort_reason(&self, fallback: impl Into<String>) -> String {
        self.hook_specific_output
            .as_ref()
            .and_then(|output| non_empty_owned(output.permission_decision_reason.as_deref()))
            .or_else(|| non_empty_owned(self.stop_reason.as_deref()))
            .or_else(|| non_empty_owned(self.reason.as_deref()))
            .or_else(|| non_empty_owned(self.system_message.as_deref()))
            .unwrap_or_else(|| fallback.into())
    }

    fn suppress_output(&self) -> bool {
        self.suppress_output.unwrap_or(false)
    }
}

fn is_block_decision(value: &str) -> bool {
    value.eq_ignore_ascii_case("block") || value.eq_ignore_ascii_case("deny")
}

fn is_deny_decision(value: &str) -> bool {
    value.eq_ignore_ascii_case("deny") || value.eq_ignore_ascii_case("block")
}

fn non_empty_owned(value: Option<&str>) -> Option<String> {
    let value = value?.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
}

fn parse_claude_hook_output(stdout: &[u8]) -> Option<ClaudeHookOutput> {
    let text = std::str::from_utf8(stdout).ok()?.trim();
    if text.is_empty() || !text.starts_with('{') {
        return None;
    }
    serde_json::from_str(text).ok()
}

fn hook_exit_message(status: std::process::ExitStatus) -> String {
    format!("Hook blocked: exit {}", status.code().unwrap_or(1))
}

fn hook_abort_message_from_output(
    stdout: &[u8],
    status: std::process::ExitStatus,
    parsed: Option<&ClaudeHookOutput>,
) -> String {
    let fallback = hook_exit_message(status);
    if let Some(output) = parsed {
        if output.wants_abort() {
            return output.abort_reason("Blocked by hook");
        }
        if output.suppress_output() {
            return fallback;
        }
        if let Some(reason) = output.reason.as_deref().and_then(|reason| {
            let trimmed = reason.trim();
            (!trimmed.is_empty()).then_some(trimmed.to_string())
        }) {
            return reason;
        }
        return fallback;
    }

    let msg = String::from_utf8_lossy(stdout).trim().to_string();
    if msg.is_empty() { fallback } else { msg }
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
    /// Execute a shell command. Exit 0 = allow (Continue), non-zero = block
    /// (Abort with stdout as message). Claude-compatible JSON stdout can
    /// return `continue: false`, `decision: "block"`,
    /// `permissionDecision: "deny"`, and `suppressOutput: true`.
    /// Optionally filter by tool name pattern.
    Shell {
        /// Shell command to execute.
        command: String,
        /// If true, run async (fire-and-forget in a background thread),
        /// don't block on result.
        async_mode: bool,
        /// Optional tool name pattern to match (e.g. "Bash", "Edit|Write").
        /// `None` matches all tools.
        matcher: Option<String>,
    },
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
                if contains_comment_slop(&ctx.tool_input) {
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
            Self::Shell {
                command,
                async_mode,
                matcher,
            } => {
                // Check matcher against tool name in ctx
                if let Some(pattern) = matcher {
                    let tool_name = ctx.tool_name.as_str();
                    let matches = pattern.split('|').any(|p| p.trim() == tool_name);
                    if !matches {
                        return HookAction::Continue;
                    }
                }
                if *async_mode {
                    // Fire-and-forget in a background thread.
                    // Capped at 8 concurrent hook threads to prevent
                    // unbounded thread growth from chatty hooks.
                    use std::sync::atomic::{AtomicUsize, Ordering};
                    static HOOK_ACTIVE: AtomicUsize = AtomicUsize::new(0);
                    const HOOK_CAP: usize = 8;

                    if HOOK_ACTIVE.load(Ordering::Acquire) >= HOOK_CAP {
                        tracing::debug!(
                            target: "jfc::hooks",
                            command = %command,
                            "hook thread cap ({HOOK_CAP}) reached, skipping async hook"
                        );
                        return HookAction::Continue;
                    }
                    HOOK_ACTIVE.fetch_add(1, Ordering::AcqRel);

                    let cmd = command.clone();
                    let env_vars = ctx.env_vars.clone();
                    let hook_point_str = format!("{point:?}");
                    let tool_name = ctx.tool_name.clone();
                    let session_id = ctx.session_id.clone();
                    let _ = std::thread::Builder::new()
                        .name("jfc-hook".into())
                        .spawn(move || {
                            let _ = std::process::Command::new("sh")
                                .arg("-c")
                                .arg(&cmd)
                                .env("JFC_HOOK_POINT", &hook_point_str)
                                .env("JFC_TOOL_NAME", &tool_name)
                                .env("JFC_SESSION_ID", &session_id)
                                .envs(env_vars)
                                .output();
                            HOOK_ACTIVE.fetch_sub(1, Ordering::AcqRel);
                        });
                    return HookAction::Continue;
                }
                // Synchronous: run and check exit status
                match std::process::Command::new("sh")
                    .arg("-c")
                    .arg(command)
                    .env("JFC_HOOK_POINT", format!("{point:?}"))
                    .env("JFC_TOOL_NAME", &ctx.tool_name)
                    .env("JFC_SESSION_ID", &ctx.session_id)
                    .envs(ctx.env_vars.iter().map(|(k, v)| (k.as_str(), v.as_str())))
                    .output()
                {
                    Ok(out) if out.status.success() => {
                        if let Some(output) = parse_claude_hook_output(&out.stdout) {
                            if output.wants_abort() {
                                return HookAction::Abort(output.abort_reason("Blocked by hook"));
                            }
                        }
                        HookAction::Continue
                    }
                    Ok(out) => {
                        let parsed = parse_claude_hook_output(&out.stdout);
                        HookAction::Abort(hook_abort_message_from_output(
                            &out.stdout,
                            out.status,
                            parsed.as_ref(),
                        ))
                    }
                    Err(e) => HookAction::Abort(format!("Hook exec error: {e}")),
                }
            }
            Self::Custom { action, .. } => action.clone(),
        }
    }
}

const COMMENT_SLOP_PATTERNS: &[&str] = &[
    "// This function",
    "// This method",
    "// TODO: implement",
    "/* eslint-disable */",
];

fn contains_comment_slop(tool_input: &str) -> bool {
    COMMENT_SLOP_PATTERNS
        .iter()
        .any(|pattern| tool_input.contains(pattern))
}

/// Registry of hooks, fired in registration order (FIFO).
pub struct HookRegistry {
    hooks: Vec<(HookPoint, HookHandler)>,
    plugin_host: Option<Arc<Mutex<PluginHost>>>,
    /// Per-handler activation metrics, keyed by `"<HookPoint:?>#<index>"`.
    metrics: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, HookMetrics>>>,
}

impl HookRegistry {
    pub fn new() -> Self {
        Self {
            hooks: Vec::new(),
            plugin_host: None,
            metrics: std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        }
    }

    /// Snapshot of current per-handler metrics (for `/hooks status`).
    pub fn metrics_snapshot(&self) -> std::collections::HashMap<String, HookMetrics> {
        self.metrics
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    fn record_metric(
        metrics: &std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, HookMetrics>>>,
        key: &str,
        dur: std::time::Duration,
    ) {
        if let Ok(mut map) = metrics.lock() {
            let entry = map.entry(key.to_owned()).or_default();
            entry.fire_count += 1;
            entry.last_fired_at = Some(std::time::SystemTime::now());
            entry.total_duration_ms = entry
                .total_duration_ms
                .saturating_add(dur.as_millis() as u64);
        }
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
        for (idx, (hook_point, handler)) in self.hooks.iter().enumerate() {
            if *hook_point == point {
                let t0 = std::time::Instant::now();
                let action = handler.execute(point, ctx);
                let dur = t0.elapsed();
                let key = format!("{point:?}#{idx}");
                Self::record_metric(&self.metrics, &key, dur);
                match &action {
                    HookAction::Continue | HookAction::Emit(_) => continue,
                    HookAction::Skip | HookAction::Abort(_) | HookAction::Replace(_) => {
                        return action;
                    }
                }
            }
        }
        self.fire_plugin_host(point, ctx)
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
        for (idx, (hook_point, handler)) in self.hooks.iter().enumerate() {
            if *hook_point == point {
                let t0 = std::time::Instant::now();
                let _ = handler.execute(point, ctx);
                let dur = t0.elapsed();
                let key = format!("{point:?}#{idx}");
                Self::record_metric(&self.metrics, &key, dur);
            }
        }
        let _ = self.fire_plugin_host(point, ctx);
    }

    /// Number of registered hooks.
    pub fn len(&self) -> usize {
        self.hooks.len() + self.plugin_host_hook_count()
    }
    pub fn is_empty(&self) -> bool {
        self.hooks.is_empty() && self.plugin_host_hook_count() == 0
    }

    /// Whether at least one handler is registered for `point`. This is used by
    /// high-frequency stream sites to skip context construction entirely when
    /// the hook surface is inactive.
    pub fn has_hooks(&self, point: HookPoint) -> bool {
        self.hooks
            .iter()
            .any(|(hook_point, _)| *hook_point == point)
            || self.plugin_host_has_hook(point)
    }

    /// Get all registered hook points (unique).
    pub fn registered_points(&self) -> Vec<HookPoint> {
        let mut points: Vec<HookPoint> = self.hooks.iter().map(|(p, _)| *p).collect();
        points.extend(self.plugin_host_points());
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
        self.plugin_host = None;
    }

    fn fire_plugin_host(&self, point: HookPoint, ctx: &HookContext) -> HookAction {
        let Some(name) = hook_name_for_point(point) else {
            return HookAction::Continue;
        };
        let Some(host) = &self.plugin_host else {
            return HookAction::Continue;
        };
        let mut host = host.lock().unwrap_or_else(|error| error.into_inner());
        let result = host.trigger_hook_until(
            name,
            hook_value_for_action(HookAction::Continue, hook_context_value(ctx)),
            terminal_hook_value,
        );
        match result {
            Ok(value) => action_from_hook_value(&value),
            Err(error) => HookAction::Abort(format!("Plugin hook error: {error}")),
        }
    }

    fn plugin_host_has_hook(&self, point: HookPoint) -> bool {
        let Some(name) = hook_name_for_point(point) else {
            return false;
        };
        let Some(host) = &self.plugin_host else {
            return false;
        };
        host.lock()
            .unwrap_or_else(|error| error.into_inner())
            .has_hook(name)
    }

    fn plugin_host_hook_count(&self) -> usize {
        let Some(host) = &self.plugin_host else {
            return 0;
        };
        host.lock()
            .unwrap_or_else(|error| error.into_inner())
            .status_snapshot()
            .plugins
            .iter()
            .map(|plugin| plugin.hooks.len())
            .sum()
    }

    fn plugin_host_points(&self) -> Vec<HookPoint> {
        let Some(host) = &self.plugin_host else {
            return Vec::new();
        };
        host.lock()
            .unwrap_or_else(|error| error.into_inner())
            .status_snapshot()
            .plugins
            .iter()
            .flat_map(|plugin| plugin.hooks.iter())
            .filter_map(|hook| point_for_hook_name(hook.name))
            .collect()
    }

    fn register_config_shell_hooks<I>(&mut self, entries: I)
    where
        I: IntoIterator<Item = (HookPoint, crate::config::ShellHookEntry)>,
    {
        let hooks = entries
            .into_iter()
            .filter_map(|(point, entry)| {
                hook_name_for_point(point).map(|name| (point, name, entry))
            })
            .collect::<Vec<_>>();
        if hooks.is_empty() {
            return;
        }

        let mut registration = PluginRegistration::new(
            PluginManifest::new(
                PluginId::new("jfc.config.shell-hooks"),
                PluginVersion::new("0.1.0"),
                PluginSource::built_in("jfc-engine"),
            )
            .with_display_name("Configured shell hooks"),
        );
        for (idx, (point, name, entry)) in hooks.into_iter().enumerate() {
            let handler = HookHandler::Shell {
                command: entry.command,
                async_mode: entry.async_mode,
                matcher: entry.matcher,
            };
            registration = registration.with_hook(name, idx as i32, move |invocation| {
                let context = context_from_hook_value(invocation.value())?;
                let context_payload = invocation
                    .value()
                    .payload()
                    .get("context")
                    .cloned()
                    .unwrap_or_else(|| hook_context_value(&context));
                Ok(hook_value_for_action(
                    handler.execute(point, &context),
                    context_payload,
                ))
            });
        }

        let mut host = PluginHost::new();
        let result = host
            .register_internal(registration)
            .and_then(|()| host.activate_all());
        if let Err(error) = result {
            tracing::warn!(
                target: "jfc::hooks",
                error = %error,
                "failed to activate configured shell-hook plugin"
            );
            return;
        }
        self.plugin_host = Some(Arc::new(Mutex::new(host)));
    }

    /// Register shell hooks from the user config's `[hooks]` section.
    /// Call once during app initialization after the config is loaded.
    pub fn register_from_config(&mut self, config: &crate::config::Config) {
        let Some(hooks_cfg) = &config.hooks else {
            return;
        };
        let mut entries = Vec::new();
        macro_rules! add_hooks {
            ($point:expr, $field:ident) => {
                entries.extend(
                    hooks_cfg
                        .$field
                        .iter()
                        .cloned()
                        .map(|entry| ($point, entry)),
                );
            };
        }

        add_hooks!(HookPoint::BeforeToolDispatch, pre_tool_use);
        add_hooks!(HookPoint::AfterToolDispatch, post_tool_use);
        add_hooks!(HookPoint::PostToolUseFailure, post_tool_use_failure);
        add_hooks!(HookPoint::OnUserPromptSubmit, user_prompt_submit);
        add_hooks!(HookPoint::OnSessionStart, session_start);
        add_hooks!(HookPoint::OnSessionEnd, session_end);
        add_hooks!(HookPoint::Stop, stop);
        add_hooks!(HookPoint::SubagentStop, subagent_stop);
        add_hooks!(HookPoint::OnSetup, setup);
        add_hooks!(HookPoint::OnUserPromptExpansion, user_prompt_expansion);
        add_hooks!(HookPoint::OnMessageDisplay, message_display);
        add_hooks!(HookPoint::OnElicitation, elicitation);
        add_hooks!(HookPoint::OnElicitationResult, elicitation_result);
        add_hooks!(HookPoint::PostToolBatch, post_tool_batch);
        add_hooks!(HookPoint::BeforeCompact, pre_compact);
        add_hooks!(HookPoint::PostCompact, post_compact);
        add_hooks!(HookPoint::SubagentStart, subagent_start);
        add_hooks!(HookPoint::OnPermissionRequest, permission_request);
        add_hooks!(HookPoint::OnPermissionDenied, permission_denied);
        add_hooks!(HookPoint::OnTaskCreated, task_created);
        add_hooks!(HookPoint::OnTaskCompleted, task_completed);
        add_hooks!(HookPoint::WorktreeCreate, worktree_create);
        add_hooks!(HookPoint::WorktreeRemove, worktree_remove);
        add_hooks!(HookPoint::ConfigChange, config_change);
        add_hooks!(HookPoint::OnInstructionsLoaded, instructions_loaded);
        add_hooks!(HookPoint::OnCwdChanged, cwd_changed);
        add_hooks!(HookPoint::OnFileChanged, file_changed);
        add_hooks!(HookPoint::OnTeammateIdle, teammate_idle);
        add_hooks!(HookPoint::StopFailure, stop_failure);
        add_hooks!(HookPoint::OnUserInterrupt, user_interrupt);
        add_hooks!(HookPoint::OnModelResponse, model_response_chunk);
        add_hooks!(HookPoint::OnUserInputRequired, user_input_required);
        self.register_config_shell_hooks(entries);
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

/// True when the global registry has at least one handler for `point`.
/// Cheap guard for hot paths that would otherwise allocate hook context on
/// every streaming chunk even when no hook can observe it.
pub fn has_hooks(point: HookPoint) -> bool {
    GLOBAL_REGISTRY
        .get()
        .is_some_and(|reg| reg.has_hooks(point))
}

/// Snapshot the global registry's per-handler activation metrics.
/// Returns an empty map if no registry has been initialized.
pub fn metrics_snapshot() -> std::collections::HashMap<String, HookMetrics> {
    GLOBAL_REGISTRY
        .get()
        .map(|reg| reg.metrics_snapshot())
        .unwrap_or_default()
}

/// List all `(HookPoint, handler_index)` tuples from the global registry.
/// Used by `/hooks status` to build the display table.
pub fn registered_hooks_summary() -> Vec<(HookPoint, usize)> {
    GLOBAL_REGISTRY
        .get()
        .map(|reg| {
            let mut summary = reg
                .hooks
                .iter()
                .enumerate()
                .map(|(idx, (point, _))| (*point, idx))
                .collect::<Vec<_>>();
            let mut next_index = reg.hooks.len();
            for point in reg.plugin_host_points() {
                summary.push((point, next_index));
                next_index = next_index.saturating_add(1);
            }
            summary
        })
        .unwrap_or_default()
}

// ─── Script-based hook runner (lifecycle events from .jfc/hooks/) ──────────
//
// This is a parallel surface to the in-process `HookRegistry` above. The
// registry handles fast in-tree dispatch (Logger, CommentChecker, ...);
// the `runner` submodule scans `.jfc/hooks/` for user-authored scripts
// (e.g. `pre-tool-use.sh`) and runs them with the event payload on stdin.
//
// Both systems are intentionally separate: the registry is process-local
// and zero-cost; the runner spawns subprocesses and is opt-in per event.
pub mod runner;

/// Per-rule activation metrics tracked by the hook registry.
///
/// Stored in a `HashMap<String, HookMetrics>` keyed by `"<point_debug>/<handler_index>"`.
/// Updated every time a handler runs. Exposed via `/hooks status`.
#[derive(Debug, Clone, Default)]
pub struct HookMetrics {
    /// Total number of times this handler has fired.
    pub fire_count: u64,
    /// Wall-clock time of the most recent invocation.
    pub last_fired_at: Option<std::time::SystemTime>,
    /// Cumulative execution time across all invocations (milliseconds).
    pub total_duration_ms: u64,
}

/// Lifecycle events that script hooks subscribe to.
///
/// Distinct from `HookPoint` (the in-process registry's enum) because
/// script hooks need the *payload* serialized to JSON, whereas registry
/// hooks operate on a borrowed `HookContext`.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum HookEvent {
    PreToolUse {
        tool_name: String,
        tool_input: serde_json::Value,
        /// Session-cumulative input tokens at the time this hook fires.
        session_input_tokens: u64,
        /// Session-cumulative output tokens at the time this hook fires.
        session_output_tokens: u64,
        /// Session-cumulative cost in USD at the time this hook fires.
        session_cost_usd: f64,
    },
    PostToolUse {
        tool_name: String,
        tool_output: String,
        is_error: bool,
        /// Session-cumulative input tokens at the time this hook fires.
        session_input_tokens: u64,
        /// Session-cumulative output tokens at the time this hook fires.
        session_output_tokens: u64,
        /// Session-cumulative cost in USD at the time this hook fires.
        session_cost_usd: f64,
    },
    UserPromptSubmit {
        prompt: String,
    },
    SessionStart {
        session_id: String,
    },
    /// Fires when a session ends (user exit, `/clear`, or programmatic shutdown).
    SessionEnd {
        session_id: String,
        /// 0 = normal exit; non-zero values indicate error conditions.
        exit_code: i32,
        /// Cumulative input tokens consumed by the model across this session.
        /// `null` when token tracking is unavailable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_input_tokens: Option<u64>,
        /// Cumulative output tokens generated by the model across this session.
        /// `null` when token tracking is unavailable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_output_tokens: Option<u64>,
        /// Estimated cumulative cost in USD across this session.
        /// `null` when cost tracking is unavailable.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_cost_usd: Option<f64>,
    },
    FileChanged {
        path: String,
    },
    CwdChanged {
        old: String,
        new: String,
    },
    Notification {
        message: String,
    },
    /// Fires when a background subagent reaches a terminal state.
    SubagentStop {
        task_id: String,
        description: String,
        /// `"completed"` | `"failed"` | `"cancelled"`
        status: String,
    },
    /// Fires when the user interrupts a running turn (Ctrl-C, Esc-Esc).
    UserInterrupt {
        session_id: String,
        /// `"ctrl_c"` | `"question"` | `"elicitation"`
        reason: String,
    },
    /// Fires on each streamed model-response text chunk.
    ///
    /// **High-frequency** — only register handlers for this event if they
    /// are genuinely cheap and non-blocking. The runner will skip firing
    /// if no `.jfc/hooks/model-response-chunk.*` script is found (the
    /// common case) so overhead is zero when unused.
    ModelResponseChunk {
        chunk: String,
        /// `true` on the final chunk of a turn (after stream end).
        is_final: bool,
    },
    /// Fires when the engine is about to block on interactive user input
    /// (permission modal, AskUserQuestion, elicitation, etc.).
    UserInputRequired {
        /// `"permission"` | `"question"` | `"elicitation"`
        kind: String,
        message: String,
    },
}

impl HookEvent {
    /// Script-name stem used for filesystem matching (without extension).
    /// e.g. `pre-tool-use` → matches `.jfc/hooks/pre-tool-use.sh` (or `.json`).
    pub fn script_name(&self) -> &'static str {
        match self {
            HookEvent::PreToolUse { .. } => "pre-tool-use",
            HookEvent::PostToolUse { .. } => "post-tool-use",
            HookEvent::UserPromptSubmit { .. } => "user-prompt-submit",
            HookEvent::SessionStart { .. } => "session-start",
            HookEvent::SessionEnd { .. } => "session-end",
            HookEvent::FileChanged { .. } => "file-changed",
            HookEvent::CwdChanged { .. } => "cwd-changed",
            HookEvent::Notification { .. } => "notification",
            HookEvent::SubagentStop { .. } => "subagent-stop",
            HookEvent::UserInterrupt { .. } => "user-interrupt",
            HookEvent::ModelResponseChunk { .. } => "model-response-chunk",
            HookEvent::UserInputRequired { .. } => "user-input-required",
        }
    }
}

/// Decision returned by a script hook (parsed from script stdout).
#[derive(Debug, Clone, PartialEq)]
pub enum HookDecision {
    /// Continue with the original input.
    Allow,
    /// Block the operation with a human-readable reason.
    Deny { reason: String },
    /// Replace the tool input with the modified payload.
    Modify { modified_input: serde_json::Value },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, ShellHookEntry, ShellHooksConfig};

    fn context() -> HookContext {
        HookContext::for_tool("bash", "cargo test", "session-1")
    }

    fn assert_continue(action: HookAction) {
        assert!(matches!(action, HookAction::Continue));
    }

    #[test]
    fn config_shell_hook_runs_through_plugin_host_normal() {
        let mut hooks = ShellHooksConfig::default();
        hooks.pre_tool_use.push(ShellHookEntry {
            matcher: Some("bash".to_owned()),
            command: "printf '%s' '{\"decision\":\"block\",\"reason\":\"host blocked\"}'"
                .to_owned(),
            async_mode: false,
        });
        let config = Config {
            hooks: Some(hooks),
            ..Config::default()
        };
        let mut registry = HookRegistry::new();

        registry.register_from_config(&config);

        assert_eq!(registry.len(), 1);
        assert!(registry.has_hooks(HookPoint::BeforeToolDispatch));
        assert!(
            registry
                .registered_points()
                .contains(&HookPoint::BeforeToolDispatch)
        );
        match registry.fire(HookPoint::BeforeToolDispatch, &context()) {
            HookAction::Abort(message) => assert_eq!(message, "host blocked"),
            action => panic!("expected abort, got {action:?}"),
        }
    }

    #[test]
    fn shell_hook_json_block_decision_normal() {
        let hook = HookHandler::Shell {
            command: "printf '%s' '{\"decision\":\"block\",\"reason\":\"policy blocked\"}'"
                .to_string(),
            async_mode: false,
            matcher: None,
        };

        match hook.execute(HookPoint::BeforeToolDispatch, &context()) {
            HookAction::Abort(message) => assert_eq!(message, "policy blocked"),
            action => panic!("expected abort, got {action:?}"),
        }
    }

    #[test]
    fn shell_hook_continue_false_uses_stop_reason_normal() {
        let hook = HookHandler::Shell {
            command: "printf '%s' '{\"continue\":false,\"stopReason\":\"stop here\"}'".to_string(),
            async_mode: false,
            matcher: None,
        };

        match hook.execute(HookPoint::BeforeToolDispatch, &context()) {
            HookAction::Abort(message) => assert_eq!(message, "stop here"),
            action => panic!("expected abort, got {action:?}"),
        }
    }

    #[test]
    fn shell_hook_hook_specific_permission_deny_normal() {
        let hook = HookHandler::Shell {
            command: "printf '%s' '{\"hookSpecificOutput\":{\"hookEventName\":\"PreToolUse\",\"permissionDecision\":\"deny\",\"permissionDecisionReason\":\"no bash\"}}'".to_string(),
            async_mode: false,
            matcher: None,
        };

        match hook.execute(HookPoint::BeforeToolDispatch, &context()) {
            HookAction::Abort(message) => assert_eq!(message, "no bash"),
            action => panic!("expected abort, got {action:?}"),
        }
    }

    #[test]
    fn shell_hook_suppress_output_hides_raw_stdout_normal() {
        let hook = HookHandler::Shell {
            command:
                "printf '%s' '{\"suppressOutput\":true,\"reason\":\"raw-json-secret\"}'; exit 1"
                    .to_string(),
            async_mode: false,
            matcher: None,
        };

        match hook.execute(HookPoint::BeforeToolDispatch, &context()) {
            HookAction::Abort(message) => {
                assert_eq!(message, "Hook blocked: exit 1");
                assert!(!message.contains("raw-json-secret"));
            }
            action => panic!("expected abort, got {action:?}"),
        }
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
            // Additional hook points from the hook-surface expansion
            HookPoint::SubagentStop,
            HookPoint::Stop,
            HookPoint::PostToolUseFailure,
            HookPoint::OnSetup,
            HookPoint::OnUserPromptExpansion,
            HookPoint::OnMessageDisplay,
            HookPoint::OnElicitation,
            HookPoint::OnElicitationResult,
            HookPoint::PostToolBatch,
            HookPoint::PostCompact,
            HookPoint::SubagentStart,
            HookPoint::WorktreeCreate,
            HookPoint::WorktreeRemove,
            HookPoint::ConfigChange,
            HookPoint::StopFailure,
            // New in hook-surface expansion v2
            HookPoint::OnUserInterrupt,
            HookPoint::OnUserInputRequired,
        ];
        assert_eq!(points.len(), 48);
    }

    #[test]
    fn test_hook_event_script_names() {
        assert_eq!(
            HookEvent::SessionEnd {
                session_id: "s1".into(),
                exit_code: 0,
                session_input_tokens: None,
                session_output_tokens: None,
                session_cost_usd: None,
            }
            .script_name(),
            "session-end"
        );
        assert_eq!(
            HookEvent::SubagentStop {
                task_id: "t1".into(),
                description: "audit".into(),
                status: "completed".into(),
            }
            .script_name(),
            "subagent-stop"
        );
        assert_eq!(
            HookEvent::UserInterrupt {
                session_id: "s1".into(),
                reason: "ctrl_c".into(),
            }
            .script_name(),
            "user-interrupt"
        );
        assert_eq!(
            HookEvent::ModelResponseChunk {
                chunk: "hello".into(),
                is_final: false,
            }
            .script_name(),
            "model-response-chunk"
        );
        assert_eq!(
            HookEvent::UserInputRequired {
                kind: "permission".into(),
                message: "allow bash?".into(),
            }
            .script_name(),
            "user-input-required"
        );
    }

    #[test]
    fn test_hook_metrics_are_recorded() {
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::OnHeartbeat, HookHandler::Logger);
        registry.register(HookPoint::OnSessionStart, HookHandler::Logger);

        let ctx = HookContext::for_session("s1");
        registry.fire(HookPoint::OnHeartbeat, &ctx);
        registry.fire(HookPoint::OnHeartbeat, &ctx);
        registry.fire(HookPoint::OnSessionStart, &ctx);

        let snap = registry.metrics_snapshot();
        let hb = snap
            .get("OnHeartbeat#0")
            .expect("heartbeat metrics present");
        assert_eq!(hb.fire_count, 2, "OnHeartbeat fired twice");
        assert!(hb.last_fired_at.is_some(), "last_fired_at set");

        let ss = snap
            .get("OnSessionStart#1")
            .expect("session-start metrics present");
        assert_eq!(ss.fire_count, 1, "OnSessionStart fired once");
    }

    #[test]
    fn test_hook_metrics_async_recorded() {
        let mut registry = HookRegistry::new();
        registry.register(HookPoint::OnHeartbeat, HookHandler::Logger);

        let ctx = HookContext::for_session("s1");
        registry.fire_async(HookPoint::OnHeartbeat, &ctx);
        registry.fire_async(HookPoint::OnHeartbeat, &ctx);
        registry.fire_async(HookPoint::OnHeartbeat, &ctx);

        let snap = registry.metrics_snapshot();
        let entry = snap.get("OnHeartbeat#0").expect("metrics present");
        assert_eq!(entry.fire_count, 3);
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
        assert!(contains_comment_slop(&ctx.tool_input));
    }

    #[test]
    fn comment_checker_does_not_match_every_tool_output_regression() {
        let ctx = HookContext::for_tool("write", "fn update() {}\n", "s1");
        assert_continue(HookHandler::CommentChecker.execute(HookPoint::AfterToolDispatch, &ctx));
        assert!(!contains_comment_slop(&ctx.tool_input));
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

    #[test]
    fn test_hook_event_cost_fields_serialize_normal() {
        // PreToolUse should carry session-cumulative cost/token fields in its
        // JSON payload so hook scripts can make budget-aware decisions.
        let event = HookEvent::PreToolUse {
            tool_name: "Bash".to_string(),
            tool_input: serde_json::json!({"command": "ls"}),
            session_input_tokens: 12345,
            session_output_tokens: 678,
            session_cost_usd: 0.042,
        };
        let json = serde_json::to_string(&event).expect("serialize");
        assert!(
            json.contains("\"session_input_tokens\":12345"),
            "input tokens in payload: {json}"
        );
        assert!(
            json.contains("\"session_output_tokens\":678"),
            "output tokens in payload: {json}"
        );
        assert!(json.contains("session_cost_usd"), "cost in payload: {json}");

        // PostToolUse carries the same fields.
        let post = HookEvent::PostToolUse {
            tool_name: "Write".to_string(),
            tool_output: "ok".to_string(),
            is_error: false,
            session_input_tokens: 50000,
            session_output_tokens: 1000,
            session_cost_usd: 0.12,
        };
        let json2 = serde_json::to_string(&post).expect("serialize");
        assert!(
            json2.contains("\"session_input_tokens\":50000"),
            "input tokens in post payload: {json2}"
        );
        assert!(
            json2.contains("\"is_error\":false"),
            "is_error in payload: {json2}"
        );
    }
}
