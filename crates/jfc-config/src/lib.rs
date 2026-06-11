//! User-facing TOML config at ~/.config/jfc/config.toml.
//!
//! Schema mirrors oh-my-opencode AgentOverrideConfigSchema but trimmed to
//! the fields jfc currently understands.

pub mod atomic_write;
pub mod catch_up_state;
pub mod claude_settings;
pub mod feature_config;
pub mod keybindings;
pub mod paths;
pub mod quiet_hours;
pub mod scheduled_tasks;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub use claude_settings::ClaudeCompatibilityConfig;
/// Re-export from jfc-mcp so existing callsites keep working.
pub use jfc_mcp::McpServerConfig;

/// Top-level config.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default)]
    pub default: AgentConfig,
    #[serde(default)]
    pub agents: HashMap<String, AgentConfig>,
    #[serde(default)]
    pub categories: HashMap<String, CategoryConfig>,
    #[serde(default)]
    pub permission_automation: Option<PermissionAutomationConfig>,
    #[serde(default)]
    pub background_task: Option<BackgroundTaskConfig>,
    #[serde(default)]
    pub argus_auto_review: Option<ArgusAutoReviewConfig>,
    #[serde(default)]
    pub mcp: HashMap<String, McpServerConfig>,
    #[serde(default)]
    pub disabled_agents: Vec<String>,
    #[serde(default)]
    pub disabled_tools: Vec<String>,
    #[serde(default)]
    pub experimental: Option<ExperimentalConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_style: Option<String>,
    #[serde(
        default,
        alias = "advisorModel",
        skip_serializing_if = "Option::is_none"
    )]
    pub advisor_model: Option<String>,
    #[serde(
        default,
        alias = "advisorEnabled",
        skip_serializing_if = "Option::is_none"
    )]
    pub advisor_enabled: Option<bool>,
    #[serde(
        default,
        alias = "serverAdvisorModel",
        skip_serializing_if = "Option::is_none"
    )]
    pub server_advisor_model: Option<String>,
    /// Opt-in: route high-stakes decisions (currently the session-goal
    /// "is the condition met?" verdict) through the model Council instead of a
    /// single model, so the active model and the advisor model must agree. Off
    /// by default; the `JFC_COUNCIL_VERDICT` env var overrides it per run.
    #[serde(
        default,
        alias = "councilVerdict",
        skip_serializing_if = "Option::is_none"
    )]
    pub council_verdict: Option<bool>,
    #[serde(default)]
    pub slate_enabled: bool,
    #[serde(default)]
    pub slate_rules: Option<Vec<SlateRuleConfig>>,
    #[serde(default = "default_memory_recall_enabled")]
    pub memory_recall_enabled: bool,
    #[serde(default = "default_plan_recall_enabled")]
    pub plan_recall_enabled: bool,
    #[serde(default)]
    pub session_cost_budget_usd: Option<f64>,
    #[serde(default = "default_auto_compact_enabled")]
    pub auto_compact_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<ShellHooksConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_control: Option<RemoteControlConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub continuation: Option<ContinuationConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exploration: Option<ExplorationConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub managed_settings: Option<ManagedSettingsConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub isolation: Option<IsolationConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub worktree: Option<WorktreeConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_shell: Option<String>,
    #[serde(default, skip_serializing_if = "ClaudeCompatibilityConfig::is_empty")]
    pub claude: ClaudeCompatibilityConfig,
    /// Auto-copy the transcript selection to the clipboard on mouse-up
    /// (drag-to-select). Default on; set `copy_on_select = false` to disable
    /// the gesture entirely so clicks/drags never touch the clipboard.
    #[serde(default = "default_true")]
    pub copy_on_select: bool,
    /// When the model refuses a turn, switch to `refusal_fallback_model` and
    /// resend once (CC 2.1.160 "switch models when a message is flagged").
    /// Default on, but INERT unless `refusal_fallback_model` is also set.
    #[serde(default = "default_true")]
    pub refusal_fallback_enabled: bool,
    /// Model to retry on after a refusal (e.g. a different/safer model). `None`
    /// ⇒ no fallback (the refusal is left as-is). The user opts in by setting it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refusal_fallback_model: Option<String>,
}

/// Controls what happens when an agent requested worktree isolation but the
/// worktree could not be created. Borrowing Dolt's "agents work on an isolated
/// branch, production stays untouched" promise: a mutating agent must NOT
/// silently fall back to the main checkout.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct IsolationConfig {
    /// When true (the default), an agent that requested `isolation:"worktree"`
    /// and failed to get one is NOT silently run in the main checkout — the
    /// dispatch fails closed. Set false to restore the legacy permissive
    /// fall-back-to-cwd behaviour.
    pub fail_closed: bool,
}

impl Default for IsolationConfig {
    fn default() -> Self {
        Self { fail_closed: true }
    }
}

/// Claude-compatible worktree settings. JFC consumes `base_ref` when creating
/// new worktrees; sparse checkout and symlink-directory policy are preserved so
/// callers can inspect them even before every behavior is implemented.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct WorktreeConfig {
    #[serde(rename = "baseRef", alias = "base_ref")]
    pub base_ref: Option<String>,
    #[serde(rename = "sparsePaths", alias = "sparse_paths")]
    pub sparse_paths: Vec<String>,
    #[serde(rename = "symlinkDirectories", alias = "symlink_directories")]
    pub symlink_directories: Vec<String>,
}

