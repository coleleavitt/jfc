#![allow(clippy::all)]

use std::io;
use std::path::PathBuf;

use clap::{Parser, Subcommand};
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        PopKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

mod auth;
mod bridge;
mod changes;
mod daemon;
mod headless;
mod logging;
mod memory;
mod plugin;
mod policy;
mod provider_bootstrap;
mod rc;
mod terminal;

pub(crate) use provider_bootstrap::{
    build_providers, provider_for_model, qualified_model_id, resolve_provider_model,
};

use auth::{AuthSubcommand, run_auth_subcommand};
use bridge::{BridgeSubcommand, run_bridge_subcommand};
use changes::{ChangesSubcommand, run_changes_subcommand};
use daemon::{DaemonSubcommand, compact_terminal_agents_on_startup, run_daemon_subcommand};
use headless::{
    HeadlessInputFormat, HeadlessOutputFormat, PrintModeConfig, run_print_mode, run_remote_session,
};
use jfc_provider::ModelId;
use logging::init_tracing;
use memory::{MemorySubcommand, run_memory_subcommand};
use plugin::{PluginSubcommand, run_plugin_subcommand};
use policy::{PolicySubcommand, run_policy_subcommand};
use rc::{RcSubcommand, run_rc_subcommand};
use terminal::{enable_keyboard_enhancement, install_terminal_panic_hook};

/// JFC - A TUI assistant for code exploration and development
#[derive(Parser, Debug)]
#[command(name = "jfc", version, about)]
pub(crate) struct Cli {
    /// Resume the most recent session
    #[arg(long = "continue", short = 'c')]
    continue_session: bool,

    /// Resume a specific session by ID
    #[arg(long, short = 'r', value_name = "SESSION_ID")]
    resume: Option<String>,

    /// Initial prompt to send. With `--print`, runs headless and exits.
    /// Without `--print`, opens the TUI and pre-fills the input.
    #[arg(long, short = 'p', value_name = "PROMPT")]
    prompt: Option<String>,

    /// Headless one-shot mode: send the prompt, stream the response to
    /// stdout, exit. Skip the TUI entirely. Pair with `--prompt "..."`
    /// or pipe the prompt on stdin.
    #[arg(long = "print", short = 'P')]
    print_mode: bool,

    /// Headless output format for `--print`: `text`, `json`, or `stream-json`.
    #[arg(long = "output-format", value_enum, default_value = "text")]
    output_format: HeadlessOutputFormat,

    /// Headless input format for `--print`: `text` or `stream-json`.
    #[arg(long = "input-format", value_enum, default_value = "text")]
    input_format: HeadlessInputFormat,

    /// Include hook lifecycle events in `--output-format stream-json`.
    #[arg(long = "include-hook-events")]
    include_hook_events: bool,

    /// Include cumulative partial assistant messages in `stream-json` output.
    #[arg(long = "include-partial-messages")]
    include_partial_messages: bool,

    /// Write a JSON mirror of the headless session transcript to this path.
    #[arg(long = "session-mirror", value_name = "PATH")]
    session_mirror: Option<PathBuf>,

    /// Tool name an SDK host uses for permission prompts in headless mode.
    /// Parsed for Claude Code SDK flag parity; local JFC permission handling
    /// still uses its native permission modes.
    #[arg(long = "permission-prompt-tool", value_name = "TOOL")]
    permission_prompt_tool: Option<String>,

    /// SDK control URL for hosted integrations. Parsed for flag parity; JFC's
    /// local runtime does not require it for normal TUI/headless operation.
    #[arg(long = "sdk-url", value_name = "URL")]
    sdk_url: Option<String>,

    /// Connect to a remote managed-agent session by ID. Streams the
    /// session's events into the TUI transcript and forwards user
    /// input via the Sessions API. Pairs with the SDK's
    /// `managed_session::ManagedSession`.
    #[arg(long = "remote-session", value_name = "SESSION_ID")]
    remote_session: Option<String>,

    /// Model to use (overrides ANTHROPIC_MODEL env var)
    #[arg(long, short = 'm', value_name = "MODEL")]
    model: Option<String>,

