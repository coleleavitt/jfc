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

mod trace;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

pub use claude_settings::ClaudeCompatibilityConfig;
/// Re-export from jfc-mcp so existing callsites keep working.
pub use jfc_mcp::McpServerConfig;

pub const REDACT_THINKING_BETA: &str = "redact-thinking-2026-02-12";

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
    #[serde(default, alias = "promptRewrite")]
    pub prompt_rewrite: Option<PromptRewriteConfig>,
    #[serde(default, alias = "pairEval", alias = "pair_eval")]
    pub pair: Option<PairEvalConfig>,
    #[serde(default, alias = "redTeam", alias = "red_team")]
    pub redteam: Option<RedTeamEvalConfig>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub council: Option<CouncilConfig>,
    #[serde(default)]
    pub slate_enabled: bool,
    #[serde(default)]
    pub slate_rules: Option<Vec<SlateRuleConfig>>,
    #[serde(default = "default_memory_recall_enabled")]
    pub memory_recall_enabled: bool,
    #[serde(default = "default_plan_recall_enabled")]
    pub plan_recall_enabled: bool,
    /// Cross-project knowledge recall (jfc-knowledge). Default OFF: the store
    /// may still self-drive imports, mining, and evidence-gated promotion, but
    /// prompt injection from other projects is an explicit config choice.
    #[serde(default = "default_cross_project_recall_enabled")]
    pub cross_project_recall_enabled: bool,
    #[serde(default)]
    pub session_cost_budget_usd: Option<f64>,
    #[serde(default = "default_auto_compact_enabled")]
    pub auto_compact_enabled: bool,
    /// Ask Anthropic to compact old turns SERVER-side (the `compact_20260112`
    /// context-management edit) before content reaches the model. This is the
    /// non-blocking primary compaction path: it runs API-side with no
    /// client-side concurrency and never stalls the user's input. Defaults on;
    /// only affects the Anthropic provider when `auto_compact_enabled` is also
    /// true. Set false to fall back to client-side compaction only.
    #[serde(default = "default_true")]
    pub server_side_compaction_enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_compact_window: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compact_instructions: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hooks: Option<ShellHooksConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remote_control: Option<RemoteControlConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dashboard: Option<DashboardConfig>,
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub voice: Option<VoiceSettingsConfig>,
    /// Safe mode disables runtime customization that can mutate local state or
    /// fetch code: plugin installs/updates/runtime registration and theme
    /// persistence/preview. Also surfaced in the TUI footer.
    #[serde(default, alias = "safeMode")]
    pub safe_mode: bool,
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
    /// On a provider refusal, run the prompt back through the local over-refusal
    /// rewrite gate and, if it produces a scope-bounded clarification (policy
    /// gate + verifier still gate it), resend the rewritten prompt. Bounded by
    /// `refusal_rewrite_retry_max`. Default on; a genuinely-disallowed prompt is
    /// `Refused` by the gate and never resent.
    #[serde(default = "default_true", alias = "refusalRewriteRetryEnabled")]
    pub refusal_rewrite_retry_enabled: bool,
    /// Max rewrite-and-resend rounds per turn for the loop above. `None` ⇒ a small
    /// default (3); hard-clamped to 20 in the accessor regardless of value, since
    /// each round is a full extra request and a real refusal won't clear after a
    /// few tries.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "refusalRewriteRetryMax"
    )]
    pub refusal_rewrite_retry_max: Option<u32>,
    /// When the model refuses (or a turn is reclassified as a refusal), write the
    /// model's chain-of-thought / thinking to ephemeral tracing debug logs so the
    /// refusal and the rewrite "chain of adaptation" can be inspected locally.
    /// Default OFF: with it off, private reasoning is never logged and the durable
    /// refusal diagnostic keeps only counts. This is a local-debug switch — the CoT
    /// is logged, never persisted to the durable knowledge store.
    #[serde(default, alias = "refusalLogReasoning")]
    pub refusal_log_reasoning: bool,
    /// Queue user messages during active streaming turns and disable explicit
    /// Alt+Enter steering. Bare Enter always queues behind active work; when
    /// this is `false`, Alt+Enter may still interrupt a safe in-flight stream.
    /// The `/queue` command shows pending queued messages and `/queue clear`
    /// discards them. Default `false`.
    #[serde(default)]
    pub message_queue_mode: bool,
    /// Seed subagent contexts with the parent's CLAUDE.md summary when spawning
    /// via the Task tool. When `true`, the spawn path attaches a compact
    /// context block as `forksParentContext` in the subagent's system prompt
    /// so it can skip redundant codebase re-scans. Default on.
    #[serde(default = "default_true")]
    pub subagent_context_inheritance: bool,
    /// Show the startup welcome/nudge banner when jfc opens (default: true).
    /// Set `show_startup_banner = false` to suppress the one-time nudge
    /// message shown at launch. Has no effect after the nudge marker file
    /// is created (i.e. after the first run).
    #[serde(default = "default_true")]
    pub show_startup_banner: bool,
    /// Shell to use for the Bash tool. Defaults to `bash`. Example: `/bin/zsh`
    /// or `fish`. The shell is invoked as `<shell> -c <command>` so it must
    /// support that interface.
    #[serde(default, alias = "bashShell", skip_serializing_if = "Option::is_none")]
    pub bash_shell: Option<String>,
    /// Path to a base config file to inherit settings from. Keys in the local
    /// config override inherited ones; unset local keys fall back to the base.
    /// Resolved relative to the directory containing the local config file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<std::path::PathBuf>,
    /// Compact when context reaches this percentage of the context window
    /// (0–100). Default 85. Ignored when auto-compact is disabled.
    #[serde(default = "default_auto_compact_threshold_pct")]
    pub auto_compact_threshold_pct: u8,
    /// Always expand and show model thinking blocks (default: false, collapsed).
    /// When true, every completed thinking block is rendered expanded instead
    /// of collapsing to a one-line teaser.
    #[serde(default)]
    pub always_show_thinking: bool,
    #[serde(
        default,
        alias = "redactedThinkingEnabled",
        skip_serializing_if = "Option::is_none"
    )]
    pub redacted_thinking_enabled: Option<bool>,
    /// Emit OSC 8 hyperlinks for file paths in tool-block headers (Edit/Write/Read).
    /// Terminals that support OSC 8 (iTerm2, kitty, WezTerm, Windows Terminal,
    /// recent gnome-terminal) render the path as a clickable `file://` link.
    /// Terminals that do not support OSC 8 ignore the escape sequences and
    /// display plain text. Default `true`; set to `false` if your terminal
    /// renders the raw escape codes visibly.
    #[serde(default = "default_true")]
    pub osc8_hyperlinks: bool,
    /// When `false`, pressing bare Enter inserts a literal newline into the
    /// input box and Ctrl+Enter submits the message. Default `true` (Enter
    /// submits, Ctrl+Enter inserts newline).
    #[serde(default = "default_true")]
    pub enter_sends_message: bool,

    // ── Session GC ───────────────────────────────────────────────────────
    /// Sessions older than this many days are GC'd at startup (0 = disabled).
    /// Default 30.
    #[serde(default = "default_session_max_age_days")]
    pub session_max_age_days: u64,

    /// Minimum number of sessions to keep regardless of age. Default 20.
    #[serde(default = "default_session_min_keep")]
    pub session_min_keep: usize,

    // ── Memory after compact ─────────────────────────────────────────────
    /// Re-consult memory recall after context compaction so the fresh
    /// context window starts with relevant memories. Default true.
    #[serde(default = "default_true")]
    pub consult_memory_after_compact: bool,

    // ── Cross-session up-arrow history ───────────────────────────────────
    /// Include prompts from previous sessions in up-arrow history.
    /// Default true.
    #[serde(default = "default_true")]
    pub cross_session_history: bool,

    /// Accessibility: when true, the TUI reduces ambiguous glyphs and renders
    /// more linear, text-first labels in key areas (status row, spinner, thinking
    /// blocks). Aliased for Claude-compatible casing.
    #[serde(default, alias = "screenReaderMode")]
    pub screen_reader_mode: bool,

    /// Large text paste handling: when true, collapse large pastes in the input
    /// box to a compact `[Pasted #N · …]` chip. When false (default), insert the
    /// full pasted text as normal editable input and show a transient toast.
    #[serde(default, alias = "collapseLargePastes")]
    pub collapse_large_pastes: bool,
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
    /// Optional default isolation mode for Task/workflow subagents when neither
    /// the tool call nor the agent definition set one. Set to `"worktree"` to
    /// make subagents use isolated worktrees by default.
    #[serde(
        default,
        rename = "defaultTaskIsolation",
        alias = "default_task_isolation",
        skip_serializing_if = "Option::is_none"
    )]
    pub default_task_isolation: Option<String>,
}