/// Claude-compatible Bash sandbox settings as loaded from JSON/TOML config.
/// Enforcement lives in `jfc-engine`; this type deliberately mirrors the
/// settings shape without depending on engine internals.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SandboxConfig {
    pub enabled: Option<bool>,
    #[serde(rename = "failIfUnavailable", alias = "fail_if_unavailable")]
    pub fail_if_unavailable: Option<bool>,
    #[serde(
        rename = "autoAllowBashIfSandboxed",
        alias = "auto_allow_bash_if_sandboxed"
    )]
    pub auto_allow_bash_if_sandboxed: Option<bool>,
    #[serde(
        rename = "allowUnsandboxedCommands",
        alias = "allow_unsandboxed_commands"
    )]
    pub allow_unsandboxed_commands: Vec<String>,
    #[serde(rename = "ignoreViolations", alias = "ignore_violations")]
    pub ignore_violations: Option<bool>,
    #[serde(
        rename = "enableWeakerNestedSandbox",
        alias = "enable_weaker_nested_sandbox"
    )]
    pub enable_weaker_nested_sandbox: Option<bool>,
    #[serde(
        rename = "enableWeakerNetworkIsolation",
        alias = "enable_weaker_network_isolation"
    )]
    pub enable_weaker_network_isolation: Option<bool>,
    #[serde(rename = "excludedCommands", alias = "excluded_commands")]
    pub excluded_commands: Vec<String>,
    #[serde(rename = "bwrapPath", alias = "bwrap_path")]
    pub bwrap_path: Option<String>,
    #[serde(rename = "socatPath", alias = "socat_path")]
    pub socat_path: Option<String>,
    pub network: SandboxNetworkConfig,
    pub filesystem: SandboxFilesystemConfig,
    pub ripgrep: SandboxRipgrepConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SandboxNetworkConfig {
    #[serde(rename = "allowedDomains", alias = "allowed_domains")]
    pub allowed_domains: Vec<String>,
    #[serde(rename = "deniedDomains", alias = "denied_domains")]
    pub denied_domains: Vec<String>,
    #[serde(
        rename = "allowManagedDomainsOnly",
        alias = "allow_managed_domains_only"
    )]
    pub allow_managed_domains_only: Option<bool>,
    #[serde(rename = "allowUnixSockets", alias = "allow_unix_sockets")]
    pub allow_unix_sockets: Option<bool>,
    #[serde(rename = "allowLocalBinding", alias = "allow_local_binding")]
    pub allow_local_binding: Option<bool>,
    #[serde(rename = "httpProxyPort", alias = "http_proxy_port")]
    pub http_proxy_port: Option<u16>,
    #[serde(rename = "socksProxyPort", alias = "socks_proxy_port")]
    pub socks_proxy_port: Option<u16>,
    #[serde(rename = "tlsTermination", alias = "tls_termination")]
    pub tls_termination: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SandboxFilesystemConfig {
    #[serde(rename = "allowWrite", alias = "allow_write")]
    pub allow_write: Vec<String>,
    #[serde(rename = "denyWrite", alias = "deny_write")]
    pub deny_write: Vec<String>,
    #[serde(rename = "allowRead", alias = "allow_read")]
    pub allow_read: Vec<String>,
    #[serde(rename = "denyRead", alias = "deny_read")]
    pub deny_read: Vec<String>,
    #[serde(
        rename = "allowManagedReadPathsOnly",
        alias = "allow_managed_read_paths_only"
    )]
    pub allow_managed_read_paths_only: Option<bool>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct SandboxRipgrepConfig {
    pub command: Option<String>,
    pub args: Vec<String>,
}

/// Admin/managed settings. These may be embedded in `config.toml` or loaded
/// from a dedicated managed settings file via [`load_managed_settings`].
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ManagedSettingsConfig {
    pub disable_remote_control: bool,
    pub disable_plugin_urls: bool,
    pub disable_plugin_dirs: bool,
    pub disable_plugin_updates: bool,
    pub disable_marketplace: bool,
    pub require_oauth: bool,
    pub require_elevated_auth: bool,
    pub required_user: Option<String>,
    pub required_env: Vec<String>,
    pub security_notice: Option<String>,
    pub policy_tier: Option<String>,
    pub force_permission_mode: Option<String>,
    pub max_budget_usd: Option<f64>,
    pub spend_limit_usd: Option<f64>,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
}

/// Diagnostic record for `jfc policy status`: every candidate managed-settings
/// source and whether it contributed a usable policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ManagedSettingsSource {
    pub label: String,
    pub path: Option<PathBuf>,
    pub exists: bool,
    pub loaded: bool,
    pub error: Option<String>,
    pub settings: Option<ManagedSettingsConfig>,
}

/// `[continuation]` section in config.toml — controls self-continuation
/// (auto-driving the next in-scope step without a user "continue").
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ContinuationConfig {
    /// Auto-continue when the model stalls on a permission-asking question
    /// ("Want me to …?") or leaves queued tasks unfinished. Off by default;
    /// factory mode (`JFC_FACTORY_MODE`) implies it.
    pub auto_continue: bool,
    /// Maximum consecutive self-continuations before stopping for the user.
    pub max_self_continuations: u32,
}

impl Default for ContinuationConfig {
    fn default() -> Self {
        Self {
            auto_continue: false,
            max_self_continuations: 25,
        }
    }
}

/// `[remote_control]` section in config.toml.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RemoteControlConfig {
    /// Automatically start the remote-control WS server when jfc launches.
    pub auto_start: bool,
    /// TCP port for the WebSocket server (default 4242).
    pub port: u16,
    /// When true, the `disable_remote_control` managed-settings flag is
    /// honored — `/remote-control` refuses to start.
    pub disabled: bool,
}

/// Default remote-control WebSocket port. Mirrors
/// `jfc_remote::protocol::DEFAULT_PORT` (kept inline to avoid coupling the
/// config crate to the transport crate).
pub const DEFAULT_REMOTE_CONTROL_PORT: u16 = 4242;