    /// Set JFC's local/client-side advisor model. Optional value accepts `opus`,
    /// `sonnet`, `haiku`, a full model id, or uses the active model when omitted.
    #[arg(
        long = "advisor",
        value_name = "MODEL",
        num_args = 0..=1,
        default_missing_value = "",
        conflicts_with = "no_advisor"
    )]
    advisor: Option<String>,

    /// Disable JFC's local/client-side advisor for this launch.
    #[arg(long = "no-advisor")]
    no_advisor: bool,

    /// Enable Anthropic's upstream server-side advisor tool. Optional value
    /// accepts `opus`, `sonnet`, or a full model id.
    #[arg(
        long = "server-advisor",
        value_name = "MODEL",
        num_args = 0..=1,
        default_missing_value = "opus"
    )]
    server_advisor: Option<String>,

    /// Initial permission mode. Matches Claude Code 2.1.141's
    /// `--permission-mode` flag: lets a caller boot directly into a
    /// non-default mode without going through Shift+Tab inside the
    /// TUI. One of: `default`, `plan`, `accept-edits` (or
    /// `acceptedits`), `bypass` (or `bypasspermissions`), `auto`.
    ///
    /// Unknown values are logged and ignored — we don't refuse to
    /// boot just because the flag is misspelled.
    #[arg(long = "permission-mode", value_name = "MODE")]
    permission_mode: Option<String>,

    /// Fork a session: create a new session ID but copy messages from the
    /// given session (or teleport export id). Use with `--resume` to
    /// branch off an existing conversation.
    #[arg(long = "fork-session", value_name = "SESSION_OR_EXPORT_ID")]
    fork_session: Option<String>,

    /// Maximum number of agentic-loop iterations per user turn. When
    /// the count is hit, the loop stops even if the model wanted to
    /// keep going. Mirrors Claude Code v2.1.144's `--max-turns`.
    #[arg(long = "max-turns", value_name = "N")]
    max_turns: Option<u32>,

    /// Hard spend cap in USD across the session. The app refuses to
    /// open a new stream once cumulative cost crosses this number.
    #[arg(long = "max-budget-usd", value_name = "DOLLARS")]
    max_budget_usd: Option<f64>,

    /// Comma-separated allowlist of tool names. Anything outside this
    /// list is rejected. Empty / unset = no allowlist (default jfc
    /// behaviour). Example: `--allowed-tools "Read,Glob,Grep"`.
    #[arg(
        long = "allowed-tools",
        alias = "allowedTools",
        alias = "tools",
        value_name = "TOOLS"
    )]
    allowed_tools: Option<String>,

    /// Comma-separated denylist of tool names. Names in this list are
    /// rejected regardless of the allowlist.
    #[arg(
        long = "disallowed-tools",
        alias = "disallowedTools",
        value_name = "TOOLS"
    )]
    disallowed_tools: Option<String>,

    /// Extra system-prompt text appended after the built-in prompt.
    /// Use for project-specific style, persona, or constraint
    /// reminders. Mutually overrideable with `--system-prompt-file`
    /// (file wins if both are set).
    #[arg(
        long = "system-prompt",
        alias = "append-system-prompt",
        value_name = "TEXT"
    )]
    system_prompt: Option<String>,

    /// Read additional system-prompt text from the given file. Takes
    /// precedence over `--system-prompt` when both are supplied.
    #[arg(
        long = "system-prompt-file",
        alias = "append-system-prompt-file",
        value_name = "PATH"
    )]
    system_prompt_file: Option<PathBuf>,

    /// Skip every permission check — every tool runs without prompting.
    /// DANGEROUS: equivalent to permanently being in `bypass` permission
    /// mode. Use only in sandboxed CI / single-shot scripts.
    #[arg(long = "dangerously-skip-permissions")]
    dangerously_skip_permissions: bool,

    /// Enable debug-level logging. Equivalent to setting
    /// `RUST_LOG=jfc=debug` before launch.
    #[arg(long = "verbose")]
    verbose: bool,

    /// JSON output mode — emit structured events on stdout instead of
    /// rich TUI rendering. Intended for CI / scripted callers.
    #[arg(long = "json")]
    json: bool,

    /// Start the remote-control WebSocket server at launch (alias `--rc`).
    /// Equivalent to running `/remote-control` once the TUI is up — prints
    /// the pairing token + connection URL. Connect from another device with
    /// `jfc rc connect ws://HOST:4242 --token <TOKEN>`.
    #[arg(long = "remote-control", visible_alias = "rc")]
    remote_control: bool,

    /// Add an extra directory to the search-context allowlist.
    /// Accepts multiple occurrences: `--add-dir /a --add-dir /b`.
    #[arg(long = "add-dir", value_name = "PATH")]
    add_dir: Vec<PathBuf>,

    /// Maximum extended-thinking budget (tokens) per turn. Lower
    /// values trade reasoning depth for latency / cost.
    #[arg(long = "max-thinking-tokens", value_name = "N")]
    max_thinking_tokens: Option<u32>,

    /// Visibility of thinking output: `show`, `hide`, or `summarize`.
    /// Maps onto `StreamOptions.thinking_display`.
    #[arg(long = "thinking-display", value_name = "MODE")]
    thinking_display: Option<String>,

    /// Ephemeral session: don't persist any state to
    /// `~/.config/jfc/sessions/`. Useful for transient probes / CI.
    #[arg(long = "no-session-persistence")]
    no_session_persistence: bool,

    /// Token budget for a single task (beta `task-budgets-2026-03-13`).
    /// Wires through `StreamOptions.task_budget_tokens`.
    #[arg(long = "task-budget", value_name = "N")]
    task_budget: Option<u64>,

    /// Additional Anthropic beta tokens to append to native requests.
    /// Accepts comma-separated values and can be repeated.
    #[arg(long = "betas", value_name = "BETA", value_delimiter = ',')]
    betas: Vec<String>,

    /// Add a local plugin directory for this run. The path may point at a
    /// plugin root containing `.jfc-plugin.toml` or directly at a workflows dir.
    #[arg(long = "plugin-dir", value_name = "DIR")]
    plugin_dir: Vec<PathBuf>,

    /// Install/register a git plugin URL for this run.
    #[arg(long = "plugin-url", value_name = "URL")]
    plugin_url: Vec<String>,

    /// Attach `eager_input_streaming` to Anthropic native tool schemas.
    #[arg(long = "fine-grained-tool-streaming")]
    fine_grained_tool_streaming: bool,

    /// Attach `strict: true` to Anthropic native tool schemas.
    #[arg(long = "strict-tool-schemas")]
    strict_tool_schemas: bool,

    /// Path to an MCP configuration file. Servers in this file are
    /// merged into the registry at startup before any tool dispatch.
    #[arg(long = "mcp-config", value_name = "PATH")]
    mcp_config: Option<PathBuf>,

    /// IDE pairing mode flag. When set, the UI advertises co-working
    /// affordances (shared cursor, context handoff) to a connected
    /// editor extension.
    #[arg(long = "cowork")]
    cowork: bool,

    /// Subcommand. When omitted, jfc launches the interactive TUI.
    #[command(subcommand)]
    command: Option<Command>,
}