impl Default for IsolationConfig {
    fn default() -> Self {
        Self {
            fail_closed: true,
            default_task_isolation: None,
        }
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
    /// ("Want me to …?") or leaves queued tasks unfinished. On by default;
    /// set `JFC_AUTO_CONTINUE=0` to hard-disable it for a run.
    pub auto_continue: bool,
    /// Maximum consecutive self-continuations before stopping for the user.
    pub max_self_continuations: u32,
}

impl Default for ContinuationConfig {
    fn default() -> Self {
        Self {
            auto_continue: true,
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

/// `[dashboard]` section in config.toml — opt-in local token-audit dashboard.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct DashboardConfig {
    /// Start the token-audit web dashboard when jfc launches.
    pub enabled: bool,
    /// TCP port for the dashboard HTTP server (default 4327).
    pub port: u16,
}

/// Default token-audit dashboard HTTP port.
pub const DEFAULT_DASHBOARD_PORT: u16 = 4327;

impl Default for DashboardConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            port: DEFAULT_DASHBOARD_PORT,
        }
    }
}

fn default_memory_recall_enabled() -> bool {
    true
}

fn default_cross_project_recall_enabled() -> bool {
    false
}
fn default_plan_recall_enabled() -> bool {
    true
}
fn default_auto_compact_enabled() -> bool {
    true
}
fn default_auto_compact_threshold_pct() -> u8 {
    85
}
fn default_true() -> bool {
    true
}
fn default_session_max_age_days() -> u64 {
    30
}
fn default_session_min_keep() -> usize {
    20
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CouncilMode {
    #[default]
    Direct,
    Agentic,
}

fn default_council_member_timeout_ms() -> u64 {
    120_000
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CouncilConfig {
    /// Optional named roster. Each member model can be provider-qualified
    /// (`anthropic/claude-sonnet-4.5`) or a bare model id resolved like
    /// AskModel/Council explicit members.
    pub members: Vec<CouncilMemberConfig>,
    /// Minimum successful member answers required before synthesis. Defaults
    /// to one for backwards compatibility with the pre-roster council.
    pub quorum: Option<usize>,
    /// Retry a failed/timed-out member this many additional times.
    pub retry_on_fail: u32,
    /// Per-member timeout in milliseconds. Set to 0 to disable.
    pub member_timeout_ms: u64,
    /// Direct mode calls each member model once without tools. Agentic mode
    /// runs read-only task-backed council members with StructuredOutput.
    pub mode: CouncilMode,
    /// Save prompt, member outputs, synthesis, and metadata under
    /// `.jfc/council/<run-id>/`.
    pub archive: bool,
    /// Default intent used when a Council call does not provide one.
    pub intent: Option<String>,
    /// Persistent-session defaults (RoundTable `/council start`).
    pub session: CouncilSessionConfig,
}

impl Default for CouncilConfig {
    fn default() -> Self {
        Self {
            members: Vec::new(),
            quorum: None,
            retry_on_fail: 0,
            member_timeout_ms: default_council_member_timeout_ms(),
            mode: CouncilMode::Direct,
            archive: false,
            intent: None,
            session: CouncilSessionConfig::default(),
        }
    }
}

/// Defaults applied when opening a persistent council session via
/// `/council start` (RoundTable-style turn-based deliberation). Distinct from
/// the one-shot fan-out knobs above.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CouncilSessionConfig {
    /// Default deliberation style: `debate` | `collaborate` | `blind-reveal` |
    /// `blind-map-reduce`. Used when `/council start` omits a mode token.
    pub mode: String,
    /// Suggested rounds before the session nudges toward a verdict.
    pub max_rounds: u32,
    /// Per-seat sealed-aside allowance (model↔model DMs a seat may open).
    pub aside_allowance: u32,
    /// Allow seats to open sealed asides (when false, only the operator can).
    pub side_conversations: bool,
    /// Per-aside message cap before it auto-closes.
    pub side_convo_max_len: usize,
    /// Allow seats to flag claims that soft-block the verdict.
    pub flagged_claims: bool,
    /// Allow seat-initiated kick / operator-mute votes.
    pub governance_votes: bool,
}

impl Default for CouncilSessionConfig {
    fn default() -> Self {
        Self {
            mode: "debate".to_owned(),
            max_rounds: 4,
            aside_allowance: 1,
            side_conversations: false,
            side_convo_max_len: 6,
            flagged_claims: true,
            governance_votes: true,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct CouncilMemberConfig {
    pub name: Option<String>,
    pub model: String,
    /// Reserved for providers that expose model variants/effort through config.
    pub variant: Option<String>,
    pub effort: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct VoiceSettingsConfig {
    pub enabled: Option<bool>,
    pub mode: Option<String>,
    #[serde(alias = "vadEngine")]
    pub vad_engine: Option<String>,
    #[serde(alias = "autoSubmit")]
    pub auto_submit: Option<bool>,
    pub language: Option<String>,
    pub backend: Option<String>,
    #[serde(alias = "anthropicVoiceUrl", alias = "anthropic_voice_url")]
    pub anthropic_voice_url: Option<String>,
    #[serde(alias = "openaiApiKey")]
    pub openai_api_key: Option<String>,
    #[serde(alias = "localWhisperBin")]
    pub local_whisper_bin: Option<String>,
    #[serde(alias = "localWhisperModel")]
    pub local_whisper_model: Option<String>,
    #[serde(alias = "speakerGate")]
    pub speaker_gate: Option<bool>,
    #[serde(alias = "speakerProfile")]
    pub speaker_profile: Option<String>,
    #[serde(alias = "speakerThreshold")]
    pub speaker_threshold: Option<f64>,
    #[serde(alias = "speakerModel")]
    pub speaker_model: Option<String>,
    #[serde(alias = "readAloud", alias = "readAssistant")]
    pub read_aloud: Option<bool>,
    #[serde(alias = "echoSuppression")]
    pub echo_suppression: Option<bool>,
    #[serde(alias = "ttsVoice")]
    pub tts_voice: Option<String>,
    #[serde(alias = "ttsSpeed")]
    pub tts_speed: Option<f32>,
    #[serde(alias = "ttsOutputFormat")]
    pub tts_output_format: Option<String>,
    #[serde(alias = "ttsBaseUrl")]
    pub tts_base_url: Option<String>,
    #[serde(alias = "ttsPlaybackCommand")]
    pub tts_playback_command: Option<String>,
    #[serde(alias = "selectedSpeakerDeviceId", alias = "speakerDeviceId")]
    pub selected_speaker_device_id: Option<String>,
    #[serde(alias = "conversationEnabled", alias = "fullDuplex")]
    pub conversation_enabled: Option<bool>,
    #[serde(alias = "conversationBaseUrl")]
    pub conversation_base_url: Option<String>,
    #[serde(
        alias = "organizationUuid",
        alias = "organizationUUID",
        alias = "orgUuid"
    )]
    pub organization_uuid: Option<String>,
    #[serde(alias = "conversationUuid", alias = "conversationUUID")]
    pub conversation_uuid: Option<String>,
    #[serde(alias = "conversationInputEncoding")]
    pub conversation_input_encoding: Option<String>,
    #[serde(alias = "conversationOutputFormat")]
    pub conversation_output_format: Option<String>,
    pub timezone: Option<String>,
    #[serde(alias = "conversationModel")]
    pub conversation_model: Option<String>,
    #[serde(alias = "conversationEffort")]
    pub conversation_effort: Option<String>,
    #[serde(alias = "conversationThinkingMode")]
    pub conversation_thinking_mode: Option<String>,
    #[serde(alias = "forwardInterims")]
    pub forward_interims: Option<bool>,
    #[serde(alias = "allowCustomAuthEndpoint")]
    pub allow_custom_auth_endpoint: Option<bool>,
    #[serde(alias = "allowInsecureAuthEndpoint")]
    pub allow_insecure_auth_endpoint: Option<bool>,
}

impl VoiceSettingsConfig {
    pub fn to_compat_json(&self) -> serde_json::Value {
        let mut value = serde_json::Map::new();
        insert_opt(&mut value, "enabled", self.enabled);
        insert_opt_ref(&mut value, "mode", self.mode.as_ref());
        insert_opt_ref(&mut value, "vadEngine", self.vad_engine.as_ref());
        insert_opt(&mut value, "autoSubmit", self.auto_submit);
        insert_opt_ref(&mut value, "language", self.language.as_ref());
        insert_opt_ref(&mut value, "backend", self.backend.as_ref());
        insert_opt_ref(
            &mut value,
            "anthropicVoiceUrl",
            self.anthropic_voice_url.as_ref(),
        );
        insert_opt_ref(&mut value, "openaiApiKey", self.openai_api_key.as_ref());
        insert_opt_ref(
            &mut value,
            "localWhisperBin",
            self.local_whisper_bin.as_ref(),
        );
        insert_opt_ref(
            &mut value,
            "localWhisperModel",
            self.local_whisper_model.as_ref(),
        );
        insert_opt(&mut value, "speakerGate", self.speaker_gate);
        insert_opt_ref(&mut value, "speakerProfile", self.speaker_profile.as_ref());
        insert_opt(&mut value, "speakerThreshold", self.speaker_threshold);
        insert_opt_ref(&mut value, "speakerModel", self.speaker_model.as_ref());
        insert_opt(&mut value, "readAloud", self.read_aloud);
        insert_opt(&mut value, "echoSuppression", self.echo_suppression);
        insert_opt_ref(&mut value, "ttsVoice", self.tts_voice.as_ref());
        insert_opt(&mut value, "ttsSpeed", self.tts_speed);
        insert_opt_ref(
            &mut value,
            "ttsOutputFormat",
            self.tts_output_format.as_ref(),
        );
        insert_opt_ref(&mut value, "ttsBaseUrl", self.tts_base_url.as_ref());
        insert_opt_ref(
            &mut value,
            "ttsPlaybackCommand",
            self.tts_playback_command.as_ref(),
        );
        insert_opt_ref(
            &mut value,
            "selectedSpeakerDeviceId",
            self.selected_speaker_device_id.as_ref(),
        );
        insert_opt(&mut value, "conversationEnabled", self.conversation_enabled);
        insert_opt_ref(
            &mut value,
            "conversationBaseUrl",
            self.conversation_base_url.as_ref(),
        );
        insert_opt_ref(
            &mut value,
            "organizationUuid",
            self.organization_uuid.as_ref(),
        );
        insert_opt_ref(
            &mut value,
            "conversationUuid",
            self.conversation_uuid.as_ref(),
        );
        insert_opt_ref(
            &mut value,
            "conversationInputEncoding",
            self.conversation_input_encoding.as_ref(),
        );
        insert_opt_ref(
            &mut value,
            "conversationOutputFormat",
            self.conversation_output_format.as_ref(),
        );
        insert_opt_ref(&mut value, "timezone", self.timezone.as_ref());
        insert_opt_ref(
            &mut value,
            "conversationModel",
            self.conversation_model.as_ref(),
        );
        insert_opt_ref(
            &mut value,
            "conversationEffort",
            self.conversation_effort.as_ref(),
        );
        insert_opt_ref(
            &mut value,
            "conversationThinkingMode",
            self.conversation_thinking_mode.as_ref(),
        );
        insert_opt(&mut value, "forwardInterims", self.forward_interims);
        insert_opt(
            &mut value,
            "allowCustomAuthEndpoint",
            self.allow_custom_auth_endpoint,
        );
        insert_opt(
            &mut value,
            "allowInsecureAuthEndpoint",
            self.allow_insecure_auth_endpoint,
        );
        serde_json::Value::Object(value)
    }
}

fn insert_opt<T>(map: &mut serde_json::Map<String, serde_json::Value>, key: &str, value: Option<T>)
where
    T: Serialize,
{
    if let Some(value) = value
        && let Ok(value) = serde_json::to_value(value)
    {
        map.insert(key.to_owned(), value);
    }
}

fn insert_opt_ref<T>(
    map: &mut serde_json::Map<String, serde_json::Value>,
    key: &str,
    value: Option<&T>,
) where
    T: Serialize,
{
    if let Some(value) = value
        && let Ok(value) = serde_json::to_value(value)
    {
        map.insert(key.to_owned(), value);
    }
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
            prompt_rewrite: None,
            pair: None,
            redteam: None,
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
            council: None,
            slate_enabled: false,
            slate_rules: None,
            memory_recall_enabled: default_memory_recall_enabled(),
            plan_recall_enabled: default_plan_recall_enabled(),
            cross_project_recall_enabled: default_cross_project_recall_enabled(),
            session_cost_budget_usd: None,
            auto_compact_enabled: default_auto_compact_enabled(),
            server_side_compaction_enabled: default_true(),
            auto_compact_window: None,
            compact_instructions: None,
            hooks: None,
            remote_control: None,
            dashboard: None,
            continuation: None,
            exploration: None,
            managed_settings: None,
            isolation: None,
            worktree: None,
            sandbox: None,
            default_shell: None,
            claude: ClaudeCompatibilityConfig::default(),
            voice: None,
            safe_mode: false,
            copy_on_select: default_true(),
            refusal_fallback_enabled: default_true(),
            refusal_fallback_model: None,
            refusal_rewrite_retry_enabled: true,
            refusal_rewrite_retry_max: None,
            refusal_log_reasoning: false,
            message_queue_mode: false,
            subagent_context_inheritance: true,
            show_startup_banner: default_true(),
            bash_shell: None,
            extends: None,
            auto_compact_threshold_pct: default_auto_compact_threshold_pct(),
            always_show_thinking: false,
            redacted_thinking_enabled: None,
            osc8_hyperlinks: default_true(),
            enter_sends_message: default_true(),
            session_max_age_days: default_session_max_age_days(),
            session_min_keep: default_session_min_keep(),
            consult_memory_after_compact: default_true(),
            cross_session_history: default_true(),
            screen_reader_mode: false,
            collapse_large_pastes: false,
        }
    }
}

impl Config {
    pub fn redacted_thinking_enabled(&self) -> bool {
        self.redacted_thinking_enabled.unwrap_or(false)
    }

    pub fn anthropic_betas<I, S>(&self, extra_betas: I) -> Vec<String>
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        let mut betas = Vec::new();
        if self.redacted_thinking_enabled() {
            push_beta_once(&mut betas, REDACT_THINKING_BETA.to_owned());
        }
        for beta in extra_betas {
            push_beta_once(&mut betas, beta.into());
        }
        betas
    }

    /// Merge `other` (local override) on top of `self` (base). For every
    /// `Option<T>` field, the local value wins when it is `Some`; otherwise
    /// the base value is used. For plain-bool/scalar fields the local wins
    /// unconditionally. Collections (Vec, HashMap) are replaced by the local
    /// copy when non-empty; the base copy is kept when the local is empty.
    pub fn merge_with(self, other: Self) -> Self {
        macro_rules! local_or_base {
            ($field:ident) => {
                other.$field.or(self.$field)
            };
        }
        macro_rules! local_wins {
            ($field:ident) => {
                other.$field
            };
        }
        macro_rules! nonempty_or_base {
            ($field:ident) => {
                if other.$field.is_empty() {
                    self.$field
                } else {
                    other.$field
                }
            };
        }

        Self {
            // Scalar / plain-bool: local wins.
            slate_enabled: local_wins!(slate_enabled),
            memory_recall_enabled: local_wins!(memory_recall_enabled),
            plan_recall_enabled: local_wins!(plan_recall_enabled),
            cross_project_recall_enabled: local_wins!(cross_project_recall_enabled),
            auto_compact_enabled: local_wins!(auto_compact_enabled),
            server_side_compaction_enabled: local_wins!(server_side_compaction_enabled),
            auto_compact_threshold_pct: local_wins!(auto_compact_threshold_pct),
            always_show_thinking: local_wins!(always_show_thinking),
            osc8_hyperlinks: local_wins!(osc8_hyperlinks),
            enter_sends_message: local_wins!(enter_sends_message),
            session_max_age_days: local_wins!(session_max_age_days),
            session_min_keep: local_wins!(session_min_keep),
            consult_memory_after_compact: local_wins!(consult_memory_after_compact),
            cross_session_history: local_wins!(cross_session_history),
            copy_on_select: local_wins!(copy_on_select),
            safe_mode: local_wins!(safe_mode),
            screen_reader_mode: local_wins!(screen_reader_mode),
            collapse_large_pastes: local_wins!(collapse_large_pastes),
            refusal_fallback_enabled: local_wins!(refusal_fallback_enabled),
            refusal_rewrite_retry_enabled: local_wins!(refusal_rewrite_retry_enabled),
            refusal_log_reasoning: local_wins!(refusal_log_reasoning),
            message_queue_mode: local_wins!(message_queue_mode),
            subagent_context_inheritance: local_wins!(subagent_context_inheritance),
            show_startup_banner: local_wins!(show_startup_banner),
            // Option<T>: local wins when Some, else fall through to base.
            permission_automation: local_or_base!(permission_automation),
            background_task: local_or_base!(background_task),
            argus_auto_review: local_or_base!(argus_auto_review),
            prompt_rewrite: local_or_base!(prompt_rewrite),
            pair: local_or_base!(pair),
            redteam: local_or_base!(redteam),
            experimental: local_or_base!(experimental),
            theme: local_or_base!(theme),
            output_style: local_or_base!(output_style),
            advisor_model: local_or_base!(advisor_model),
            advisor_enabled: local_or_base!(advisor_enabled),
            server_advisor_model: local_or_base!(server_advisor_model),
            council_verdict: local_or_base!(council_verdict),
            council: local_or_base!(council),
            slate_rules: local_or_base!(slate_rules),
            session_cost_budget_usd: local_or_base!(session_cost_budget_usd),
            auto_compact_window: local_or_base!(auto_compact_window),
            compact_instructions: local_or_base!(compact_instructions),
            hooks: local_or_base!(hooks),
            remote_control: local_or_base!(remote_control),
            dashboard: local_or_base!(dashboard),
            continuation: local_or_base!(continuation),
            exploration: local_or_base!(exploration),
            managed_settings: local_or_base!(managed_settings),
            isolation: local_or_base!(isolation),
            worktree: local_or_base!(worktree),
            sandbox: local_or_base!(sandbox),
            default_shell: local_or_base!(default_shell),
            voice: local_or_base!(voice),
            refusal_fallback_model: local_or_base!(refusal_fallback_model),
            refusal_rewrite_retry_max: local_or_base!(refusal_rewrite_retry_max),
            bash_shell: local_or_base!(bash_shell),
            redacted_thinking_enabled: local_or_base!(redacted_thinking_enabled),
            // `extends` from the local file is already resolved; don't
            // propagate the base's extends path into the merged result.
            extends: other.extends,
            // Sub-struct: local AgentConfig wins entirely.
            default: other.default,
            // Maps / Vecs: non-empty local wins; otherwise inherit base.
            agents: nonempty_or_base!(agents),
            categories: nonempty_or_base!(categories),
            mcp: nonempty_or_base!(mcp),
            disabled_agents: nonempty_or_base!(disabled_agents),
            disabled_tools: nonempty_or_base!(disabled_tools),
            claude: other.claude,
        }
    }
}