impl Default for RemoteControlConfig {
    fn default() -> Self {
        Self {
            auto_start: false,
            port: DEFAULT_REMOTE_CONTROL_PORT,
            disabled: false,
        }
    }
}

fn default_memory_recall_enabled() -> bool {
    true
}
fn default_plan_recall_enabled() -> bool {
    true
}
fn default_auto_compact_enabled() -> bool {
    true
}
fn default_true() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            default: AgentConfig::default(),
            agents: HashMap::new(),
            categories: HashMap::new(),
            permission_automation: None,
            background_task: None,
            argus_auto_review: None,
            mcp: HashMap::new(),
            disabled_agents: Vec::new(),
            disabled_tools: Vec::new(),
            experimental: None,
            theme: None,
            output_style: None,
            advisor_model: None,
            advisor_enabled: None,
            server_advisor_model: None,
            council_verdict: None,
            slate_enabled: false,
            slate_rules: None,
            memory_recall_enabled: default_memory_recall_enabled(),
            plan_recall_enabled: default_plan_recall_enabled(),
            session_cost_budget_usd: None,
            auto_compact_enabled: default_auto_compact_enabled(),
            auto_compact_window: None,
            compact_instructions: None,
            hooks: None,
            remote_control: None,
            continuation: None,
            exploration: None,
            managed_settings: None,
            isolation: None,
            worktree: None,
            sandbox: None,
            default_shell: None,
            claude: ClaudeCompatibilityConfig::default(),
            copy_on_select: default_true(),
            refusal_fallback_enabled: default_true(),
            refusal_fallback_model: None,
        }
    }
}

/// `[exploration]` section in config.toml — controls the adaptive
/// effort/temperature controller. Category-specific baselines continue to live
/// in `[categories.<query-class>]` via `temperature` / `reasoning_effort`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ExplorationConfig {
    /// `adaptive` lets jfc choose effort/temperature from query class and
    /// stall/retry signals. `fixed` preserves only explicit `/effort` or
    /// `/temp` pins plus provider defaults.
    pub policy: Option<String>,
    /// Lower bound for adaptive levels, inclusive (`0..=4`).
    pub min_level: Option<u8>,
    /// Upper bound for adaptive levels, inclusive (`0..=4`).
    pub max_level: Option<u8>,
    /// How many adaptive bump rungs decay after a clean completed turn.
    pub decay: Option<u8>,
}

impl Default for ExplorationConfig {
    fn default() -> Self {
        Self {
            policy: None,
            min_level: None,
            max_level: None,
            decay: Some(1),
        }
    }
}

/// User-configurable shell hooks config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct ShellHooksConfig {
    #[serde(rename = "PreToolUse", default)]
    pub pre_tool_use: Vec<ShellHookEntry>,
    #[serde(rename = "PostToolUse", default)]
    pub post_tool_use: Vec<ShellHookEntry>,
    #[serde(rename = "PostToolUseFailure", default)]
    pub post_tool_use_failure: Vec<ShellHookEntry>,
    #[serde(rename = "UserPromptSubmit", default)]
    pub user_prompt_submit: Vec<ShellHookEntry>,
    #[serde(rename = "SessionStart", default)]
    pub session_start: Vec<ShellHookEntry>,
    #[serde(rename = "SessionEnd", default)]
    pub session_end: Vec<ShellHookEntry>,
    #[serde(rename = "Stop", default)]
    pub stop: Vec<ShellHookEntry>,
    #[serde(rename = "SubagentStop", default)]
    pub subagent_stop: Vec<ShellHookEntry>,
    /// Fires before the first model turn; hook output injected as context.
    #[serde(rename = "Setup", default)]
    pub setup: Vec<ShellHookEntry>,
    /// Fires when a slash-command expands before prompt submission.
    #[serde(rename = "UserPromptExpansion", default)]
    pub user_prompt_expansion: Vec<ShellHookEntry>,
    /// Fires as assistant text streams; hook can rewrite displayed content.
    #[serde(rename = "MessageDisplay", default)]
    pub message_display: Vec<ShellHookEntry>,
    /// Fires when an MCP server requests structured user input.
    #[serde(rename = "Elicitation", default)]
    pub elicitation: Vec<ShellHookEntry>,
    /// Fires after the user responds to an elicitation.
    #[serde(rename = "ElicitationResult", default)]
    pub elicitation_result: Vec<ShellHookEntry>,
    /// Fires after a batch of tools completes (PostToolBatch).
    #[serde(rename = "PostToolBatch", default)]
    pub post_tool_batch: Vec<ShellHookEntry>,
    /// Fires after context compaction completes.
    #[serde(rename = "PostCompact", default)]
    pub post_compact: Vec<ShellHookEntry>,
    /// Fires when a subagent starts.
    #[serde(rename = "SubagentStart", default)]
    pub subagent_start: Vec<ShellHookEntry>,
    /// Fires when a permission is requested.
    #[serde(rename = "PermissionRequest", default)]
    pub permission_request: Vec<ShellHookEntry>,
    /// Fires when a permission is denied.
    #[serde(rename = "PermissionDenied", default)]
    pub permission_denied: Vec<ShellHookEntry>,
    /// Fires after a task is created (TaskCreated).
    #[serde(rename = "TaskCreated", default)]
    pub task_created: Vec<ShellHookEntry>,
    /// Fires after a task is completed (TaskCompleted).
    #[serde(rename = "TaskCompleted", default)]
    pub task_completed: Vec<ShellHookEntry>,
    /// Fires when a worktree is created.
    #[serde(rename = "WorktreeCreate", default)]
    pub worktree_create: Vec<ShellHookEntry>,
    /// Fires when a worktree is removed.
    #[serde(rename = "WorktreeRemove", default)]
    pub worktree_remove: Vec<ShellHookEntry>,
    /// Fires when configuration changes.
    #[serde(rename = "ConfigChange", default)]
    pub config_change: Vec<ShellHookEntry>,
    /// Fires when instructions/system prompt is loaded.
    #[serde(rename = "InstructionsLoaded", default)]
    pub instructions_loaded: Vec<ShellHookEntry>,
    /// Fires when the working directory changes.
    #[serde(rename = "CwdChanged", default)]
    pub cwd_changed: Vec<ShellHookEntry>,
    /// Fires when a file is changed.
    #[serde(rename = "FileChanged", default)]
    pub file_changed: Vec<ShellHookEntry>,
    /// Fires when a teammate goes idle.
    #[serde(rename = "TeammateIdle", default)]
    pub teammate_idle: Vec<ShellHookEntry>,
    /// Fires when a stop fails.
    #[serde(rename = "StopFailure", default)]
    pub stop_failure: Vec<ShellHookEntry>,
}