/// Top-level subcommands. Currently the daemon family and `auth` for
/// multi-account OAuth management — leaving the TUI as the default
/// invocation keeps `jfc` ergonomic for humans.
#[derive(Subcommand, Debug)]
enum Command {
    /// Manage the background daemon (cron jobs + scheduled wakeups).
    Daemon {
        #[command(subcommand)]
        sub: DaemonSubcommand,
    },
    /// Manage provider authentication (OAuth/API-key helpers).
    Auth {
        #[command(subcommand)]
        sub: AuthSubcommand,
    },
    /// Remote control — connect to or probe a running session.
    Rc {
        #[command(subcommand)]
        sub: RcSubcommand,
    },
    /// Review/apply/revert agent change-sets (isolated branch proposals).
    Changes {
        #[command(subcommand)]
        sub: ChangesSubcommand,
    },
    /// Manage local plugins and workflow bundles.
    Plugin {
        #[command(subcommand)]
        sub: PluginSubcommand,
    },
    /// Inspect local managed-settings policy and source precedence.
    Policy {
        #[command(subcommand)]
        sub: PolicySubcommand,
    },
    /// Manage local memory files and team-memory sync.
    Memory {
        #[command(subcommand)]
        sub: MemorySubcommand,
    },
    /// Run or inspect the self-hosted worker bridge.
    Bridge {
        #[command(subcommand)]
        sub: BridgeSubcommand,
    },
}

/// Parse the `--permission-mode` CLI value into the `PermissionMode`
/// enum. Accepts the canonical labels jfc uses (`default`, `plan`,
/// `accept-edits`, `bypass`, `auto`) plus their hyphen-stripped
/// equivalents (`acceptedits`, `bypasspermissions`) and the v141
/// `bypassPermissions` casing for parity with Claude Code's spelling.
///
/// Returns `None` for `None` input or unknown labels — boot proceeds
/// in the default mode and the misuse is logged. We don't refuse to
/// start the TUI just because the flag is misspelled.
pub(crate) fn parse_permission_mode(raw: Option<&str>) -> Option<crate::app::PermissionMode> {
    let raw = raw?.trim();
    if raw.is_empty() {
        return None;
    }
    let normalized = raw.to_ascii_lowercase().replace(['-', '_'], "");
    let mode = match normalized.as_str() {
        "default" | "normal" => crate::app::PermissionMode::Default,
        "plan" => crate::app::PermissionMode::Plan,
        "acceptedits" | "edits" => crate::app::PermissionMode::AcceptEdits,
        "bypass" | "bypasspermissions" | "yolo" => crate::app::PermissionMode::BypassPermissions,
        "auto" => crate::app::PermissionMode::Auto,
        _ => {
            tracing::warn!(
                target: "jfc::cli",
                raw = %raw,
                "--permission-mode: unknown value, falling back to Default"
            );
            return None;
        }
    };
    tracing::info!(
        target: "jfc::cli",
        ?mode,
        "applied --permission-mode"
    );
    Some(mode)
}

