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
mod daemon;
mod headless;
mod logging;
mod provider_bootstrap;
mod rc;
mod terminal;

pub(crate) use provider_bootstrap::{build_providers, provider_for_model};

use auth::{AuthSubcommand, run_auth_subcommand};
use daemon::{DaemonSubcommand, compact_terminal_agents_on_startup, run_daemon_subcommand};
use headless::{run_print_mode, run_remote_session};
use jfc_provider::ModelId;
use logging::init_tracing;
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

    /// Connect to a remote managed-agent session by ID. Streams the
    /// session's events into the TUI transcript and forwards user
    /// input via the Sessions API. Pairs with the SDK's
    /// `managed_session::ManagedSession`.
    #[arg(long = "remote-session", value_name = "SESSION_ID")]
    remote_session: Option<String>,

    /// Model to use (overrides ANTHROPIC_MODEL env var)
    #[arg(long, short = 'm', value_name = "MODEL")]
    model: Option<String>,

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
    #[arg(long = "allowed-tools", value_name = "TOOLS")]
    allowed_tools: Option<String>,

    /// Comma-separated denylist of tool names. Names in this list are
    /// rejected regardless of the allowlist.
    #[arg(long = "disallowed-tools", value_name = "TOOLS")]
    disallowed_tools: Option<String>,

    /// Extra system-prompt text appended after the built-in prompt.
    /// Use for project-specific style, persona, or constraint
    /// reminders. Mutually overrideable with `--system-prompt-file`
    /// (file wins if both are set).
    #[arg(long = "system-prompt", value_name = "TEXT")]
    system_prompt: Option<String>,

    /// Read additional system-prompt text from the given file. Takes
    /// precedence over `--system-prompt` when both are supplied.
    #[arg(long = "system-prompt-file", value_name = "PATH")]
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

    // CLI --model overrides env var
    let model = cli.model.map(ModelId::from).unwrap_or(init.model);
    let oauth_handle = init.oauth;
    let provider = providers[active_idx].clone();

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
    let runtime_config = CliRuntimeConfig {
        max_turns: cli.max_turns,
        max_budget_usd: cli.max_budget_usd,
        allowed_tools: cli
            .allowed_tools
            .as_deref()
            .map(parse_tool_list)
            .unwrap_or_default(),
        disallowed_tools: cli
            .disallowed_tools
            .as_deref()
            .map(parse_tool_list)
            .unwrap_or_default(),
        system_prompt: cli_system_prompt,
        dangerously_skip_permissions: cli.dangerously_skip_permissions,
        json_mode: cli.json,
        extra_dirs: cli.add_dir.clone(),
        max_thinking_tokens: cli.max_thinking_tokens,
        thinking_display: cli.thinking_display.clone(),
        no_session_persistence: cli.no_session_persistence,
        task_budget: cli.task_budget,
        mcp_config_path: cli.mcp_config.clone(),
        cowork: cli.cowork,
    };

    // v132 `-p`/`--print` headless one-shot mode. Skips the TUI
    // entirely, streams the response to stdout, exits with the
    // model's stop_reason. Used for scripting and CI: `jfc -p
    // "summarize this PR" --print | tee out.md`. When `--print` is
    // set without `--prompt`, read the prompt from stdin.
    if print_mode {
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
        return run_print_mode(provider, model, prompt).await;
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

    let initial_permission_mode =
        parse_permission_mode(cli.permission_mode.as_deref()).or_else(|| {
            // If no --permission-mode flag was passed, read the persisted
            // mode from config.toml [default.permission].mode so the user's
            // `/mode` choice survives across sessions.
            let cfg = crate::config::load();
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
    }
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