/// A single user-defined shell hook entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShellHookEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matcher: Option<String>,
    pub command: String,
    #[serde(default)]
    pub async_mode: bool,
}

/// TOML form of a slate routing rule.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SlateRuleConfig {
    pub query_class: String,
    pub model: String,
    #[serde(default)]
    pub fallback_model: Option<String>,
}

/// Category-based model routing.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct CategoryConfig {
    pub model: Option<String>,
    #[serde(default)]
    pub prompt_append: Option<String>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
}

/// Permission automation rules in main config.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PermissionAutomationConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub allowed_tools: Vec<String>,
    #[serde(default)]
    pub denied_tools: Vec<String>,
    #[serde(default)]
    pub rules: Vec<PermissionRuleEntry>,
    #[serde(default = "default_true")]
    pub auto_allow_if_sandboxed: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct PermissionRuleEntry {
    pub action: String,
    pub tool: String,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Background task concurrency limits.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BackgroundTaskConfig {
    #[serde(default = "default_provider_concurrency")]
    pub provider_concurrency: usize,
    #[serde(default = "default_model_concurrency")]
    pub model_concurrency: usize,
}

fn default_provider_concurrency() -> usize {
    3
}
fn default_model_concurrency() -> usize {
    5
}

impl Default for BackgroundTaskConfig {
    fn default() -> Self {
        Self {
            provider_concurrency: 3,
            model_concurrency: 5,
        }
    }
}

/// Argus auto-review configuration.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ArgusAutoReviewConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub threshold: Option<u32>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Experimental feature flags.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ExperimentalConfig {
    #[serde(default)]
    pub fork_agent_enabled: bool,
    #[serde(default)]
    pub hashline_edit: bool,
    #[serde(default)]
    pub model_fallback: bool,
    #[serde(default)]
    pub speculation_enabled: bool,
}

/// Per-agent overrides.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct AgentConfig {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub fallback_models: Vec<FallbackModel>,
    #[serde(default)]
    pub temperature: Option<f64>,
    #[serde(default)]
    pub disallowed_tools: Vec<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub max_tokens: Option<u32>,
    #[serde(default)]
    pub thinking_budget: Option<u32>,
    #[serde(default)]
    pub permission: HashMap<String, String>,
    #[serde(default)]
    pub prompt_append: Option<String>,
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub reasoning_effort: Option<String>,
    #[serde(default)]
    pub top_p: Option<f64>,
    #[serde(default)]
    pub variant: Option<String>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub provider_options: HashMap<String, serde_json::Value>,
    #[serde(default)]
    pub compaction_model: Option<String>,
    #[serde(default)]
    pub ultrawork_model: Option<String>,
    #[serde(default)]
    pub text_verbosity: Option<String>,
    /// When true, force xhigh effort regardless of `reasoning_effort`.
    /// Mirrors Claude Code 2.1.154's `ultracode` settings key — CC's `e$7`
    /// returns "xhigh" whenever `settings.ultracode === true`, ignoring the
    /// otherwise-resolved effort level. Session-scoped in CC (provided via
    /// `--settings` / `apply_flag_settings`, not persisted by interactive
    /// toggles); in jfc the same flag at `[default]` or `[agents.<id>]` does
    /// the same job through `resolve_effort_for_model`.
    #[serde(default)]
    pub ultracode: Option<bool>,
}

/// A fallback model entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum FallbackModel {
    Simple(String),
    Detailed {
        model: String,
        #[serde(default)]
        variant: Option<String>,
        #[serde(default)]
        temperature: Option<f64>,
        #[serde(default)]
        reasoning_effort: Option<String>,
    },
}

impl FallbackModel {
    pub fn model_id(&self) -> &str {
        match self {
            Self::Simple(s) => s,
            Self::Detailed { model, .. } => model,
        }
    }
}

/// Canonical path to the config file.
pub fn config_path() -> PathBuf {
    static CONFIG_PATH: OnceLock<PathBuf> = OnceLock::new();
    CONFIG_PATH
        .get_or_init(|| {
            let path = dirs::config_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join("jfc")
                .join("config.toml");
            tracing::trace!(target: "jfc::config", path = %path.display(), "resolved config path");
            path
        })
        .clone()
}

#[derive(Clone)]
struct Cached {
    path: PathBuf,
    mtime: Option<SystemTime>,
    generation: u64,
    config: Arc<Config>,
}