/// Bag of CLI-flag-derived values transferred onto `App` after
/// construction. Lives at module scope so `event_loop::run` can
/// accept it without depending on the `Cli` type directly.
#[derive(Debug, Default, Clone)]
pub(crate) struct CliRuntimeConfig {
    pub max_turns: Option<u32>,
    pub max_budget_usd: Option<f64>,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub system_prompt: Option<String>,
    pub dangerously_skip_permissions: bool,
    pub json_mode: bool,
    pub extra_dirs: Vec<PathBuf>,
    pub max_thinking_tokens: Option<u32>,
    pub thinking_display: Option<String>,
    pub no_session_persistence: bool,
    pub task_budget: Option<u64>,
    pub mcp_config_path: Option<PathBuf>,
    pub cowork: bool,
    pub local_advisor_model: Option<ModelId>,
    pub server_advisor_model: Option<ModelId>,
    pub custom_betas: Vec<String>,
    pub fine_grained_tool_streaming: bool,
    pub strict_tool_schemas: bool,
    /// Start the remote-control server at launch (from `--remote-control`
    /// or the `[remote_control] auto_start = true` config).
    pub remote_control: bool,
}

/// Parse a comma-separated tool list (`"Read, Glob,Grep"` → `["Read",
/// "Glob", "Grep"]`). Whitespace around each entry is trimmed; empty
/// entries are dropped so `"Read,,Glob"` yields two names, not three.
fn parse_tool_list(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn dedup_tool_list(tools: &mut Vec<String>) {
    let mut seen = std::collections::HashSet::new();
    tools.retain(|tool| seen.insert(tool.to_ascii_lowercase()));
}

fn managed_forces_non_bypass(managed: Option<&crate::config::ManagedSettingsConfig>) -> bool {
    managed
        .and_then(|m| m.force_permission_mode.as_deref())
        .and_then(|mode| parse_permission_mode(Some(mode)))
        .is_some_and(|mode| mode != crate::app::PermissionMode::BypassPermissions)
}

/// Session to load at startup based on CLI args
pub(crate) enum StartupSession {
    /// No session to load — start fresh
    Fresh,
    /// Continue most recent session
    Continue,
    /// Resume specific session by ID
    Resume(String),
    /// Fork: copy messages from an existing session into a new session ID
    Fork(String),
}

impl Cli {
    fn startup_session(&self) -> StartupSession {
        if let Some(ref fork_id) = self.fork_session {
            StartupSession::Fork(fork_id.clone())
        } else if let Some(ref id) = self.resume {
            StartupSession::Resume(id.clone())
        } else if self.continue_session {
            StartupSession::Continue
        } else {
            StartupSession::Fresh
        }
    }
}

pub(crate) async fn run(cli: Cli) -> anyhow::Result<()> {
    // `--verbose` must be honored *before* `init_tracing` reads the env,
    // since the filter is captured into the global subscriber once and
    // can't be changed later. We only set RUST_LOG if it wasn't already
    // provided externally — explicit user override always wins.
    if cli.verbose && std::env::var_os("RUST_LOG").is_none() {
        // SAFETY: we're still single-threaded at this point in startup.
        // No other thread can read RUST_LOG concurrently with this write.
        unsafe {
            std::env::set_var("RUST_LOG", "jfc=debug,reqwest=warn,hyper=warn,h2=warn");
        }
    }

    // Tracing → file under `~/.config/jfc/logs/`. Stderr writes corrupted the
    // TUI alt-screen, so we route to a rolling daily file via
    // `tracing-appender::non_blocking`. The `WorkerGuard` is held for the
    // lifetime of `main` so buffered writes flush on exit (per the tracing
    // skill: dropping the guard early loses logs).
    //
    // Filter via `RUST_LOG` (e.g. `RUST_LOG=jfc=debug,reqwest=warn`); default
    // is `info` which lights up the high-signal #[instrument] spans we
    // sprinkled across providers, the classifier, and the tool dispatcher.
    let _trace_guard = init_tracing(cli.command.is_some());

    // Initialize the process-global hook registry once. From here on
    // any `crate::hooks::fire(point, ctx)` call short-circuits to the
    // registered handlers (Logger only, by default — user-defined
    // hooks land via .claude/settings.json in a future pass). Idempotent.
    crate::hooks::init_global(crate::hooks::default_registry());

    // Clean up tool-result spill files older than 24h to prevent unbounded /tmp growth.
    crate::stream::cleanup_tool_result_spills(std::time::Duration::from_secs(24 * 3600));

    // v132 file watcher: install on startup so CLAUDE.md /
    // .claude/agents/*.md / settings.toml edits emit a system-reminder
    // on the next turn instead of waiting for a session restart. The
    // Tick handler in the main loop polls the change counter and
    // emits the reminder when it sees a bump.
    crate::file_watcher::install();
    crate::keybindings::load();

    let managed_settings = crate::config::load_managed_settings();
    let policy_inspection = matches!(cli.command.as_ref(), Some(Command::Policy { .. }));
    if !policy_inspection {
        enforce_managed_startup_policy(managed_settings.as_ref())?;
    }

    if managed_settings
        .as_ref()
        .is_some_and(|m| m.disable_plugin_dirs)
        && !cli.plugin_dir.is_empty()
    {
        tracing::warn!(
            target: "jfc::plugins",
            "--plugin-dir ignored by managed settings"
        );
    } else {
        for dir in &cli.plugin_dir {
            crate::workflows::registry::register_extra_plugin_dir(dir.clone());
        }
    }
    if managed_settings
        .as_ref()
        .is_some_and(|m| m.disable_plugin_urls)
        && !cli.plugin_url.is_empty()
    {
        tracing::warn!(
            target: "jfc::plugins",
            "--plugin-url ignored by managed settings"
        );
    } else {
        for url in &cli.plugin_url {
            match plugin::ensure_plugin_url(url) {
                Ok(path) => crate::workflows::registry::register_extra_plugin_dir(path),
                Err(err) => tracing::warn!(
                target: "jfc::plugins",
                url,
                error = %err,
                "--plugin-url install/register failed"
                ),
            }
        }
    }

    // One-shot startup compaction of `daemon-state.json`. Long-lived users
    // accumulate hundreds of completed background-agent records here; the
    // UI re-reads the file every second on the render thread, so an
    // unbounded roster turns into a real CPU sink (~38% steady-state on
    // a 1.4 MB file with ~500 entries). Drop anything past the retention
    // window AND anything beyond the most-recent cap. Best-effort —
    // failures are silent because compaction is a hygiene step, not a
    // correctness invariant.
    compact_terminal_agents_on_startup();

    // Subcommand dispatch must run before any TUI setup — `daemon start`
    // expects a clean stdout, and `daemon status / list / stop / fire`
    // print plain text rather than entering the alt-screen.
    if let Some(cmd) = cli.command {
        return run_subcommand(cmd).await;
    }

    let init = build_providers();
    let providers = init.providers;
    let active_idx = init.active_idx;
    // Determine startup session from CLI flags (before consuming cli fields)
    let startup_session = cli.startup_session();
    let initial_prompt = cli.prompt;
    let print_mode = cli.print_mode;

    // CLI --model overrides env var. Qualified specs (`provider/model`) also
    // reroute the active provider and are normalized before request dispatch.
    let mut model = cli.model.map(ModelId::from).unwrap_or(init.model);
    let oauth_handle = init.oauth;
    let mut provider = providers[active_idx].clone();
    if let Some(resolved) = crate::resolve_provider_model(&providers, model.as_str()) {
        provider = resolved.provider;
        model = resolved.model;
    }
    let advisor_cli = cli.advisor.clone();
    let local_advisor_model = {
        let cfg = crate::config::load_arc();
        let configured = advisor_cli
            .as_deref()
            .or_else(|| cfg.advisor_model.as_deref());
        let force = advisor_cli.is_some();
        let advisor = if cli.no_advisor {
            None
        } else {
            match crate::advisor::resolve_local_advisor_model(
                &model,
                configured,
                force,
                cfg.advisor_enabled,
            ) {
                Ok(model) => model,
                Err(e) if force => anyhow::bail!("--advisor: {e}"),
                Err(e) => {
                    tracing::warn!(target: "jfc::advisor", error = %e, "local advisor disabled");
                    None
                }
            }
        };
        crate::advisor::set_active_local_advisor_model(advisor.clone());
        if cli.no_advisor {
            tracing::info!(target: "jfc::advisor", "local advisor disabled by --no-advisor");
        } else if let Some(advisor_model) = &advisor {
            tracing::info!(
                target: "jfc::advisor",
                advisor_model = %advisor_model,
                "local advisor enabled"
            );
        } else {
            tracing::info!(
                target: "jfc::advisor",
                "local advisor disabled"
            );
        }
        advisor
    };
    let server_advisor_cli = cli.server_advisor.clone();
    let advisor_model = {
        let cfg = crate::config::load_arc();
        let configured = server_advisor_cli
            .as_deref()
            .or_else(|| cfg.server_advisor_model.as_deref());
        let force = server_advisor_cli.is_some();
        let resolved =
            crate::advisor::resolve_server_advisor_model(&model, configured, force, force);
        let mut advisor = match resolved {
            Ok(model) => model,
            Err(e) if force => anyhow::bail!("--server-advisor: {e}"),
            Err(e) => {
                tracing::warn!(target: "jfc::advisor", error = %e, "server advisor disabled");
                None
            }
        };
        let provider_supports_advisor =
            matches!(
                provider.stream_convention(),
                jfc_provider::StreamConvention::AnthropicNative
            ) && matches!(provider.name(), "anthropic" | "anthropic-oauth");
        if advisor.is_some() && !provider_supports_advisor {
            let msg = format!(
                "advisor requires an Anthropic-native provider; active provider is {}",
                provider.name()
            );
            if force {
                anyhow::bail!("--server-advisor: {msg}");
            }
            tracing::warn!(target: "jfc::advisor", %msg);
            advisor = None;
        }
        crate::advisor::set_active_server_advisor_model(advisor.clone());
        if let Some(advisor_model) = &advisor {
            tracing::info!(
                target: "jfc::advisor",
                advisor_model = %advisor_model,
                base_model = %model,
                "server advisor enabled"
            );
        }
        advisor
    };

    // Collect all flag-derived runtime config in one place so we can
    // thread it into `event_loop::run` without growing the signature
    // every time a new flag lands. `--system-prompt-file` wins when
    // both file and inline text are supplied (file is the more
    // explicit choice). A read error on the file is logged + treated
    // as "no extra prompt" — refusing to boot over a missing prompt
    // file would be a hostile failure mode for the user.
    let cli_system_prompt = match cli.system_prompt_file.as_ref() {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(s) => Some(s),
            Err(e) => {
                tracing::warn!(
                    target: "jfc::cli",
                    path = %path.display(),
                    error = %e,
                    "--system-prompt-file: read failed, ignoring"
                );
                cli.system_prompt.clone()
            }
        },
        None => cli.system_prompt.clone(),
    };
    let mut allowed_tools = cli
        .allowed_tools
        .as_deref()
        .map(parse_tool_list)
        .unwrap_or_default();
    let mut disallowed_tools = cli
        .disallowed_tools
        .as_deref()
        .map(parse_tool_list)
        .unwrap_or_default();
    let mut max_budget_usd = cli.max_budget_usd;
    let mut remote_control = cli.remote_control;
    if let Some(managed) = managed_settings.as_ref() {
        if !managed.allowed_tools.is_empty() {
            if allowed_tools.is_empty() {
                allowed_tools = managed.allowed_tools.clone();
            } else {
                let managed_allowed: std::collections::HashSet<String> = managed
                    .allowed_tools
                    .iter()
                    .map(|tool| tool.to_ascii_lowercase())
                    .collect();
                allowed_tools.retain(|tool| managed_allowed.contains(&tool.to_ascii_lowercase()));
            }
        }
        disallowed_tools.extend(managed.disallowed_tools.clone());
        dedup_tool_list(&mut allowed_tools);
        dedup_tool_list(&mut disallowed_tools);
        let managed_budget = [managed.max_budget_usd, managed.spend_limit_usd]
            .into_iter()
            .flatten()
            .reduce(f64::min);
        max_budget_usd = match (max_budget_usd, managed_budget) {
            (Some(cli_budget), Some(managed_budget)) => Some(cli_budget.min(managed_budget)),
            (None, Some(managed_budget)) => Some(managed_budget),
            (other, None) => other,
        };
        if managed.disable_remote_control {
            remote_control = false;
        }
    }

    let runtime_config = CliRuntimeConfig {
        max_turns: cli.max_turns,
        max_budget_usd,
        allowed_tools,
        disallowed_tools,
        system_prompt: cli_system_prompt,
        dangerously_skip_permissions: cli.dangerously_skip_permissions
            && !managed_forces_non_bypass(managed_settings.as_ref()),
        json_mode: cli.json,
        extra_dirs: cli.add_dir.clone(),
        max_thinking_tokens: cli.max_thinking_tokens,
        thinking_display: cli.thinking_display.clone(),
        no_session_persistence: cli.no_session_persistence,
        task_budget: cli.task_budget,
        mcp_config_path: cli.mcp_config.clone(),
        cowork: cli.cowork,
        local_advisor_model,
        server_advisor_model: advisor_model,
        custom_betas: cli.betas.clone(),
        fine_grained_tool_streaming: cli.fine_grained_tool_streaming,
        strict_tool_schemas: cli.strict_tool_schemas,
        remote_control,
    };

    // v132 `-p`/`--print` headless one-shot mode. Skips the TUI
    // entirely, streams the response to stdout, exits with the
    // model's stop_reason. Used for scripting and CI: `jfc -p
    // "summarize this PR" --print | tee out.md`. When `--print` is
    // set without `--prompt`, read the prompt from stdin.
    if print_mode {
        crate::tools::register_active_provider(provider.clone(), model.clone());
        let prompt = match initial_prompt {
            Some(p) => p,
            None => {
                use std::io::Read;
                let mut buf = String::new();
                if std::io::stdin().read_to_string(&mut buf).is_err() {
                    eprintln!("--print: no prompt provided (use --prompt or pipe stdin)");
                    std::process::exit(2);
                }
                buf
            }
        };
        let output_format = if cli.json && cli.output_format == HeadlessOutputFormat::Text {
            HeadlessOutputFormat::Json
        } else {
            cli.output_format
        };
        return run_print_mode(
            provider,
            model,
            prompt,
            PrintModeConfig {
                output_format,
                input_format: cli.input_format,
                include_hook_events: cli.include_hook_events,
                include_partial_messages: cli.include_partial_messages,
                session_mirror: cli.session_mirror.clone(),
                permission_prompt_tool: cli.permission_prompt_tool.clone(),
                sdk_url: cli.sdk_url.clone(),
                custom_betas: cli.betas.clone(),
                fine_grained_tool_streaming: cli.fine_grained_tool_streaming,
                strict_tool_schemas: cli.strict_tool_schemas,
                max_turns: cli.max_turns,
            },
        )
        .await;
    }

    // v132 `--remote-session <id>` — connect to a managed-agent
    // session via the SDK. Pre-flight check the SDK client now so
    // we fail fast on missing API key instead of after entering
    // the alt-screen.
    if let Some(remote_id) = cli.remote_session.clone() {
        let Some(sdk_client) = crate::sdk_bridge::build_client() else {
            eprintln!(
                "--remote-session: no Anthropic API key found (set ANTHROPIC_API_KEY \
                 or configure a profile via .jfc/account.toml)"
            );
            std::process::exit(2);
        };
        return run_remote_session(sdk_client, remote_id).await;
    }

    // Register the active provider with the tools layer so the
    // agent-economy auto_dispatch path can spawn real solver +
    // validator subagent LLM calls without needing a wider
    // signature change. Safe to call multiple times — model swaps
    // overwrite the registered handle.
    crate::tools::register_active_provider(provider.clone(), model.clone());

    install_terminal_panic_hook();
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        EnableBracketedPaste
    )?;
    let kbd_enhanced = enable_keyboard_enhancement(&mut stdout);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let initial_permission_mode = managed_settings
        .as_ref()
        .and_then(|managed| parse_permission_mode(managed.force_permission_mode.as_deref()))
        .or_else(|| parse_permission_mode(cli.permission_mode.as_deref()))
        .or_else(|| {
            // If no --permission-mode flag was passed, read the persisted
            // mode from config.toml [default.permission].mode so the user's
            // `/mode` choice survives across sessions.
            let cfg = crate::config::load_arc();
            cfg.default
                .permission
                .get("mode")
                .and_then(|s| parse_permission_mode(Some(s.as_str())))
        });

    let result = crate::runtime::event_loop::run(
        &mut terminal,
        providers,
        provider,
        model,
        oauth_handle,
        startup_session,
        initial_prompt,
        initial_permission_mode,
        runtime_config,
    )
    .await;

    if kbd_enhanced {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture,
        DisableBracketedPaste
    )?;
    terminal.show_cursor()?;

    result
}