fn push_beta_once(betas: &mut Vec<String>, beta: String) {
    let beta = beta.trim();
    if beta.is_empty() || betas.iter().any(|existing| existing == beta) {
        return;
    }
    betas.push(beta.to_owned());
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
    /// Fires before context compaction begins.
    #[serde(rename = "PreCompact", default)]
    pub pre_compact: Vec<ShellHookEntry>,
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
    /// Fires when the user interrupts a running turn (Ctrl-C / Esc-Esc).
    #[serde(rename = "UserInterrupt", default)]
    pub user_interrupt: Vec<ShellHookEntry>,
    /// Fires on each streamed model-response text chunk.
    ///
    /// **High-frequency** — only register cheap, non-blocking handlers.
    #[serde(rename = "ModelResponseChunk", default)]
    pub model_response_chunk: Vec<ShellHookEntry>,
    /// Fires when the engine is about to block on interactive user input.
    #[serde(rename = "UserInputRequired", default)]
    pub user_input_required: Vec<ShellHookEntry>,
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

/// Argus auto-review configuration (`[argus_auto_review]` in config.toml).
///
/// All fields are optional so a user can set just one knob. The engine resolves
/// the effective on/off state with this precedence (see
/// `jfc_engine::auto_review::auto_review_mode`): the `JFC_AUTO_REVIEW` env var
/// wins, then `mode`, then `enabled`, then the built-in default (Smart).
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct ArgusAutoReviewConfig {
    /// Master on/off switch. `Some(false)` turns auto-review off durably (the
    /// opt-out for users who find it slow/noisy); `Some(true)` or omitted keeps
    /// the default. Distinct from `None` (omitted) so setting only `model` does
    /// not silently disable the feature.
    #[serde(default)]
    pub enabled: Option<bool>,
    /// Explicit mode override: `off` | `manual` | `smart` | `always`. Takes
    /// precedence over `enabled` when set, so it is the precise knob.
    #[serde(default)]
    pub mode: Option<String>,
    #[serde(default)]
    pub threshold: Option<u32>,
    #[serde(default)]
    pub model: Option<String>,
}

/// Local prompt-rewriter / over-refusal-mitigation configuration.
///
/// Default on for response-side refusal recovery. Set `enabled = false` for an
/// explicit no-op pass-through. `constitution` overrides the built-in policy text.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PromptRewriteConfig {
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Model used by the LLM-backed stages. Falls back to `advisor_model`.
    #[serde(default)]
    pub model: Option<String>,
    /// Intent-preservation acceptance threshold τ in [0, 1].
    #[serde(default)]
    pub threshold: Option<f64>,
    /// Inline natural-language constitution; overrides the built-in default.
    #[serde(default)]
    pub constitution: Option<String>,
}