static CACHE: Mutex<Option<Cached>> = Mutex::new(None);
static CACHE_GENERATION: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static READ_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
pub fn read_count() -> u64 {
    READ_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bust the cached parse.
pub fn invalidate_cache() {
    mark_config_changed();
    if let Ok(mut slot) = CACHE.lock() {
        *slot = None;
    }
}

/// Current cache invalidation generation.
pub fn cache_generation() -> u64 {
    CACHE_GENERATION.load(Ordering::Acquire)
}

/// Mark the canonical config inputs as changed.
///
/// The file watcher calls this on real config-file notifications, so the hot
/// cache path can avoid polling `metadata()` for every caller.
pub fn mark_config_changed() {
    CACHE_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Read + parse config from disk, no caching.
fn load_from(path: &Path) -> Config {
    let mut cfg = load_toml_from(path);
    if path == config_path().as_path()
        && let Ok(project_root) = std::env::current_dir()
    {
        claude_settings::apply_to_config(&mut cfg, &project_root);
    }
    cfg
}

fn load_toml_from(path: &Path) -> Config {
    #[cfg(test)]
    READ_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let raw = match std::fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::trace!(
                target: "jfc::config",
                path = %path.display(),
                "config file not found â using defaults"
            );
            return Config::default();
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::config",
                path = %path.display(),
                error = %e,
                "failed to read config file â using defaults"
            );
            return Config::default();
        }
    };
    match toml::from_str::<Config>(&raw) {
        Ok(cfg) => cfg,
        Err(e) => {
            tracing::warn!(
                target: "jfc::config",
                path = %path.display(),
                error = %e,
                "failed to parse config â using defaults"
            );
            Config::default()
        }
    }
}

/// Load the canonical TOML config plus Claude-compatible JSON settings for a
/// specific project root. This bypasses the hot cache so tests and daemon
/// workers can resolve project-local `.claude/settings*.json` deterministically.
pub fn load_with_project(project_root: &Path) -> Config {
    let mut cfg = load_toml_from(&config_path());
    claude_settings::apply_to_config(&mut cfg, project_root);
    cfg
}

/// Load config with caching.
pub fn load() -> Config {
    (*load_arc()).clone()
}

/// Load the canonical config as a shared value.
pub fn load_arc() -> Arc<Config> {
    load_cached_arc(&config_path())
}

/// Candidate managed-settings files, from highest to lowest precedence.
pub fn managed_settings_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(path) = std::env::var("JFC_MANAGED_SETTINGS") {
        paths.push(PathBuf::from(path));
    }
    paths.push(PathBuf::from("/etc/jfc/managed-settings.toml"));
    paths.push(PathBuf::from("/etc/claude-code/managed-settings.toml"));
    if let Some(cfg) = dirs::config_dir() {
        paths.push(cfg.join("jfc").join("managed-settings.toml"));
    }
    paths
}

/// Load the first available managed-settings TOML file. Invalid files are
/// ignored with a warning so a broken policy file does not brick the CLI.
pub fn load_managed_settings() -> Option<ManagedSettingsConfig> {
    for path in managed_settings_paths() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        match toml::from_str::<ManagedSettingsConfig>(&raw) {
            Ok(settings) => return Some(settings),
            Err(e) => {
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "failed to parse managed settings - ignoring"
                );
            }
        }
    }
    load().managed_settings
}

/// Report all managed-settings sources in precedence order. This is separate
/// from [`load_managed_settings`] so policy diagnostics can explain why a
/// higher-precedence file was skipped instead of only showing the final merge.
pub fn managed_settings_sources() -> Vec<ManagedSettingsSource> {
    let mut out = Vec::new();
    for path in managed_settings_paths() {
        match std::fs::read_to_string(&path) {
            Ok(raw) => match toml::from_str::<ManagedSettingsConfig>(&raw) {
                Ok(settings) => out.push(ManagedSettingsSource {
                    label: "file".to_owned(),
                    path: Some(path),
                    exists: true,
                    loaded: true,
                    error: None,
                    settings: Some(settings),
                }),
                Err(e) => out.push(ManagedSettingsSource {
                    label: "file".to_owned(),
                    path: Some(path),
                    exists: true,
                    loaded: false,
                    error: Some(e.to_string()),
                    settings: None,
                }),
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                out.push(ManagedSettingsSource {
                    label: "file".to_owned(),
                    path: Some(path),
                    exists: false,
                    loaded: false,
                    error: None,
                    settings: None,
                });
            }
            Err(e) => out.push(ManagedSettingsSource {
                label: "file".to_owned(),
                path: Some(path),
                exists: false,
                loaded: false,
                error: Some(e.to_string()),
                settings: None,
            }),
        }
    }
    let embedded = load().managed_settings;
    out.push(ManagedSettingsSource {
        label: "config.toml [managed_settings]".to_owned(),
        path: Some(config_path()),
        exists: true,
        loaded: embedded.is_some(),
        error: None,
        settings: embedded,
    });
    out
}

/// Inner cache-and-load against an arbitrary path.
pub fn load_cached(path: &Path) -> Config {
    (*load_cached_arc(path)).clone()
}

/// Inner cache-and-load against an arbitrary path, returning a cheap shared
/// pointer on cache hits.
pub fn load_cached_arc(path: &Path) -> Arc<Config> {
    let generation = cache_generation();
    let canonical_path = config_path();
    let generation_only = path == canonical_path.as_path();
    let cur_mtime = if generation_only {
        None
    } else {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    };

    {
        let slot = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = slot.as_ref()
            && c.path == path
            && c.generation == generation
            && c.mtime == cur_mtime
        {
            return Arc::clone(&c.config);
        }
    }

    let config = Arc::new(load_from(path));
    let mut slot = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(Cached {
        path: path.to_path_buf(),
        mtime: cur_mtime,
        generation,
        config: Arc::clone(&config),
    });
    config
}

/// Persist a chosen theme name to config.toml.
pub fn save_theme(theme_name: &str) -> Result<std::path::PathBuf, String> {
    save_theme_to(&config_path(), theme_name)
}