/// Dispatch `jfc <subcommand>`. Pure CLI — no terminal raw-mode, no TUI.
async fn run_subcommand(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Daemon { sub } => run_daemon_subcommand(sub).await,
        Command::Auth { sub } => run_auth_subcommand(sub).await,
        Command::Rc { sub } => run_rc_subcommand(sub).await,
        Command::Changes { sub } => run_changes_subcommand(sub).await,
        Command::Plugin { sub } => run_plugin_subcommand(sub).await,
        Command::Policy { sub } => run_policy_subcommand(sub).await,
        Command::Memory { sub } => run_memory_subcommand(sub).await,
        Command::Bridge { sub } => run_bridge_subcommand(sub).await,
    }
}

fn enforce_managed_startup_policy(
    managed: Option<&crate::config::ManagedSettingsConfig>,
) -> anyhow::Result<()> {
    let Some(managed) = managed else {
        return Ok(());
    };
    if let Some(notice) = managed.security_notice.as_deref().filter(|s| !s.is_empty()) {
        eprintln!("managed policy notice: {notice}");
    }
    if let Some(required_user) = managed.required_user.as_deref().filter(|s| !s.is_empty()) {
        let current_user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .or_else(|_| std::env::var("USERNAME"))
            .unwrap_or_default();
        if current_user != required_user {
            anyhow::bail!(
                "managed policy requires user '{required_user}', current user is '{}'",
                if current_user.is_empty() {
                    "(unknown)"
                } else {
                    current_user.as_str()
                }
            );
        }
    }
    for requirement in &managed.required_env {
        let requirement = requirement.trim();
        if requirement.is_empty() {
            continue;
        }
        if let Some((key, expected)) = requirement.split_once('=') {
            let actual = std::env::var(key).unwrap_or_default();
            if actual != expected {
                anyhow::bail!("managed policy requires environment {key}={expected}");
            }
        } else if std::env::var_os(requirement).is_none() {
            anyhow::bail!("managed policy requires environment variable {requirement}");
        }
    }
    if managed.require_elevated_auth
        && !std::env::var("JFC_ELEVATED_AUTH")
            .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
            .unwrap_or(false)
    {
        anyhow::bail!("managed policy requires elevated auth (set JFC_ELEVATED_AUTH=1)");
    }
    if managed.require_oauth && !anthropic_oauth_available() {
        anyhow::bail!("managed policy requires an Anthropic OAuth account");
    }
    Ok(())
}