impl Default for PromptRewriteConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            threshold: None,
            constitution: None,
        }
    }
}

/// Controlled PAIR red-team evaluation configuration.
///
/// Default-OFF: callers should require either `enabled = true` here or an
/// explicit per-run opt-in flag before making provider calls.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct PairEvalConfig {
    pub enabled: bool,
    pub attacker_model: Option<String>,
    pub target_model: Option<String>,
    pub judge_model: Option<String>,
    pub attacker_provider: Option<String>,
    pub target_provider: Option<String>,
    pub judge_provider: Option<String>,
    pub judge: Option<String>,
    pub n_streams: Option<usize>,
    pub n_iterations: Option<usize>,
    pub keep_last_n: Option<usize>,
    pub max_attack_attempts: Option<usize>,
    pub success_threshold: Option<f64>,
    pub parallel_streams: Option<bool>,
    pub attack_max_tokens: Option<u32>,
    pub target_max_tokens: Option<u32>,
    pub judge_max_tokens: Option<u32>,
    pub attack_temperature: Option<f64>,
    pub target_temperature: Option<f64>,
    pub judge_temperature: Option<f64>,
    pub attack_top_p: Option<f64>,
    pub target_top_p: Option<f64>,
    pub judge_top_p: Option<f64>,
}