/// Test-friendly inner helper for save_theme.
pub fn save_theme_to(
    path: &std::path::Path,
    theme_name: &str,
) -> Result<std::path::PathBuf, String> {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            target: "jfc::config",
            path = %path.display(),
            error = %e,
            "save_theme: cannot create parent dir"
        );
        return Err(format!("cannot create {}: {e}", parent.display()));
    }
    let mut cfg: Config = match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => match toml::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "save_theme: refusing to overwrite unparseable config"
                );
                return Err(format!(
                    "{} is not valid TOML â fix it first ({e})",
                    path.display()
                ));
            }
        },
        _ => Config::default(),
    };
    cfg.theme = Some(theme_name.to_string());
    let serialized = toml::to_string_pretty(&cfg).map_err(|e| format!("serialize failed: {e}"))?;
    atomic_write::write_atomic_sync(path, serialized.as_bytes())
        .map_err(|e| format!("write {} failed: {e}", path.display()))?;
    invalidate_cache();
    tracing::info!(
        target: "jfc::config",
        path = %path.display(),
        theme = %theme_name,
        "save_theme: persisted theme"
    );
    Ok(path.to_path_buf())
}

/// Persist the local/client-side advisor model. `None` disables the persisted advisor.
pub fn save_advisor_model(model: Option<&str>) -> Result<std::path::PathBuf, String> {
    save_advisor_model_to(&config_path(), model)
}

/// Test-friendly inner helper for `save_advisor_model`.
pub fn save_advisor_model_to(
    path: &std::path::Path,
    model: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            target: "jfc::config",
            path = %path.display(),
            error = %e,
            "save_advisor_model: cannot create parent dir"
        );
        return Err(format!("cannot create {}: {e}", parent.display()));
    }
    let mut cfg: Config = match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => match toml::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "save_advisor_model: refusing to overwrite unparseable config"
                );
                return Err(format!(
                    "{} is not valid TOML - fix it first ({e})",
                    path.display()
                ));
            }
        },
        _ => Config::default(),
    };
    cfg.advisor_model = model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    cfg.advisor_enabled = Some(cfg.advisor_model.is_some());
    let serialized = toml::to_string_pretty(&cfg).map_err(|e| format!("serialize failed: {e}"))?;
    atomic_write::write_atomic_sync(path, serialized.as_bytes())
        .map_err(|e| format!("write {} failed: {e}", path.display()))?;
    invalidate_cache();
    tracing::info!(
        target: "jfc::config",
        path = %path.display(),
        advisor_model = ?cfg.advisor_model,
        "save_advisor_model: persisted advisor model"
    );
    Ok(path.to_path_buf())
}

/// Persist the Anthropic server-side advisor model. This is separate from
/// `advisor_model`, which controls JFC's local/client-side Advisor tool.
pub fn save_server_advisor_model(model: Option<&str>) -> Result<std::path::PathBuf, String> {
    save_server_advisor_model_to(&config_path(), model)
}

pub fn save_server_advisor_model_to(
    path: &std::path::Path,
    model: Option<&str>,
) -> Result<std::path::PathBuf, String> {
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            target: "jfc::config",
            path = %path.display(),
            error = %e,
            "save_server_advisor_model: cannot create parent dir"
        );
        return Err(format!("cannot create {}: {e}", parent.display()));
    }
    let mut cfg: Config = match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => match toml::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "save_server_advisor_model: refusing to overwrite unparseable config"
                );
                return Err(format!(
                    "{} is not valid TOML - fix it first ({e})",
                    path.display()
                ));
            }
        },
        _ => Config::default(),
    };
    cfg.server_advisor_model = model
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned);
    let serialized = toml::to_string_pretty(&cfg).map_err(|e| format!("serialize failed: {e}"))?;
    atomic_write::write_atomic_sync(path, serialized.as_bytes())
        .map_err(|e| format!("write {} failed: {e}", path.display()))?;
    invalidate_cache();
    tracing::info!(
        target: "jfc::config",
        path = %path.display(),
        server_advisor_model = ?cfg.server_advisor_model,
        "save_server_advisor_model: persisted server advisor model"
    );
    Ok(path.to_path_buf())
}

/// Resolve a prompt value that may be a file:// URI.
pub fn resolve_prompt(value: &str, base_dir: Option<&std::path::Path>) -> String {
    if let Some(path_str) = value.strip_prefix("file://") {
        let path = if let Some(base) = base_dir {
            base.join(path_str)
        } else {
            PathBuf::from(path_str)
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => content,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "failed to read prompt file, using raw value"
                );
                value.to_owned()
            }
        }
    } else {
        value.to_owned()
    }
}

/// Persist the permission mode string to config.toml.
pub fn save_permission_mode_str(mode_str: &str) {
    save_permission_mode_to(&config_path(), mode_str);
}

fn save_permission_mode_to(path: &std::path::Path, mode: &str) {
    let mut cfg: Config = match std::fs::read_to_string(path) {
        Ok(s) if !s.trim().is_empty() => match toml::from_str(&s) {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "save_permission_mode: cannot parse config â skipping persist"
                );
                return;
            }
        },
        _ => Config::default(),
    };
    cfg.default
        .permission
        .insert("mode".to_owned(), mode.to_owned());
    if let Ok(serialized) = toml::to_string_pretty(&cfg) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).ok();
        }
        atomic_write::write_atomic_sync(path, serialized.as_bytes()).ok();
        invalidate_cache();
        tracing::info!(
            target: "jfc::config",
            mode,
            path = %path.display(),
            "permission mode persisted to config.toml"
        );
    }
}