fn anthropic_oauth_available() -> bool {
    if std::env::var_os("ANTHROPIC_AUTH_TOKEN").is_some()
        || std::env::var_os("ANTHROPIC_OAUTH_TOKEN").is_some()
    {
        return true;
    }
    let path = crate::providers::anthropic_oauth::default_store_path();
    let Ok(raw) = std::fs::read_to_string(path) else {
        return false;
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return false;
    };
    value
        .get("accounts")
        .and_then(|v| v.as_array())
        .is_some_and(|accounts| {
            accounts.iter().any(|account| {
                account
                    .get("enabled")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(true)
                    && account
                        .get("refresh_token")
                        .and_then(|v| v.as_str())
                        .is_some_and(|token| !token.is_empty())
            })
        })
}

#[cfg(test)]
mod cli_tests {
    use super::*;

    // Normal: each canonical permission-mode label round-trips through
    // the parser to the matching enum variant.
    #[test]
    fn parse_permission_mode_canonical_labels_normal() {
        assert_eq!(
            parse_permission_mode(Some("default")),
            Some(crate::app::PermissionMode::Default)
        );
        assert_eq!(
            parse_permission_mode(Some("plan")),
            Some(crate::app::PermissionMode::Plan)
        );
        assert_eq!(
            parse_permission_mode(Some("accept-edits")),
            Some(crate::app::PermissionMode::AcceptEdits)
        );
        assert_eq!(
            parse_permission_mode(Some("bypass")),
            Some(crate::app::PermissionMode::BypassPermissions)
        );
        assert_eq!(
            parse_permission_mode(Some("auto")),
            Some(crate::app::PermissionMode::Auto)
        );
    }

    // Robust: hyphen-stripped + Claude Code v141 spellings + ambient
    // casing all map to the same enum variant.
    #[test]
    fn parse_permission_mode_accepts_alternate_spellings_robust() {
        assert_eq!(
            parse_permission_mode(Some("acceptEdits")),
            Some(crate::app::PermissionMode::AcceptEdits)
        );
        assert_eq!(
            parse_permission_mode(Some("bypassPermissions")),
            Some(crate::app::PermissionMode::BypassPermissions)
        );
        assert_eq!(
            parse_permission_mode(Some("ACCEPT_EDITS")),
            Some(crate::app::PermissionMode::AcceptEdits)
        );
        assert_eq!(
            parse_permission_mode(Some("  Plan  ")),
            Some(crate::app::PermissionMode::Plan)
        );
    }

    // Robust: unknown / empty / None values all return None so boot
    // falls through to the default mode rather than panicking.
    #[test]
    fn parse_permission_mode_returns_none_for_invalid_robust() {
        assert!(parse_permission_mode(None).is_none());
        assert!(parse_permission_mode(Some("")).is_none());
        assert!(parse_permission_mode(Some("   ")).is_none());
        assert!(parse_permission_mode(Some("not-a-real-mode")).is_none());
    }
}
