#![allow(dead_code, unused_imports, unused_mut, unused_variables)]
#![allow(clippy::all)]

use std::io;

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
mod terminal;

pub(crate) use provider_bootstrap::{build_providers, provider_for_model};

use crate::provider::ModelId;
use auth::{AuthSubcommand, run_auth_subcommand};
use daemon::{DaemonSubcommand, compact_terminal_agents_on_startup, run_daemon_subcommand};
use headless::{run_print_mode, run_remote_session};
use logging::init_tracing;
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

/// Session to load at startup based on CLI args
pub(crate) enum StartupSession {
    /// No session to load — start fresh
    Fresh,
    /// Continue most recent session
    Continue,
    /// Resume specific session by ID
    Resume(String),
}

impl Cli {
    fn startup_session(&self) -> StartupSession {
        if let Some(ref id) = self.resume {
            StartupSession::Resume(id.clone())
        } else if self.continue_session {
            StartupSession::Continue
        } else {
            StartupSession::Fresh
        }
    }
}

pub(crate) async fn run(cli: Cli) -> anyhow::Result<()> {
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

    let initial_permission_mode = parse_permission_mode(cli.permission_mode.as_deref());

    let result = crate::event_loop::run(
        &mut terminal,
        providers,
        provider,
        model,
        oauth_handle,
        startup_session,
        initial_prompt,
        initial_permission_mode,
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