/// Resolve which model id should be used for a given agent.
pub fn resolve_model(cfg: &Config, agent_name: Option<&str>) -> Option<String> {
    let result = if let Some(name) = agent_name {
        if let Some(agent) = cfg.agents.get(name) {
            if let Some(m) = agent.model.as_ref().filter(|s| !s.is_empty()) {
                Some(m.clone())
            } else if let Some(m) = agent.fallback_models.first() {
                Some(m.model_id().to_owned())
            } else {
                cfg.default.model.clone().filter(|s| !s.is_empty())
            }
        } else {
            cfg.default.model.clone().filter(|s| !s.is_empty())
        }
    } else {
        cfg.default.model.clone().filter(|s| !s.is_empty())
    };
    tracing::debug!(
        target: "jfc::config",
        agent_name = ?agent_name,
        resolved_model = ?result,
        "resolve_model"
    );
    result
}

/// Tools the named agent should NOT have access to.
pub fn agent_disallowed<'a>(cfg: &'a Config, agent_name: &str) -> &'a [String] {
    cfg.agents
        .get(agent_name)
        .map(|a| a.disallowed_tools.as_slice())
        .unwrap_or(&[])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> Config {
        toml::from_str::<Config>(src).expect("expected valid toml")
    }

    #[test]
    fn save_theme_to_creates_new_file_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nested").join("config.toml");
        save_theme_to(&path, "dracula").expect("write");
        assert!(path.exists(), "save_theme_to should create the file");
        let raw = std::fs::read_to_string(&path).expect("read");
        let parsed: Config = toml::from_str(&raw).expect("parse");
        assert_eq!(parsed.theme.as_deref(), Some("dracula"));
    }

    #[test]
    fn save_theme_to_preserves_other_fields_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
[default]
model = "anthropic/claude-opus-4-7"

[agents.researcher]
model = "openai/gpt-5"
"#,
        )
        .unwrap();
        save_theme_to(&path, "tokyo-night").expect("write");
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.theme.as_deref(), Some("tokyo-night"));
        assert_eq!(
            cfg.default.model.as_deref(),
            Some("anthropic/claude-opus-4-7")
        );
        assert!(cfg.agents.contains_key("researcher"));
    }

    #[test]
    fn save_theme_to_refuses_to_overwrite_broken_file_robust() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "this = is not [ valid toml").unwrap();
        let res = save_theme_to(&path, "dark");
        assert!(res.is_err(), "should refuse to overwrite invalid TOML");
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(
            raw.contains("not [ valid"),
            "original contents must be preserved"
        );
    }

    #[test]
    fn save_theme_to_treats_empty_file_as_fresh_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        save_theme_to(&path, "nord").expect("write");
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.theme.as_deref(), Some("nord"));
    }

    #[test]
    fn advisor_model_accepts_camel_case_alias_normal() {
        let cfg = parse(r#"advisorModel = "opus""#);
        assert_eq!(cfg.advisor_model.as_deref(), Some("opus"));
    }

    #[test]
    fn advisor_enabled_accepts_camel_case_alias_normal() {
        let cfg = parse(r#"advisorEnabled = false"#);
        assert_eq!(cfg.advisor_enabled, Some(false));
    }

    #[test]
    fn server_advisor_model_accepts_camel_case_alias_normal() {
        let cfg = parse(r#"serverAdvisorModel = "opus""#);
        assert_eq!(cfg.server_advisor_model.as_deref(), Some("opus"));
    }

    #[test]
    fn managed_settings_embedded_config_parses_normal() {
        let cfg = parse(
            r#"
[managed_settings]
disable_remote_control = true
disable_plugin_urls = true
force_permission_mode = "plan"
max_budget_usd = 2.5
allowed_tools = ["Read", "Grep"]
disallowed_tools = ["Bash"]
"#,
        );
        let managed = cfg.managed_settings.expect("managed settings");
        assert!(managed.disable_remote_control);
        assert!(managed.disable_plugin_urls);
        assert_eq!(managed.force_permission_mode.as_deref(), Some("plan"));
        assert_eq!(managed.max_budget_usd, Some(2.5));
        assert_eq!(managed.allowed_tools, vec!["Read", "Grep"]);
        assert_eq!(managed.disallowed_tools, vec!["Bash"]);
    }

    #[test]
    fn save_advisor_model_to_sets_and_clears_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
theme = "nord"

[default]
model = "claude-opus-4-7"
"#,
        )
        .unwrap();
        save_advisor_model_to(&path, Some("sonnet")).expect("write advisor model");
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.advisor_model.as_deref(), Some("sonnet"));
        assert_eq!(cfg.advisor_enabled, Some(true));
        assert_eq!(cfg.theme.as_deref(), Some("nord"));
        assert_eq!(cfg.default.model.as_deref(), Some("claude-opus-4-7"));

        save_advisor_model_to(&path, None).expect("clear advisor model");
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.advisor_model, None);
        assert_eq!(cfg.advisor_enabled, Some(false));
        assert_eq!(cfg.theme.as_deref(), Some("nord"));
    }

    #[test]
    fn save_server_advisor_model_to_sets_and_clears_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
advisor_model = "sonnet"
advisor_enabled = true
theme = "nord"