/// `[redteam]` configuration for post-PAIR controlled red-team methods.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct RedTeamEvalConfig {
    pub enabled: bool,
    pub method: Option<String>,
    pub attacker_model: Option<String>,
    pub target_model: Option<String>,
    pub judge_model: Option<String>,
    pub attacker_provider: Option<String>,
    pub target_provider: Option<String>,
    pub judge_provider: Option<String>,
    pub judge: Option<String>,
    pub n_streams: Option<usize>,
    pub n_iterations: Option<usize>,
    pub branch_factor: Option<usize>,
    pub prune_width: Option<usize>,
    pub population_size: Option<usize>,
    pub generations: Option<usize>,
    pub max_turns: Option<usize>,
    pub success_threshold: Option<f64>,
    pub proact_defense: Option<bool>,
    pub robot_context: Option<String>,
    pub beta0: Option<f64>,
    pub casp_drift: Option<f64>,
    pub embedding_dim: Option<usize>,
    pub bo_candidates: Option<usize>,
    pub jrl_learning_rate: Option<f64>,
    pub jrl_gamma: Option<f64>,
    pub sinkhorn_epsilon: Option<f64>,
    pub sinkhorn_iterations: Option<usize>,
    pub control_grid: Option<usize>,
    pub control_cost: Option<f64>,
    pub attack_max_tokens: Option<u32>,
    pub target_max_tokens: Option<u32>,
    pub judge_max_tokens: Option<u32>,
    pub attack_temperature: Option<f64>,
    pub target_temperature: Option<f64>,
    pub judge_temperature: Option<f64>,
    pub attack_top_p: Option<f64>,
    pub target_top_p: Option<f64>,
    pub judge_top_p: Option<f64>,
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
            let config_dir = dirs::config_dir().unwrap_or_else(|| PathBuf::from("."));
            let path = resolve_config_path(&config_dir);
            tracing::trace!(target: "jfc::config", path = %path.display(), "resolved config path");
            path
        })
        .clone()
}