[default]
model = "claude-opus-4-7"
"#,
        )
        .unwrap();
        save_server_advisor_model_to(&path, Some("opus")).expect("write server advisor model");
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.server_advisor_model.as_deref(), Some("opus"));
        assert_eq!(cfg.advisor_model.as_deref(), Some("sonnet"));
        assert_eq!(cfg.advisor_enabled, Some(true));
        assert_eq!(cfg.theme.as_deref(), Some("nord"));

        save_server_advisor_model_to(&path, None).expect("clear server advisor model");
        let cfg: Config = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(cfg.server_advisor_model, None);
        assert_eq!(cfg.advisor_model.as_deref(), Some("sonnet"));
        assert_eq!(cfg.advisor_enabled, Some(true));
    }

    #[test]
    fn theme_field_roundtrips_normal() {
        let cfg = Config {
            theme: Some("monokai".into()),
            ..Config::default()
        };
        let s = toml::to_string(&cfg).expect("serialize");
        assert!(s.contains("theme"));
        let back: Config = toml::from_str(&s).expect("parse");
        assert_eq!(back.theme.as_deref(), Some("monokai"));
    }

    #[test]
    fn config_default_round_trips_robust() {
        let original = Config::default();
        let serialized = toml::to_string_pretty(&original).expect("serialize default");
        let parsed: Config = toml::from_str(&serialized).expect("parse default");
        assert_eq!(original, parsed);
    }

    #[test]
    fn parse_minimal_config_normal() {
        let cfg = parse(
            r#"
[default]
model = "x"
"#,
        );
        assert_eq!(cfg.default.model.as_deref(), Some("x"));
        assert!(cfg.agents.is_empty());
    }

    #[test]
    fn resolve_model_uses_agent_override_normal() {
        let cfg = parse(
            r#"
[default]
model = "B"

[agents.code-reviewer]
model = "A"
"#,
        );
        assert_eq!(
            resolve_model(&cfg, Some("code-reviewer")),
            Some("A".to_owned())
        );
    }

    #[test]
    fn resolve_model_falls_through_to_default_normal() {
        let cfg = parse(
            r#"
[default]
model = "B"

[agents.code-reviewer]
temperature = 0.1
"#,
        );
        assert_eq!(
            resolve_model(&cfg, Some("code-reviewer")),
            Some("B".to_owned())
        );
    }

    #[test]
    fn resolve_model_returns_none_when_nothing_configured_robust() {
        let cfg = Config::default();
        assert_eq!(resolve_model(&cfg, None), None);
        assert_eq!(resolve_model(&cfg, Some("anything")), None);
    }

    #[test]
    fn agent_disallowed_returns_list_normal() {
        let cfg = parse(
            r#"
[agents.code-reviewer]
disallowed_tools = ["Bash", "Write"]
"#,
        );
        assert_eq!(
            agent_disallowed(&cfg, "code-reviewer"),
            &["Bash".to_owned(), "Write".to_owned()]
        );
    }

    #[test]
    fn agent_disallowed_unknown_agent_returns_empty_robust() {
        let cfg = Config::default();
        assert!(agent_disallowed(&cfg, "ghost").is_empty());
    }

    #[test]
    fn resolve_prompt_file_uri_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let prompt_file = tmp.path().join("system.md");
        std::fs::write(&prompt_file, "You are a helpful assistant.").unwrap();
        let resolved = resolve_prompt("file://system.md", Some(tmp.path()));
        assert_eq!(resolved, "You are a helpful assistant.");
    }

    #[test]
    fn resolve_prompt_plain_string_normal() {
        let resolved = resolve_prompt("Just a plain prompt", None);
        assert_eq!(resolved, "Just a plain prompt");
    }

    #[test]
    fn parse_malformed_toml_returns_default_robust() {
        let bad = "this is = = not toml [ [ [";
        let result = toml::from_str::<Config>(bad);
        assert!(result.is_err(), "garbage toml must not parse");
    }

    #[test]
    #[serial_test::serial]
    fn load_cached_reads_file_once_for_repeated_calls_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, b"[default]\nmodel = \"anthropic/claude-opus-4-7\"\n").unwrap();

        invalidate_cache();
        let before = read_count();

        let first = load_cached(&path);
        let second = load_cached(&path);
        let third = load_cached(&path);

        assert_eq!(
            first.default.model.as_deref(),
            Some("anthropic/claude-opus-4-7")
        );
        assert_eq!(first, second);
        assert_eq!(second, third);
        assert_eq!(read_count() - before, 1);
    }

    #[test]
    #[serial_test::serial]
    fn load_cached_reparses_when_mtime_changes_robust() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, b"[default]\nmodel = \"a/one\"\n").unwrap();

        invalidate_cache();
        let before = read_count();

        let first = load_cached(&path);
        assert_eq!(first.default.model.as_deref(), Some("a/one"));
        assert_eq!(read_count() - before, 1);

        std::fs::write(&path, b"[default]\nmodel = \"b/two\"\n").unwrap();
        let future = std::time::SystemTime::now() + std::time::Duration::from_secs(10);
        let f = std::fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("reopen");
        let times = std::fs::FileTimes::new().set_modified(future);
        f.set_times(times).expect("bump mtime");
        drop(f);

        let second = load_cached(&path);
        assert_eq!(second.default.model.as_deref(), Some("b/two"));
        assert_eq!(read_count() - before, 2);
    }

    #[test]
    #[serial_test::serial]
    fn invalidate_cache_forces_reread_robust() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, b"[default]\nmodel = \"a/one\"\n").unwrap();

        invalidate_cache();
        let before = read_count();

        let first = load_cached(&path);
        assert_eq!(first.default.model.as_deref(), Some("a/one"));
        assert_eq!(read_count() - before, 1);

        invalidate_cache();
        let second = load_cached(&path);
        assert_eq!(second.default.model.as_deref(), Some("a/one"));
        assert_eq!(read_count() - before, 2);
    }

    // Robust: an `[isolation]` table that omits `fail_closed` still defaults
    // to fail-closed (true) — no silent flip to the permissive cwd fallback.
    #[test]
    fn isolation_table_defaults_fail_closed_robust() {
        let cfg: Config = toml::from_str("[isolation]\n").expect("parse");
        assert!(
            cfg.isolation.expect("isolation present").fail_closed,
            "omitted key must default to fail-closed"
        );
    }

    // Robust: opting out is explicit and round-trips.
    #[test]
    fn isolation_fail_open_opt_out_robust() {
        let cfg: Config = toml::from_str("[isolation]\nfail_closed = false\n").expect("parse");
        assert!(!cfg.isolation.expect("present").fail_closed);
    }
}