fn resolve_config_path(config_dir: &Path) -> PathBuf {
    let canonical = config_dir.join("jfc").join("config.toml");
    let legacy_alias = config_dir.join("kfc").join("config.toml");
    if legacy_alias.exists() {
        legacy_alias
    } else {
        canonical
    }
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
static SAFE_MODE_OVERRIDE: AtomicBool = AtomicBool::new(false);

#[cfg(test)]
static READ_COUNT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[cfg(test)]
pub fn read_count() -> u64 {
    READ_COUNT.load(std::sync::atomic::Ordering::Relaxed)
}

/// Bust the cached parse.
pub fn invalidate_cache() {
    linkscope::record_items("config.cache.invalidate", 1);
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
    linkscope::record_items("config.cache.generation", 1);
    CACHE_GENERATION.fetch_add(1, Ordering::AcqRel);
}

/// Read + parse config from disk, no caching.
fn load_from(path: &Path) -> Config {
    let _linkscope_load = linkscope::phase("config.load_from");
    trace_path_event("config.load_from.start", path);
    let mut cfg = load_toml_from(path);
    if path == config_path().as_path()
        && let Ok(project_root) = std::env::current_dir()
    {
        claude_settings::apply_to_config(&mut cfg, &project_root);
    }
    cfg
}

fn load_toml_from(path: &Path) -> Config {
    load_toml_from_depth(path, 0)
}

/// Inner recursive loader that handles `extends` inheritance.  Depth-limited
/// to 8 levels so a circular chain of `extends` doesn't hang the process.
fn load_toml_from_depth(path: &Path, depth: u8) -> Config {
    let _linkscope_load = linkscope::phase("config.load_toml");
    trace::record_path_shape("config.load_toml.start", path);
    if linkscope::is_enabled() {
        linkscope::detail_event_fields(
            "config.load_toml.start",
            [
                linkscope::TraceField::text("path", path.display().to_string()),
                linkscope::TraceField::count("depth", u64::from(depth)),
            ],
        );
    }
    #[cfg(test)]
    READ_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    let raw = match std::fs::read_to_string(path) {
        Ok(s) => {
            trace::record_config_load(trace::ConfigLoadTrace {
                label: "config.toml.load",
                depth,
                bytes: s.len(),
            });
            linkscope::record_bytes("config.toml.read", usize_to_u64_saturating(s.len()));
            s
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            trace::record_status("config.toml.load_result", "missing");
            linkscope::record_items("config.toml.missing", 1);
            tracing::trace!(
                target: "jfc::config",
                path = %path.display(),
                "config file not found — using defaults"
            );
            return Config::default();
        }
        Err(e) => {
            trace::record_status("config.toml.load_result", "read_error");
            linkscope::record_items("config.toml.read_error", 1);
            tracing::warn!(
                target: "jfc::config",
                path = %path.display(),
                error = %e,
                "failed to read config file — using defaults"
            );
            return Config::default();
        }
    };
    let local: Config = match toml::from_str::<Config>(&raw) {
        Ok(cfg) => {
            trace::record_config_shape("config.toml.shape", &cfg, depth);
            trace::record_status("config.toml.load_result", "parsed");
            linkscope::record_items("config.toml.parsed", 1);
            cfg
        }
        Err(e) => {
            trace::record_status("config.toml.load_result", "parse_error");
            linkscope::record_items("config.toml.parse_error", 1);
            tracing::warn!(
                target: "jfc::config",
                path = %path.display(),
                error = %e,
                "failed to parse config — using defaults"
            );
            return Config::default();
        }
    };

    // Handle `extends` inheritance: load the base config and merge, with the
    // local config's values taking priority.  Guard against deep / circular
    // chains.
    if let Some(ref base_rel) = local.extends {
        linkscope::record_items("config.toml.extends", 1);
        const MAX_DEPTH: u8 = 8;
        if depth >= MAX_DEPTH {
            linkscope::record_items("config.toml.extends_too_deep", 1);
            tracing::warn!(
                target: "jfc::config",
                path = %path.display(),
                "config `extends` chain too deep (max {MAX_DEPTH}); ignoring further inheritance"
            );
            return local;
        }
        let base_path = path
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(base_rel);
        tracing::debug!(
            target: "jfc::config",
            local = %path.display(),
            base = %base_path.display(),
            depth,
            "loading base config via `extends`"
        );
        let base = load_toml_from_depth(&base_path, depth + 1);
        let merged = base.merge_with(local);
        trace::record_config_shape("config.toml.merged_shape", &merged, depth);
        return merged;
    }

    local
}

/// Load the canonical TOML config plus Claude-compatible JSON settings for a
/// specific project root. This bypasses the hot cache so tests and daemon
/// workers can resolve project-local `.claude/settings*.json` deterministically.
pub fn load_with_project(project_root: &Path) -> Config {
    let _linkscope_load = linkscope::phase("config.load_with_project");
    trace_path_event("config.load_with_project.start", project_root);
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
    let _linkscope_load = linkscope::phase("config.load_arc");
    load_cached_arc(&config_path())
}

pub fn set_safe_mode_override(enabled: bool) {
    SAFE_MODE_OVERRIDE.store(enabled, Ordering::Release);
}

pub fn safe_mode_enabled() -> bool {
    if SAFE_MODE_OVERRIDE.load(Ordering::Acquire) {
        return true;
    }
    if std::env::var("JFC_SAFE_MODE")
        .or_else(|_| std::env::var("CLAUDE_CODE_SAFE_MODE"))
        .ok()
        .is_some_and(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
    {
        return true;
    }
    load_arc().safe_mode
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
    let _linkscope_load = linkscope::phase("config.managed_settings.load");
    for path in managed_settings_paths() {
        let Ok(raw) = std::fs::read_to_string(&path) else {
            continue;
        };
        match toml::from_str::<ManagedSettingsConfig>(&raw) {
            Ok(settings) => {
                linkscope::record_items("config.managed_settings.file_loaded", 1);
                return Some(settings);
            }
            Err(e) => {
                linkscope::record_items("config.managed_settings.parse_error", 1);
                tracing::warn!(
                    target: "jfc::config",
                    path = %path.display(),
                    error = %e,
                    "failed to parse managed settings - ignoring"
                );
            }
        }
    }
    linkscope::record_items("config.managed_settings.embedded_fallback", 1);
    load().managed_settings
}

/// Report all managed-settings sources in precedence order. This is separate
/// from [`load_managed_settings`] so policy diagnostics can explain why a
/// higher-precedence file was skipped instead of only showing the final merge.
pub fn managed_settings_sources() -> Vec<ManagedSettingsSource> {
    let _linkscope_sources = linkscope::phase("config.managed_settings.sources");
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
    let _linkscope_load = linkscope::phase("config.load_cached_arc");
    let generation = cache_generation();
    let canonical_path = config_path();
    let generation_only = path == canonical_path.as_path();
    let cur_mtime = if generation_only {
        None
    } else {
        std::fs::metadata(path).and_then(|m| m.modified()).ok()
    };
    trace::record_cache_probe(trace::ConfigCacheTrace {
        label: "config.cache.probe",
        generation,
        generation_only,
        mtime_known: cur_mtime.is_some(),
    });

    {
        let slot = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = slot.as_ref()
            && c.path == path
            && c.generation == generation
            && c.mtime == cur_mtime
        {
            trace::record_cache_probe(trace::ConfigCacheTrace {
                label: "config.cache.hit_shape",
                generation,
                generation_only,
                mtime_known: cur_mtime.is_some(),
            });
            linkscope::record_items("config.cache.hit", 1);
            return Arc::clone(&c.config);
        }
    }

    trace::record_cache_probe(trace::ConfigCacheTrace {
        label: "config.cache.miss_shape",
        generation,
        generation_only,
        mtime_known: cur_mtime.is_some(),
    });
    linkscope::record_items("config.cache.miss", 1);
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
    let _linkscope_save = linkscope::phase("config.save_theme");
    trace_path_event("config.save_theme.start", path);
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
                    "{} is not valid TOML - fix it first ({e})",
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
    if let Some(name) = value
        .strip_prefix("db://system-prompt/")
        .or_else(|| value.strip_prefix("db://system_prompt/"))
        && let Some(body) = load_system_prompt_definition(name, base_dir)
    {
        return body;
    }
    if let Some(path_str) = value.strip_prefix("file://") {
        let path = if let Some(base) = base_dir {
            base.join(path_str)
        } else {
            PathBuf::from(path_str)
        };
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                import_system_prompt_definition(&path, &content, base_dir);
                content
            }
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

fn load_system_prompt_definition(name: &str, base_dir: Option<&std::path::Path>) -> Option<String> {
    let store = open_definition_store(base_dir)?;
    let current_dir = std::env::current_dir().ok();
    let project_root = base_dir.or(current_dir.as_deref())?;
    let project_key = jfc_knowledge::project_key(project_root);
    let project = jfc_knowledge::block_on_knowledge(async {
        store
            .get_definition_by_name(
                "system_prompt",
                jfc_knowledge::DefinitionScope::Project,
                Some(&project_key),
                None,
                name,
            )
            .await
    })
    .ok()
    .flatten();
    project.map(|def| def.body)
}

fn import_system_prompt_definition(
    path: &std::path::Path,
    content: &str,
    base_dir: Option<&std::path::Path>,
) {
    let Some(store) = open_definition_store(base_dir) else {
        return;
    };
    let project_root = base_dir.unwrap_or_else(|| path.parent().unwrap_or_else(|| Path::new(".")));
    let project_key = jfc_knowledge::project_key(project_root);
    let name = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .filter(|stem| !stem.is_empty())
        .unwrap_or("system")
        .to_owned();
    let def = jfc_knowledge::NewDefinition {
        kind: "system_prompt".to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some(project_key),
        namespace: None,
        name,
        title: None,
        description: Some("Imported system prompt".to_owned()),
        body: content.to_owned(),
        metadata_json: serde_json::json!({
            "legacy_import": true,
        })
        .to_string(),
        source_path: Some(path.to_string_lossy().to_string()),
        source_hash: Some(definition_content_hash(content)),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "legacy_import".to_owned(),
    };
    if let Err(err) =
        jfc_knowledge::block_on_knowledge(async { store.upsert_definition(&def).await })
    {
        tracing::warn!(
            target: "jfc::config",
            path = %path.display(),
            error = %err,
            "failed to import system prompt definition"
        );
    }
}

fn open_definition_store(
    base_dir: Option<&std::path::Path>,
) -> Option<jfc_knowledge::KnowledgeStore> {
    #[cfg(test)]
    {
        let root = base_dir.unwrap_or_else(|| Path::new("."));
        let path = root.join(".jfc").join("definition-test.db");
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open(&path)).ok()
    }
    #[cfg(not(test))]
    {
        let _ = base_dir;
        jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open_default()).ok()
    }
}

fn definition_content_hash(raw: &str) -> String {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    raw.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
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

fn trace_path_event(name: &'static str, path: &Path) {
    if linkscope::is_enabled() {
        linkscope::detail_event_fields(
            name,
            [linkscope::TraceField::text(
                "path",
                path.display().to_string(),
            )],
        );
    }
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
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
    fn config_load_trace_records_shape_without_config_values_normal() {
        linkscope::trace_detail_enable();
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
theme = "private-theme-value"
safe_mode = true

[agents.private-agent-name]
model = "private-model-value"

[mcp.private-server-name]
command = "private-command-value"
"#,
        )
        .expect("write config");

        let cfg = load_toml_from(&path);

        assert!(cfg.safe_mode);
        let snapshot = linkscope::snapshot();
        let rendered = format!("{snapshot:?}");
        assert!(rendered.contains("config.toml.load"));
        assert!(rendered.contains("config.toml.shape"));
        assert!(rendered.contains("agents"));
        assert!(rendered.contains("mcp"));
        assert!(!rendered.contains("private-theme-value"));
        assert!(!rendered.contains("private-agent-name"));
        assert!(!rendered.contains("private-model-value"));
        assert!(!rendered.contains("private-server-name"));
        assert!(!rendered.contains("private-command-value"));
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
    fn voice_table_parses_and_converts_to_compat_json_normal() {
        let cfg = parse(
            r#"
[voice]
enabled = true
mode = "vad"
auto_submit = true
backend = "anthropic"
read_aloud = true
tts_voice = "buttery"
conversation_enabled = true
organization_uuid = "org-123"
conversation_uuid = "conv-456"
allow_custom_auth_endpoint = true
"#,
        );
        let voice = cfg.voice.expect("voice config");
        let compat = voice.to_compat_json();

        assert_eq!(voice.mode.as_deref(), Some("vad"));
        assert_eq!(compat["enabled"], true);
        assert_eq!(compat["autoSubmit"], true);
        assert_eq!(compat["readAloud"], true);
        assert_eq!(compat["ttsVoice"], "buttery");
        assert_eq!(compat["conversationEnabled"], true);
        assert_eq!(compat["organizationUuid"], "org-123");
        assert_eq!(compat["conversationUuid"], "conv-456");
        assert_eq!(compat["allowCustomAuthEndpoint"], true);
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
    fn prompt_rewrite_defaults_on_regression() {
        let cfg = parse(
            r#"
[default]
model = "x"
"#,
        );
        assert!(cfg.prompt_rewrite.is_none());
        assert!(PromptRewriteConfig::default().enabled);
        assert!(Config::default().refusal_rewrite_retry_enabled);
        assert!(Config::default().subagent_context_inheritance);
        // Privacy default: refusal CoT logging is OFF unless explicitly enabled.
        assert!(!Config::default().refusal_log_reasoning);
    }

    #[test]
    fn refusal_log_reasoning_opt_in_parses() {
        let cfg = parse(
            r#"
refusal_log_reasoning = true

[default]
model = "x"
"#,
        );
        assert!(
            cfg.refusal_log_reasoning,
            "explicit opt-in should enable refusal CoT logging"
        );
    }

    #[test]
    fn prompt_rewrite_parses_when_present() {
        let cfg = parse(
            r#"
[default]
model = "x"

[prompt_rewrite]
enabled = true
model = "local-judge"
threshold = 0.8
constitution = "PERMITTED: coding. DISALLOWED: harm."
"#,
        );
        let pr = cfg.prompt_rewrite.expect("section present");
        assert!(pr.enabled);
        assert_eq!(pr.model.as_deref(), Some("local-judge"));
        assert_eq!(pr.threshold, Some(0.8));
        assert!(pr.constitution.unwrap().contains("PERMITTED"));
    }

    #[test]
    fn prompt_rewrite_section_defaults_enabled_regression() {
        let cfg = parse(
            r#"
[prompt_rewrite]
model = "local-judge"
"#,
        );
        assert!(cfg.prompt_rewrite.expect("section present").enabled);
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
    fn resolve_prompt_imports_file_uri_to_db_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let prompt_file = tmp.path().join("system.md");
        std::fs::write(&prompt_file, "You are a helpful assistant.").unwrap();

        let from_file = resolve_prompt("file://system.md", Some(tmp.path()));
        let from_db = resolve_prompt("db://system-prompt/system", Some(tmp.path()));

        assert_eq!(from_file, "You are a helpful assistant.");
        assert_eq!(from_db, "You are a helpful assistant.");
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
    fn resolve_config_path_uses_kfc_alias_when_present_regression() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let kfc_dir = tmp.path().join("kfc");
        std::fs::create_dir_all(&kfc_dir).expect("mkdir");
        std::fs::write(kfc_dir.join("config.toml"), b"theme = \"claude\"\n").expect("write");

        let resolved = resolve_config_path(tmp.path());

        assert_eq!(resolved, tmp.path().join("kfc").join("config.toml"));
    }

    #[test]
    fn resolve_config_path_defaults_to_jfc_when_alias_missing_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");

        let resolved = resolve_config_path(tmp.path());

        assert_eq!(resolved, tmp.path().join("jfc").join("config.toml"));
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

    #[test]
    fn isolation_default_task_isolation_parses_snake_case_normal() {
        let cfg: Config =
            toml::from_str("[isolation]\ndefault_task_isolation = \"worktree\"\n").expect("parse");
        let isolation = cfg.isolation.expect("present");
        assert!(isolation.fail_closed);
        assert_eq!(
            isolation.default_task_isolation.as_deref(),
            Some("worktree")
        );
    }

    #[test]
    fn isolation_default_task_isolation_parses_camel_case_normal() {
        let cfg: Config =
            toml::from_str("[isolation]\ndefaultTaskIsolation = \"worktree\"\n").expect("parse");
        let isolation = cfg.isolation.expect("present");
        assert!(isolation.fail_closed);
        assert_eq!(
            isolation.default_task_isolation.as_deref(),
            Some("worktree")
        );
    }

    // ── Feature 1: bash_shell ───────────────────────────────────────────────

    #[test]
    fn bash_shell_defaults_to_none_normal() {
        let cfg = Config::default();
        assert!(cfg.bash_shell.is_none(), "bash_shell must default to None");
    }

    #[test]
    fn bash_shell_parses_from_toml_normal() {
        let cfg: Config = toml::from_str(r#"bash_shell = "/bin/zsh""#).expect("parse");
        assert_eq!(cfg.bash_shell.as_deref(), Some("/bin/zsh"));
    }

    #[test]
    fn bash_shell_alias_camel_case_normal() {
        let cfg: Config = toml::from_str(r#"bashShell = "fish""#).expect("parse");
        assert_eq!(cfg.bash_shell.as_deref(), Some("fish"));
    }

    // ── Feature 2: extends / config inheritance ─────────────────────────────

    #[test]
    fn extends_parses_from_toml_normal() {
        let cfg: Config = toml::from_str(r#"extends = "base.toml""#).expect("parse");
        assert_eq!(
            cfg.extends.as_deref(),
            Some(std::path::Path::new("base.toml"))
        );
    }

    #[test]
    fn merge_with_local_wins_over_base_normal() {
        let base = Config {
            theme: Some("base-theme".into()),
            bash_shell: Some("sh".into()),
            always_show_thinking: false,
            ..Config::default()
        };
        let local = Config {
            theme: Some("local-theme".into()),
            bash_shell: None, // not set in local → fall through to base
            always_show_thinking: true,
            ..Config::default()
        };
        let merged = base.merge_with(local);
        assert_eq!(
            merged.theme.as_deref(),
            Some("local-theme"),
            "local theme wins"
        );
        assert_eq!(
            merged.bash_shell.as_deref(),
            Some("sh"),
            "base bash_shell kept when local is None"
        );
        assert!(merged.always_show_thinking, "local bool wins");
    }

    #[test]
    #[serial_test::serial]
    fn extends_loads_base_and_merges_normal() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let base_path = tmp.path().join("base.toml");
        let local_path = tmp.path().join("config.toml");

        std::fs::write(
            &base_path,
            r#"bash_shell = "/bin/sh"
theme = "base-theme"
"#,
        )
        .unwrap();
        std::fs::write(
            &local_path,
            r#"extends = "base.toml"
theme = "local-theme"
"#,
        )
        .unwrap();

        let cfg = load_toml_from(&local_path);
        assert_eq!(
            cfg.theme.as_deref(),
            Some("local-theme"),
            "local theme wins"
        );
        assert_eq!(
            cfg.bash_shell.as_deref(),
            Some("/bin/sh"),
            "base bash_shell inherited"
        );
    }

    // ── Feature 3: auto_compact_threshold_pct ──────────────────────────────

    #[test]
    fn auto_compact_threshold_pct_defaults_to_85_normal() {
        let cfg = Config::default();
        assert_eq!(cfg.auto_compact_threshold_pct, 85);
    }

    #[test]
    fn cross_project_recall_defaults_off_regression() {
        let cfg = Config::default();
        assert!(!cfg.cross_project_recall_enabled);

        let parsed: Config = toml::from_str("").expect("parse minimal config");
        assert!(!parsed.cross_project_recall_enabled);
    }

    #[test]
    fn auto_compact_threshold_pct_parses_from_toml_normal() {
        let cfg: Config = toml::from_str("auto_compact_threshold_pct = 70").expect("parse");
        assert_eq!(cfg.auto_compact_threshold_pct, 70);
    }

    #[test]
    fn continuation_auto_continue_defaults_on_regression() {
        assert!(ContinuationConfig::default().auto_continue);
    }

    // ── Feature 4: always_show_thinking ────────────────────────────────────

    #[test]
    fn always_show_thinking_defaults_false_normal() {
        let cfg = Config::default();
        assert!(!cfg.always_show_thinking);
    }

    #[test]
    fn always_show_thinking_parses_from_toml_normal() {
        let cfg: Config = toml::from_str("always_show_thinking = true").expect("parse");
        assert!(cfg.always_show_thinking);
    }

    #[test]
    fn redacted_thinking_defaults_off_regression() {
        let cfg = Config::default();
        assert!(!cfg.redacted_thinking_enabled());
        assert!(cfg.anthropic_betas(std::iter::empty::<String>()).is_empty());
    }

    #[test]
    fn redacted_thinking_flag_appends_beta_when_enabled_normal() {
        let cfg: Config = toml::from_str("redacted_thinking_enabled = true").expect("parse");
        assert!(cfg.redacted_thinking_enabled());
        assert_eq!(
            cfg.anthropic_betas(["custom-beta-2099-01-01"]),
            vec![
                REDACT_THINKING_BETA.to_owned(),
                "custom-beta-2099-01-01".to_owned()
            ]
        );
    }

    #[test]
    fn redacted_thinking_false_keeps_beta_out_robust() {
        let cfg: Config = toml::from_str("redacted_thinking_enabled = false").expect("parse");
        assert!(!cfg.redacted_thinking_enabled());
        assert_eq!(
            cfg.anthropic_betas(["custom-beta-2099-01-01"]),
            vec!["custom-beta-2099-01-01".to_owned()]
        );
    }
}
