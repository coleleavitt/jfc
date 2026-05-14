mod advisor;
mod agents;
mod app;

mod attachments;
mod auto_mode;
mod bash_processes;
mod compact;
mod config;
mod context;
mod cost;
mod credential_vault;
mod diagnostics;
mod diagnostics_producer;
mod effort;
mod env_context;
mod event_loop;
mod feature_gates;
mod file_watcher;
mod fleet_view;
mod git_context;
mod github;
mod idle_prefetch;
mod ids;
mod inline_tools;
mod input;
mod keybindings;
mod lsp_client;
mod lsp_rpc;
mod managed_session;
mod markdown;
mod mcp;
mod memory;
mod memory_recall;
mod mentions;
mod message_view;
mod notifications;
mod output_style;
mod plan_cache;
mod provider;
mod providers;
mod push_notifications;
mod query;
mod render;
mod render_cache;
mod scheduler;
mod sdk_bridge;
mod session;
mod session_naming;
mod slash_commands;
mod slate;
mod speculation;
mod spinner;
mod stream;
mod swarm;
mod system_reminder;
mod tasks;
mod telemetry;
mod theme;
mod toast;
mod tools;
mod types;
mod web_cache;
mod web_search;
mod workflows;
mod worktrees;

#[cfg(feature = "background-agents")]
mod background;
mod daemon;
mod hallucination_guard;
#[cfg(feature = "hashline")]
mod hashline;
#[cfg(feature = "hooks")]
mod hooks;
#[cfg(feature = "intent-gate")]
mod intent;
#[cfg(feature = "permission-automation")]
mod permissions;
#[cfg(feature = "landlock-sandbox")]
mod sandbox;

use std::{io, path::PathBuf, sync::Arc, time::Duration};

use clap::{Parser, Subcommand};
use crossterm::{
    event::{
        DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};

use provider::{ModelId, ModelSpec, Provider};
use providers::{
    AnthropicOAuthProvider, AnthropicProvider, BedrockProvider, CodexOAuthProvider,
    LiteLLMProvider, OpenAIProvider, OpenWebUIProvider, VertexProvider,
};

/// JFC - A TUI assistant for code exploration and development
#[derive(Parser, Debug)]
#[command(name = "jfc", version, about)]
struct Cli {
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

#[derive(Subcommand, Debug)]
enum AuthSubcommand {
    /// Anthropic-specific account commands.
    Anthropic {
        #[command(subcommand)]
        sub: AnthropicAuthSubcommand,
    },
    /// OpenAI Codex / ChatGPT OAuth commands.
    Codex {
        #[command(subcommand)]
        sub: CodexAuthSubcommand,
    },
    /// LiteLLM proxy instance credentials.
    Litellm {
        #[command(subcommand)]
        sub: LiteLLMAuthSubcommand,
    },
    /// OpenWebUI account commands (Shibboleth + Duo OIDC, manual JWT, etc.).
    Openwebui {
        #[command(subcommand)]
        sub: OpenWebUIAuthSubcommand,
    },
}

#[derive(Subcommand, Debug)]
enum OpenWebUIAuthSubcommand {
    /// Automated OIDC login (Shibboleth + Duo 2FA). Requires OWUI_USERNAME +
    /// OWUI_PASSWORD env vars; OWUI_DUO_PASSCODE is optional (uses push if unset).
    Login {
        /// OpenWebUI base URL (default: $OWUI_BASE_URL or https://chat.ai2s.org).
        base_url: Option<String>,
    },
    /// Add an account by manually pasting a JWT.
    Add {
        /// OpenWebUI base URL.
        base_url: String,
        /// JWT cookie value (3-segment).
        token: String,
    },
    /// List configured accounts.
    List,
    /// Switch to a different account.
    Use {
        /// Account name (e.g. user@example.com@chat.example.com).
        name: String,
    },
    /// Remove an account.
    Remove {
        /// Account name.
        name: String,
    },
    /// List models accessible to the active account.
    Models,
    /// Verify the active account's token + show user identity.
    Whoami,
    /// Show OpenWebUI instance config (name, version, features).
    Config,
}

#[derive(Subcommand, Debug)]
enum AnthropicAuthSubcommand {
    /// Add a new account via the PKCE OAuth flow. Opens a browser-pasteable
    /// URL. By default jfc waits for a localhost callback; `--manual` falls
    /// back to the older paste-the-`code#state` flow.
    Login {
        /// Optional local alias. If omitted, jfc derives the canonical
        /// account identity from the OAuth profile automatically.
        name: Option<String>,
        /// Use the manual callback-page paste flow instead of the localhost callback flow.
        #[arg(long)]
        manual: bool,
    },
    /// List configured accounts with tier, runtime status, and active marker.
    List,
    /// Print the active account's name (the one that would be picked first).
    Active,
    /// Switch which account is preferred for the next request. Rotation may
    /// still bypass this if the picked account is rate-limited.
    Switch {
        /// Account name to mark active.
        name: String,
    },
    /// Disable an account so the rotation manager skips it permanently
    /// until re-enabled (e.g., after re-login).
    Disable {
        /// Account name to disable.
        name: String,
        /// Optional reason recorded in the store.
        #[arg(long)]
        reason: Option<String>,
    },
    /// Remove an account entirely from the store. The refresh token on disk
    /// is wiped before deletion.
    Remove {
        /// Account name to remove.
        name: String,
    },
}

#[derive(Subcommand, Debug)]
enum CodexAuthSubcommand {
    /// Print the browser URL for ChatGPT/Codex OAuth login.
    Login,
    /// Start a device-code login and print the one-time code.
    Device,
    /// Show configured Codex OAuth token status.
    Status,
    /// Remove stored Codex OAuth tokens.
    Logout,
}

#[derive(Subcommand, Debug)]
enum LiteLLMAuthSubcommand {
    /// Configure a LiteLLM proxy instance (API key + base URL).
    Login {
        /// Base URL of the LiteLLM proxy (e.g. https://api.example.com/v1).
        #[arg(long)]
        url: String,
        /// API key for authentication.
        #[arg(long)]
        key: String,
    },
    /// Show configured LiteLLM credentials.
    Status,
    /// Remove stored LiteLLM credentials.
    Logout,
}

#[derive(Subcommand, Debug)]
enum DaemonSubcommand {
    /// Run the daemon in the foreground (cron + wakeup poll loop).
    /// Use `&` or `nohup` from the shell to background it.
    Start,
    /// Stop the running daemon (SIGTERM the PID file).
    Stop,
    /// Print daemon health + session/cron/wakeup counts.
    Status,
    /// List registered cron jobs and pending wakeups.
    List,
    /// Manually fire a cron job by ID once.
    Fire {
        /// Cron job ID returned by `daemon list` (e.g. `cron-abcd1234`).
        id: String,
    },
    /// List persistent background-agent roster.
    Agents,
    /// Print recent log lines for one background agent.
    Logs {
        /// Background agent / Task id.
        id: String,
        /// Number of log lines to show.
        #[arg(long, default_value_t = 80)]
        lines: usize,
    },
    /// Follow a background agent log until it reaches a terminal state.
    Attach {
        /// Background agent / Task id.
        id: String,
        /// Number of existing log lines to print before following.
        #[arg(long, default_value_t = 80)]
        lines: usize,
    },
    /// Wait until a background agent reaches a terminal state.
    Wait {
        /// Background agent / Task id.
        id: String,
        /// Maximum seconds to wait before returning current status.
        #[arg(long, default_value_t = 300)]
        timeout_secs: u64,
    },
    /// Request cancellation for a background agent.
    Kill {
        /// Background agent / Task id.
        id: String,
    },
    /// Internal worker entrypoint for durable background agents.
    #[command(hide = true)]
    Worker {
        /// Launch spec written by the parent process.
        #[arg(long)]
        launch: PathBuf,
    },
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

#[cfg(feature = "dhat-heap")]
#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

#[tokio::main(worker_threads = 4)]
async fn main() -> anyhow::Result<()> {
    #[cfg(feature = "dhat-heap")]
    let _dhat_profiler = dhat::Profiler::new_heap();

    let cli = Cli::parse();

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
    {
        let paths = crate::daemon::DaemonPaths::default_user();
        if let Some(mut state) = crate::daemon::load_state(&paths) {
            let dropped = crate::daemon::compact_background_agents(
                &mut state,
                std::time::SystemTime::now(),
                crate::daemon::TERMINAL_AGENT_RETENTION,
                crate::daemon::TERMINAL_AGENT_CAP,
            );
            if dropped > 0 {
                if let Err(err) = crate::daemon::save_state(&paths, &state) {
                    tracing::warn!(
                        target: "jfc::daemon",
                        error = %err,
                        dropped,
                        "compact_background_agents: save_state failed"
                    );
                } else {
                    tracing::info!(
                        target: "jfc::daemon",
                        dropped,
                        retained = state.background_agents.len(),
                        "compacted terminal background-agent records on startup"
                    );
                }
            }
        }
    }

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

    let result = event_loop::run(
        &mut terminal,
        providers,
        provider,
        model,
        oauth_handle,
        startup_session,
        initial_prompt,
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
/// v132 `--print` headless mode entry. Builds a minimal stream against
/// the active provider, prints text deltas to stdout as they arrive,
/// exits with the stream's stop_reason. No TUI, no session save, no
/// tool dispatch (tools require user approval which is meaningless in
/// headless mode — callers needing tools should drive the TUI).
async fn run_print_mode(
    provider: std::sync::Arc<dyn provider::Provider>,
    model: provider::ModelId,
    prompt: String,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    use provider::{ProviderContent, ProviderMessage, ProviderRole, StreamEvent, StreamOptions};
    use std::io::Write;

    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(prompt)],
    }];
    let opts = StreamOptions::new(model.clone()).max_tokens(8192);
    let mut stream = provider
        .stream(messages, &opts)
        .await
        .map_err(|e| anyhow::anyhow!("stream open failed: {e}"))?;
    let mut stdout = std::io::stdout().lock();
    let mut exit_code = 0;
    while let Some(event) = stream.next().await {
        match event {
            Ok(StreamEvent::TextDelta { delta, .. }) => {
                let _ = stdout.write_all(delta.as_bytes());
                let _ = stdout.flush();
            }
            Ok(StreamEvent::Done { .. }) => break,
            Ok(_) => {}
            Err(e) => {
                eprintln!("\n[stream error: {e}]");
                exit_code = 1;
                break;
            }
        }
    }
    let _ = stdout.write_all(b"\n");
    let _ = stdout.flush();
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
    Ok(())
}

/// `--remote-session <id>` entry. Streams events from a managed-agent
/// session to stdout. Minimal first cut — full TUI integration with
/// rendering of v132's 17 event types lives in `managed_session.rs`
/// and ships behind a follow-on flag once the eventer is verified.
async fn run_remote_session(
    client: jfc_anthropic_sdk::Client,
    session_id: String,
) -> anyhow::Result<()> {
    use futures::StreamExt;
    let session = crate::managed_session::ManagedSession::new(client, session_id.clone());
    eprintln!("--remote-session: subscribing to session {session_id}");
    let mut stream = session
        .connect()
        .await
        .map_err(|e| anyhow::anyhow!("session connect: {e}"))?;
    while let Some(event) = stream.next().await {
        match event {
            Ok(ev) => {
                println!("{}", crate::managed_session::render_event_line(&ev));
            }
            Err(e) => {
                eprintln!("[stream error: {e}]");
                break;
            }
        }
    }
    Ok(())
}

async fn run_subcommand(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Daemon { sub } => run_daemon_subcommand(sub).await,
        Command::Auth { sub } => run_auth_subcommand(sub).await,
    }
}

async fn run_auth_subcommand(sub: AuthSubcommand) -> anyhow::Result<()> {
    match sub {
        AuthSubcommand::Anthropic { sub } => run_anthropic_auth_subcommand(sub).await,
        AuthSubcommand::Codex { sub } => run_codex_auth_subcommand(sub).await,
        AuthSubcommand::Litellm { sub } => run_litellm_auth_subcommand(sub).await,
        AuthSubcommand::Openwebui { sub } => run_openwebui_auth_subcommand(sub).await,
    }
}

async fn run_openwebui_auth_subcommand(sub: OpenWebUIAuthSubcommand) -> anyhow::Result<()> {
    use crate::providers::openwebui::{
        default_store_path, fetch_instance_config, get_current, list_accounts, load_store,
        normalize_base_url, oidc_login, parse_jwt_claims, remove_account, set_current,
        upsert_account, verify_token, Account, DuoMethod, OidcLoginOptions,
    };

    let store_path = default_store_path();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    let default_base = std::env::var("OWUI_BASE_URL")
        .unwrap_or_else(|_| "https://chat.ai2s.org".to_owned());

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    match sub {
        OpenWebUIAuthSubcommand::Login { base_url } => {
            let base = normalize_base_url(&base_url.unwrap_or(default_base))?;
            let username = std::env::var("OWUI_USERNAME")
                .map_err(|_| anyhow::anyhow!("OWUI_USERNAME env var required"))?;
            let password = std::env::var("OWUI_PASSWORD")
                .map_err(|_| anyhow::anyhow!("OWUI_PASSWORD env var required"))?;
            let passcode = std::env::var("OWUI_DUO_PASSCODE").ok();
            let method = if passcode.is_some() {
                DuoMethod::Passcode
            } else {
                DuoMethod::Push
            };

            println!("→ logging in to {base} as {username}...");
            if matches!(method, DuoMethod::Push) {
                println!("→ no OWUI_DUO_PASSCODE set — sending Duo Push (approve on your phone)");
            } else {
                println!("→ using OWUI_DUO_PASSCODE for 2FA");
            }

            let mut opts = OidcLoginOptions::new(&base, &username, &password);
            opts.duo_passcode = passcode;
            opts.duo_method = method;
            let result = oidc_login(opts).await?;

            let user = verify_token(&client, &base, &result.token).await?;
            let cfg = fetch_instance_config(&client, &base).await.ok();
            let host = url::Url::parse(&base)?.host_str().unwrap_or("").to_owned();
            let name = format!("{}@{}", user.email, host);
            let now = now_ms();

            upsert_account(
                &store_path,
                Account {
                    name: name.clone(),
                    base_url: base.clone(),
                    token: result.token,
                    expires_at: Some(result.expires_at),
                    created_at: Some(now),
                    updated_at: Some(now),
                    ..Default::default()
                },
            )?;

            println!("\n✓ logged in as {} <{}> ({})", user.name, user.email, user.role);
            if let Some(c) = cfg {
                println!("  instance: {} v{}", c.name, c.version);
            }
            let exp = chrono::DateTime::<chrono::Utc>::from_timestamp_millis(result.expires_at)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "?".into());
            println!("  token expires: {exp}");
            println!("  account stored as: {name}");
        }
        OpenWebUIAuthSubcommand::Add { base_url, token } => {
            let base = normalize_base_url(&base_url)?;
            let claims = parse_jwt_claims(&token)
                .ok_or_else(|| anyhow::anyhow!("token does not decode as a JWT"))?;
            let user = verify_token(&client, &base, &token).await?;
            let cfg = fetch_instance_config(&client, &base).await.ok();
            let host = url::Url::parse(&base)?.host_str().unwrap_or("").to_owned();
            let name = format!("{}@{}", user.email, host);
            let now = now_ms();
            upsert_account(
                &store_path,
                Account {
                    name: name.clone(),
                    base_url: base,
                    token,
                    expires_at: Some(claims.exp * 1000),
                    created_at: Some(now),
                    updated_at: Some(now),
                    ..Default::default()
                },
            )?;
            let exp = chrono::DateTime::<chrono::Utc>::from_timestamp(claims.exp, 0)
                .map(|d| d.to_rfc3339())
                .unwrap_or_else(|| "?".into());
            println!(
                "✓ added {name} (instance={} v{}, expires={exp})",
                cfg.as_ref().map(|c| c.name.as_str()).unwrap_or("unknown"),
                cfg.as_ref().map(|c| c.version.as_str()).unwrap_or("?")
            );
        }
        OpenWebUIAuthSubcommand::List => {
            let store = load_store(&store_path);
            let current = store.current.clone();
            let accounts = list_accounts(&store);
            if accounts.is_empty() {
                println!("(no accounts)");
            } else {
                for a in accounts {
                    let star = if Some(&a.name) == current.as_ref() { "*" } else { " " };
                    let exp = a
                        .expires_at
                        .and_then(chrono::DateTime::<chrono::Utc>::from_timestamp_millis)
                        .map(|d| d.to_rfc3339())
                        .unwrap_or_else(|| "?".into());
                    println!("{star} {:48}  {}  expires={exp}", a.name, a.base_url);
                }
            }
        }
        OpenWebUIAuthSubcommand::Use { name } => {
            if !set_current(&store_path, &name)? {
                anyhow::bail!("no account named {name}");
            }
            println!("current → {name}");
        }
        OpenWebUIAuthSubcommand::Remove { name } => {
            remove_account(&store_path, &name)?;
            println!("removed {name}");
        }
        OpenWebUIAuthSubcommand::Models => {
            let store = load_store(&store_path);
            let account = get_current(&store).ok_or_else(|| anyhow::anyhow!("no current account"))?;
            let res: serde_json::Value = client
                .get(format!("{}/api/models", account.base_url.trim_end_matches('/')))
                .header("Authorization", format!("Bearer {}", account.token))
                .header("Accept", "application/json")
                .send()
                .await?
                .error_for_status()?
                .json()
                .await?;
            if let Some(arr) = res.get("data").and_then(|v| v.as_array()) {
                for m in arr {
                    let id = m.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                    let name = m.get("name").and_then(|v| v.as_str()).unwrap_or("");
                    println!("{id:48}  {name}");
                }
                println!("\n{} model(s) accessible to {}", arr.len(), account.name);
            }
        }
        OpenWebUIAuthSubcommand::Whoami => {
            let store = load_store(&store_path);
            let account = get_current(&store).ok_or_else(|| anyhow::anyhow!("no current account"))?;
            let user = verify_token(&client, &account.base_url, &account.token).await?;
            println!(
                "{}\n  {} {} <{}>",
                account.name, user.role, user.id, user.email
            );
        }
        OpenWebUIAuthSubcommand::Config => {
            let store = load_store(&store_path);
            let base = get_current(&store).map(|a| a.base_url).unwrap_or(default_base);
            let cfg = fetch_instance_config(&client, &base).await?;
            println!("instance:  {} v{}", cfg.name, cfg.version);
            println!("baseUrl:   {base}");
            println!("status:    {}", if cfg.status { "online" } else { "offline" });
            let enabled: Vec<&String> = cfg
                .features
                .iter()
                .filter(|(_, v)| v.as_bool().unwrap_or(false))
                .map(|(k, _)| k)
                .collect();
            if enabled.is_empty() {
                println!("features:  (none enabled)");
            } else {
                let mut joined: Vec<&str> = enabled.iter().map(|s| s.as_str()).collect();
                joined.sort();
                println!("features:  {}", joined.join(", "));
            }
        }
    }
    Ok(())
}

async fn run_codex_auth_subcommand(sub: CodexAuthSubcommand) -> anyhow::Result<()> {
    use crate::providers::codex_oauth::CodexOAuthProvider;
    use crate::providers::oauth_core::TokenStore;

    let provider = CodexOAuthProvider::new();
    match sub {
        CodexAuthSubcommand::Login => {
            let redirect_uri = "http://localhost:1455/auth/callback";
            let req = CodexOAuthProvider::authorize_url(redirect_uri);
            println!();
            println!("=== OpenAI Codex OAuth login ===");
            println!();
            println!("Open this URL in a browser:");
            println!();
            println!("   {}", req.url);
            println!();
            println!(
                "After approving, capture the callback code and exchange it through the Codex OAuth flow."
            );
            println!("Device-code flow is also available with: jfc auth codex device");
            println!("store: {}", provider.store_path().display());
            Ok(())
        }
        CodexAuthSubcommand::Device => {
            let code = provider.request_device_code().await?;
            println!();
            println!("=== OpenAI Codex device login ===");
            println!();
            println!("Open: {}", code.verification_url);
            println!("Code: {}", code.user_code);
            println!();
            println!("Waiting for authorization...");
            provider.poll_device_code(&code).await?;
            println!(
                "✓ Codex OAuth tokens stored at {}",
                provider.store_path().display()
            );
            Ok(())
        }
        CodexAuthSubcommand::Status => {
            let store = TokenStore::new(TokenStore::default_path());
            match store.get("codex")? {
                Some(crate::providers::oauth_core::AuthMethod::OAuth {
                    expires_at,
                    account_id,
                    ..
                }) => {
                    println!("codex: configured");
                    println!("account: {}", account_id.as_deref().unwrap_or("(unknown)"));
                    println!("expires_at: {expires_at}");
                }
                _ => println!(
                    "codex: not configured (run `jfc auth codex login` or `jfc auth codex device`)"
                ),
            }
            Ok(())
        }
        CodexAuthSubcommand::Logout => {
            let store = TokenStore::new(TokenStore::default_path());
            if store.remove("codex")? {
                println!("removed Codex OAuth tokens from {}", store.path().display());
            } else {
                println!("no Codex OAuth tokens found in {}", store.path().display());
            }
            Ok(())
        }
    }
}

async fn run_litellm_auth_subcommand(sub: LiteLLMAuthSubcommand) -> anyhow::Result<()> {
    use crate::providers::litellm;

    let cred_path = litellm::credentials_path();
    match sub {
        LiteLLMAuthSubcommand::Login { url, key } => {
            litellm::save_credentials(&url, &key)?;
            println!("✓ LiteLLM credentials saved to {}", cred_path.display());

            let client = reqwest::Client::new();
            let base = url.trim_end_matches('/');
            match client
                .get(format!("{base}/models"))
                .header("Authorization", format!("Bearer {key}"))
                .timeout(std::time::Duration::from_secs(8))
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => {
                    println!("✓ Connection verified — instance is reachable");
                }
                Ok(resp) => {
                    println!(
                        "⚠ Instance returned HTTP {} — credentials may be invalid",
                        resp.status()
                    );
                }
                Err(e) => {
                    println!("⚠ Could not reach instance: {e}");
                    println!("  Credentials are saved; fix the URL and re-run login.");
                }
            }
            Ok(())
        }
        LiteLLMAuthSubcommand::Status => {
            match litellm::load_credentials() {
                Some(creds) => {
                    println!("litellm: configured");
                    println!("url: {}", creds.base_url);
                    println!(
                        "key: {}…{}",
                        &creds.api_key[..creds.api_key.len().min(4)],
                        &creds.api_key[creds.api_key.len().saturating_sub(4)..]
                    );
                    println!("store: {}", cred_path.display());
                }
                None => {
                    println!("litellm: not configured");
                    println!("  run: jfc auth litellm login --url <URL> --key <KEY>");
                }
            }
            Ok(())
        }
        LiteLLMAuthSubcommand::Logout => {
            if cred_path.exists() {
                std::fs::remove_file(&cred_path)?;
                println!("removed LiteLLM credentials from {}", cred_path.display());
            } else {
                println!("no LiteLLM credentials found at {}", cred_path.display());
            }
            Ok(())
        }
    }
}

async fn run_anthropic_auth_subcommand(sub: AnthropicAuthSubcommand) -> anyhow::Result<()> {
    use crate::providers::anthropic_accounts::AccountManager;
    use crate::providers::anthropic_oauth::default_store_path;
    use crate::providers::anthropic_oauth_login as login;

    let store_path = default_store_path();
    let mgr = AccountManager::load(store_path.clone()).await?;

    match sub {
        AnthropicAuthSubcommand::Login { name, manual } => {
            let requested_name = name.as_deref().unwrap_or("");
            if manual {
                let req = login::authorize();
                println!();
                println!("=== Anthropic OAuth login ===");
                println!();
                println!("1. Open this URL in a browser:");
                println!();
                println!("   {}", req.url);
                println!();
                println!("2. After approving, the callback page will show a string like:");
                println!("      <code>#<state>");
                println!("3. Paste the entire string (with the `#`) here, then press Enter.");
                println!();
                print!("code#state> ");
                use std::io::Write;
                std::io::stdout().flush().ok();

                let mut paste = String::new();
                std::io::stdin().read_line(&mut paste)?;
                let paste = paste.trim();
                if paste.is_empty() {
                    anyhow::bail!("login: no input provided");
                }

                match login::login(&mgr, requested_name, paste, &req.verifier, &req.state).await {
                    Ok(resolved_name) => {
                        println!("\n✓ logged in as '{resolved_name}'.");
                        println!("  store: {}", store_path.display());
                        Ok(())
                    }
                    Err(e) => Err(anyhow::anyhow!("login failed: {e}")),
                }
            } else {
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?;
                let port = listener.local_addr()?.port();
                let req = login::authorize_with_redirect(
                    crate::providers::anthropic_oauth_login::RedirectTarget::Localhost(port),
                );
                println!();
                println!("=== Anthropic OAuth login ===");
                println!();
                println!("Open this URL in a browser:");
                println!();
                println!("   {}", req.url);
                println!();
                println!("Waiting for callback on http://localhost:{port}/callback ...");
                println!("If that fails, rerun with: jfc auth anthropic login --manual");

                let (code, returned_state) = wait_for_oauth_callback(listener).await?;
                match login::login_with_code_and_state(
                    &mgr,
                    requested_name,
                    &code,
                    &returned_state,
                    &req.verifier,
                    &req.state,
                    &req.redirect_uri,
                )
                .await
                {
                    Ok(resolved_name) => {
                        println!("\n✓ logged in as '{resolved_name}'.");
                        println!("  store: {}", store_path.display());
                        Ok(())
                    }
                    Err(e) => Err(anyhow::anyhow!("login failed: {e}")),
                }
            }
        }
        AnthropicAuthSubcommand::List => {
            let pairs = mgr.list_with_runtime().await;
            if pairs.is_empty() {
                println!("(no accounts in {})", store_path.display());
                println!("Run `jfc auth anthropic login <name>` to add one.");
                return Ok(());
            }
            let active_name = mgr.active_account().await.map(|a| a.name);
            println!(
                "{:<20} {:<8} {:<22} {:<10} {:<14}",
                "NAME", "ACTIVE", "TIER", "ENABLED", "RUNTIME"
            );
            for (acct, rt) in pairs {
                let is_active = active_name.as_deref() == Some(acct.name.as_str());
                let active_marker = if is_active { "*" } else { "" };
                let tier = acct.rate_limit_tier.as_deref().unwrap_or("(unknown)");
                let enabled = if acct.is_enabled() { "yes" } else { "no" };
                let runtime = format_runtime_state(&acct, &rt);
                println!(
                    "{:<20} {:<8} {:<22} {:<10} {:<14}",
                    acct.name, active_marker, tier, enabled, runtime
                );
            }
            Ok(())
        }
        AnthropicAuthSubcommand::Active => match mgr.active_account().await {
            Some(a) => {
                println!("{}", a.name);
                Ok(())
            }
            None => {
                eprintln!("(no active account)");
                std::process::exit(1);
            }
        },
        AnthropicAuthSubcommand::Switch { name } => {
            if mgr.atomic_set_active(&name).await? {
                println!("active = {name}");
                Ok(())
            } else {
                Err(anyhow::anyhow!("switch: account '{name}' not found"))
            }
        }
        AnthropicAuthSubcommand::Disable { name, reason } => {
            mgr.atomic_disable_account(&name, reason.as_deref().unwrap_or("manual"))
                .await?;
            println!("disabled '{name}'");
            Ok(())
        }
        AnthropicAuthSubcommand::Remove { name } => {
            mgr.atomic_clear_refresh_token(&name).await.ok();
            if mgr.atomic_remove_account(&name).await? {
                println!("removed '{name}'");
                Ok(())
            } else {
                Err(anyhow::anyhow!("remove: account '{name}' not found"))
            }
        }
    }
}

async fn wait_for_oauth_callback(
    listener: tokio::net::TcpListener,
) -> anyhow::Result<(String, String)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let (mut socket, _) = tokio::time::timeout(Duration::from_secs(300), listener.accept())
        .await
        .map_err(|_| anyhow::anyhow!("timed out waiting for OAuth callback"))??;

    let mut buf = vec![0u8; 8192];
    let n = socket.read(&mut buf).await?;
    let req = String::from_utf8_lossy(&buf[..n]);
    let first_line = req
        .lines()
        .next()
        .ok_or_else(|| anyhow::anyhow!("malformed callback request"))?;
    let path = first_line
        .split_whitespace()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("malformed callback request line"))?;
    let url = reqwest::Url::parse(&format!("http://localhost{path}"))?;
    let code = url
        .query_pairs()
        .find_map(|(k, v)| (k == "code").then(|| v.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("callback missing code"))?;
    let state = url
        .query_pairs()
        .find_map(|(k, v)| (k == "state").then(|| v.into_owned()))
        .ok_or_else(|| anyhow::anyhow!("callback missing state"))?;

    let body =
        "<html><body><h1>Anthropic login complete</h1><p>You can return to jfc.</p></body></html>";
    let resp = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    );
    socket.write_all(resp.as_bytes()).await.ok();
    socket.shutdown().await.ok();

    Ok((code, state))
}

fn format_runtime_state(
    acct: &crate::providers::anthropic_accounts::Account,
    rt: &crate::providers::anthropic_accounts::RuntimeState,
) -> String {
    if !acct.is_disk_rate_limit_cleared() {
        return "rate-limited".into();
    }
    if !rt.cooldown_cleared() {
        return "cooldown".into();
    }
    if rt.consecutive_failures > 0 {
        return format!("fails={}", rt.consecutive_failures);
    }
    if acct.is_token_expired() {
        return "token-expired".into();
    }
    "ok".into()
}

async fn run_daemon_subcommand(sub: DaemonSubcommand) -> anyhow::Result<()> {
    use crate::daemon::{
        DaemonPaths, attach_background_agent_cli, background_agent_logs_string,
        background_agents_string, fire_cron_cli, list_string, request_background_agent_cancel,
        run_daemon, status_string, stop_daemon, wait_background_agent_cli,
    };
    let paths = DaemonPaths::default_user();

    match sub {
        DaemonSubcommand::Start => {
            run_daemon(paths)
                .await
                .map_err(|e| anyhow::anyhow!("daemon start failed: {e}"))?;
            Ok(())
        }
        DaemonSubcommand::Stop => {
            stop_daemon(&paths).map_err(|e| anyhow::anyhow!("daemon stop failed: {e}"))?;
            println!("daemon stopped");
            Ok(())
        }
        DaemonSubcommand::Status => {
            print!("{}", status_string(&paths));
            Ok(())
        }
        DaemonSubcommand::List => {
            print!("{}", list_string(&paths));
            Ok(())
        }
        DaemonSubcommand::Fire { id } => {
            let msg = fire_cron_cli(&paths, &id)
                .await
                .map_err(|e| anyhow::anyhow!("fire failed: {e}"))?;
            println!("{msg}");
            Ok(())
        }
        DaemonSubcommand::Agents => {
            print!("{}", background_agents_string(&paths));
            Ok(())
        }
        DaemonSubcommand::Logs { id, lines } => {
            print!("{}", background_agent_logs_string(&paths, &id, lines));
            Ok(())
        }
        DaemonSubcommand::Attach { id, lines } => {
            attach_background_agent_cli(&paths, &id, lines)
                .await
                .map_err(|e| anyhow::anyhow!("attach failed: {e}"))?;
            Ok(())
        }
        DaemonSubcommand::Wait { id, timeout_secs } => {
            let msg = wait_background_agent_cli(
                &paths,
                &id,
                std::time::Duration::from_secs(timeout_secs),
            )
            .await
            .map_err(|e| anyhow::anyhow!("wait failed: {e}"))?;
            print!("{msg}");
            Ok(())
        }
        DaemonSubcommand::Kill { id } => {
            request_background_agent_cancel(&paths, &id)
                .map_err(|e| anyhow::anyhow!("kill failed: {e}"))?;
            println!("cancel requested for {id}");
            Ok(())
        }
        DaemonSubcommand::Worker { launch } => crate::daemon::run_background_agent_worker(launch)
            .await
            .map_err(|e| anyhow::anyhow!("worker failed: {e}")),
    }
}

fn install_terminal_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            LeaveAlternateScreen,
            DisableMouseCapture,
            DisableBracketedPaste
        );
        previous(info);
    }));
}

/// Initialize tracing so structured logs flow to `~/.config/jfc/logs/`.
/// Returns the `WorkerGuard` from `tracing-appender::non_blocking` — caller
/// must hold it until process exit so buffered logs flush.
///
/// File routing:
/// - **Interactive UI** (`is_short_lived_cli=false`): per-session file
///   `ses_YYYYMMDD_HHMMSS.log`, with `latest.log` symlink kept in sync.
///   Each UI session is its own file so a crash trace doesn't get mixed
///   with the next run.
/// - **CLI subcommand** (`is_short_lived_cli=true`): a single shared
///   `jfc-cli.log`. Subcommands like `daemon agents`/`status`/`fire`
///   exit in milliseconds; giving each its own file would leave a
///   per-invocation empty file behind (we used to ship hundreds of
///   them — see the cleanup pass below).
///
/// On startup, also unlinks empty `ses_*.log` files older than 1 hour
/// to garbage-collect leftovers from previous buggy runs.
fn init_tracing(is_short_lived_cli: bool) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::EnvFilter;

    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    // Sweep empty `ses_*.log` files left behind by previous short-lived
    // CLI invocations or buggy launches. Only target files >1h old so a
    // live UI session that hasn't logged its first line yet stays put.
    cleanup_empty_session_logs(&log_dir);

    let log_path = if is_short_lived_cli {
        // Short-lived subcommand — share one file across all CLI calls.
        log_dir.join("jfc-cli.log")
    } else {
        // Interactive UI — own its session file.
        let now = chrono::Local::now();
        let log_filename = format!("ses_{}.log", now.format("%Y%m%d_%H%M%S"));
        log_dir.join(log_filename)
    };

    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .unwrap_or_else(|_| {
            // Fallback to /dev/null equivalent
            std::fs::OpenOptions::new()
                .write(true)
                .open(if cfg!(unix) { "/dev/null" } else { "NUL" })
                .expect("cannot open null device")
        });

    if !is_short_lived_cli {
        // Update `latest.log` symlink only for interactive sessions —
        // CLI subcommands shouldn't redirect what "latest" means.
        let latest_link = log_dir.join("latest.log");
        let _ = std::fs::remove_file(&latest_link);
        #[cfg(unix)]
        {
            if let Some(name) = log_path.file_name() {
                let _ = std::os::unix::fs::symlink(name, &latest_link);
            }
        }
        #[cfg(not(unix))]
        {
            let _ = std::fs::copy(&log_path, &latest_link);
        }
    }

    let (writer, guard) = tracing_appender::non_blocking(file);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("debug,reqwest=warn,hyper=warn,h2=warn"));

    if let Err(e) = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(writer)
        .with_ansi(false) // file output — no ANSI escapes
        .with_target(true)
        .with_file(false)
        .with_line_number(false)
        .with_thread_ids(false)
        .try_init()
    {
        // Subscriber already set (or failed). Don't silently swallow — write a
        // breadcrumb to the log dir so the user has *something* to look at when
        // logs come up empty.
        let _ = std::fs::write(
            log_dir.join("tracing-init-error.txt"),
            format!("tracing init failed: {e}\n"),
        );
    }

    tracing::info!(log_dir = %log_dir.display(), "tracing initialized");
    guard
}

/// Remove zero-byte `ses_*.log` files older than one hour.
///
/// We used to create a fresh `ses_YYYYMMDD_HHMMSS.log` file for every
/// process start, including each short-lived CLI subcommand. Most CLI
/// runs exited before writing a line, leaving the log directory full of
/// empty files (237 on one local box). This pass GC's that leftover
/// set on every startup. Best-effort: any IO error is silently ignored.
fn cleanup_empty_session_logs(log_dir: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(log_dir) else {
        return;
    };
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(3600))
        .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("ses_") || !name.ends_with(".log") {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if meta.len() != 0 {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if modified > cutoff {
            continue;
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// Push kitty keyboard enhancement flags so Ctrl+M is distinguishable from Enter
/// (and Ctrl+J / Shift+Enter from one another). Returns true if flags were pushed
/// and need to be popped on exit.
fn enable_keyboard_enhancement(stdout: &mut io::Stdout) -> bool {
    if !matches!(
        crossterm::terminal::supports_keyboard_enhancement(),
        Ok(true)
    ) {
        return false;
    }
    execute!(
        stdout,
        PushKeyboardEnhancementFlags(
            KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES
                | KeyboardEnhancementFlags::REPORT_EVENT_TYPES
        )
    )
    .is_ok()
}

/// Result of `build_providers()`. We keep a typed `Arc<AnthropicOAuthProvider>` next
/// to the trait-object list so the OAuth-specific profile fetch can run without
/// needing `Any`-style downcasting through the `Provider` trait.
pub(crate) struct ProvidersInit {
    pub(crate) providers: Vec<Arc<dyn Provider>>,
    pub(crate) active_idx: usize,
    pub(crate) model: ModelId,
    oauth: Option<Arc<AnthropicOAuthProvider>>,
}

/// Build every provider that has usable config in this environment, plus pick which one
/// should be active at startup.
///
/// Active selection mirrors the prior single-provider precedence: explicit `ANTHROPIC_API_KEY`
/// wins, then `OPENWEBUI_BASE_URL`, then OAuth.
pub(crate) fn build_providers() -> ProvidersInit {
    // Cascade for the startup model id:
    //   1. ANTHROPIC_MODEL / OPENWEBUI_MODEL env (explicit override for one run)
    //   2. ~/.config/jfc/config.toml `[default].model` (the user's persisted choice)
    //   3. recent_models[0] (last model the user picked from the UI)
    //   4. hardcoded `claude-opus-4-5` (last-resort fallback)
    //
    // The config value may be a qualified `ModelSpec` like `"openwebui/bedrock-claude-4-6-opus"`
    // or a bare model id like `"claude-opus-4-7"`. When qualified, the provider prefix
    // directly routes to the matching provider — no heuristic guessing needed.
    let env_model = std::env::var("ANTHROPIC_MODEL")
        .ok()
        .or_else(|| std::env::var("OPENWEBUI_MODEL").ok())
        .or_else(|| std::env::var("JFC_LITELLM_MODEL").ok())
        .filter(|s| !s.is_empty());
    let cfg_model = config::load().default.model.filter(|s| !s.is_empty());
    let recent_model = crate::app::load_recent_models()
        .into_iter()
        .next()
        .filter(|s| !s.is_empty());
    let resolved_raw = env_model
        .or(cfg_model)
        .or(recent_model)
        .unwrap_or_else(|| "claude-opus-4-5".to_owned());

    // Parse as ModelSpec: "provider/model" or bare "model". Lenient because
    // `resolved_raw` came from an env var / config / recent-models entry — a
    // user-typed value that might contain stray slashes we'd rather treat as
    // part of a bare id than reject. `resolved_raw` is filtered non-empty
    // above, so the only `Err` path here is the empty-string guard.
    let spec: ModelSpec = ModelSpec::parse_lenient(&resolved_raw)
        .unwrap_or_else(|_| ModelSpec::bare(resolved_raw.clone()));
    tracing::info!(
        target: "jfc::startup",
        spec = %spec,
        provider_prefix = ?spec.provider().map(|p| p.as_str()),
        model_id = %spec.model(),
        "resolved startup model spec"
    );
    let model = spec.model().clone();

    let mut providers: Vec<Arc<dyn Provider>> = Vec::new();
    let mut prefer: Option<&'static str> = None;

    // Explicit env wins: ANTHROPIC_API_KEY → API-key provider as default.
    if let Ok(api_key) = std::env::var("ANTHROPIC_API_KEY") {
        providers.push(Arc::new(AnthropicProvider::new(api_key)));
        prefer.get_or_insert("anthropic");
    }

    // OAuth before OpenWebUI: when both stores exist (e.g. user runs opencode for
    // both auths), OAuth is what the model ids in `anthropic_models` actually serve.
    // Defaulting to OpenWebUI here caused "Model not found" because the seeded
    // `claude-sonnet-4-20250514` id doesn't exist on most OpenWebUI instances.
    let oauth_inst = AnthropicOAuthProvider::new();
    let oauth_arc = if oauth_inst.has_usable_config() {
        let arc = Arc::new(oauth_inst);
        providers.push(Arc::clone(&arc) as Arc<dyn Provider>);
        prefer.get_or_insert("anthropic-oauth");
        Some(arc)
    } else {
        None
    };

    if let Some(openai) = OpenAIProvider::from_env() {
        providers.push(Arc::new(openai));
        prefer.get_or_insert("openai");
    }

    if let Some(litellm) = LiteLLMProvider::from_env() {
        providers.push(Arc::new(litellm));
        prefer.get_or_insert("litellm");
    }

    let codex = CodexOAuthProvider::new();
    if codex.has_usable_config() {
        providers.push(Arc::new(codex));
        prefer.get_or_insert("codex");
    }

    // OpenWebUI is registered as a candidate so its models show up in the picker, but
    // it only becomes the *default* when the user explicitly opts in via OPENWEBUI_BASE_URL.
    let openwebui = OpenWebUIProvider::new();
    let has_openwebui_config = openwebui.has_usable_config();
    if has_openwebui_config {
        providers.push(Arc::new(openwebui));
        if std::env::var("OPENWEBUI_BASE_URL").is_ok() {
            prefer.get_or_insert("openwebui");
        }
    }

    // Bedrock + Vertex: register as candidates when the wizard config is on
    // disk *and* the relevant CLI is installed. Neither becomes the default
    // — the user opts in by picking a Bedrock/Vertex model from the picker
    // or by using a `bedrock/<id>` / `vertex/<id>` qualified ModelSpec.
    let bedrock = BedrockProvider::new();
    if bedrock.has_usable_config() {
        tracing::info!(
            target: "jfc::startup",
            "registering Bedrock provider (config + aws CLI present)"
        );
        providers.push(Arc::new(bedrock));
    }
    let vertex = VertexProvider::new();
    if vertex.has_usable_config() {
        tracing::info!(
            target: "jfc::startup",
            "registering Vertex provider (config + gcloud CLI present)"
        );
        providers.push(Arc::new(vertex));
    }

    if providers.is_empty() {
        // Last-resort fallback so we don't panic on empty list — OAuth provider will
        // surface a clean "no accounts" error on first stream.
        providers.push(Arc::new(AnthropicOAuthProvider::new()));
        prefer = Some("anthropic-oauth");
    }

    // Provider routing — three-tier:
    //
    // 1. **Explicit prefix** (from ModelSpec): `"openwebui/bedrock-claude-4-6-opus"`
    //    → directly look up provider named "openwebui". No guessing.
    //
    // 2. **Static catalogue match**: scan each provider's `available_models()` for
    //    an id matching the model portion. First match wins.
    //
    // 3. **Heuristic fallback**: if no static match AND OpenWebUI is configured AND
    //    the model id doesn't look Anthropic-native (`claude-…`), route to OpenWebUI
    //    as the catch-all proxy whose catalogue is populated at runtime.
    //
    // Without any of these matching, fall back to the env-var precedence (`prefer`).
    let model_str = model.as_str();

    let provider_for_model: Option<String> = if let Some(prefix) = spec.provider() {
        // Tier 1: explicit provider prefix from config
        tracing::info!(
            target: "jfc::startup",
            model = %model_str,
            explicit_provider = %prefix,
            "model spec has explicit provider prefix — routing directly"
        );
        Some(prefix.as_str().to_owned())
    } else {
        // Tier 2: static catalogue lookup
        let static_match: Option<String> = providers
            .iter()
            .find(|p| {
                p.available_models()
                    .iter()
                    .any(|m| m.id.as_str() == model_str)
            })
            .map(|p| p.name().to_owned());

        static_match.or_else(|| {
            // Tier 3: heuristic — OpenAI-looking ids route to OpenAI, then
            // non-`claude-` ids route to OpenWebUI proxy when configured.
            let has_codex_config = providers.iter().any(|p| p.name() == "codex");
            if has_codex_config && looks_codex_model(model_str) {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks Codex-native → codex"
                );
                return Some("codex".to_owned());
            }

            let has_openai_config = providers.iter().any(|p| p.name() == "openai");
            if has_openai_config && looks_openai_model(model_str) {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks OpenAI-native → openai"
                );
                return Some("openai".to_owned());
            }

            let has_litellm_config = providers.iter().any(|p| p.name() == "litellm");
            let looks_proxy_routed = !model_str.is_empty() && !model_str.starts_with("claude-");
            if has_litellm_config && looks_proxy_routed {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks proxy-routed → litellm"
                );
                return Some("litellm".to_owned());
            }

            if has_openwebui_config && looks_proxy_routed {
                tracing::info!(
                    target: "jfc::startup",
                    model = %model_str,
                    "no static match, model looks proxy-routed → openwebui"
                );
                Some("openwebui".to_owned())
            } else {
                None
            }
        })
    };

    if let Some(name) = provider_for_model.as_deref() {
        tracing::info!(
            target: "jfc::startup",
            model = %model_str,
            matched_provider = %name,
            "routed startup model to its owning provider"
        );
    }

    let active_idx = provider_for_model
        .as_deref()
        .or(prefer)
        .and_then(|name| providers.iter().position(|p| p.name() == name))
        .unwrap_or(0);

    ProvidersInit {
        providers,
        active_idx,
        model,
        oauth: oauth_arc,
    }
}

/// Route a model id to the provider that owns it.
///
/// Accepts either a qualified `"provider/model"` spec or a bare `"model"` id.
/// When qualified, looks up the provider by name directly. Otherwise uses the
/// same three-tier logic as `build_providers`: static catalogue → heuristic.
///
/// Used by `--continue`/`--resume` to re-route when the saved session's model
/// belongs to a different provider than the env-var precedence picked.
pub(crate) fn provider_for_model(
    providers: &[Arc<dyn Provider>],
    model_id: &str,
) -> Option<Arc<dyn Provider>> {
    if model_id.is_empty() {
        return None;
    }
    // Try parsing as ModelSpec — if qualified, route directly by provider name
    if let Ok(spec) = model_id.parse::<ModelSpec>() {
        if let Some(prefix) = spec.provider() {
            return providers
                .iter()
                .find(|p| p.name() == prefix.as_str())
                .cloned();
        }
    }
    // Tier 2: static catalogue lookup
    if let Some(p) = providers.iter().find(|p| {
        p.available_models()
            .iter()
            .any(|m| m.id.as_str() == model_id)
    }) {
        return Some(Arc::clone(p));
    }
    // Tier 3: heuristic — OpenAI-looking ids route to OpenAI first, then
    // non-`claude-` ids route to OpenWebUI proxy.
    let has_codex = providers.iter().any(|p| p.name() == "codex");
    if has_codex && looks_codex_model(model_id) {
        return providers.iter().find(|p| p.name() == "codex").cloned();
    }

    let has_openai = providers.iter().any(|p| p.name() == "openai");
    if has_openai && looks_openai_model(model_id) {
        return providers.iter().find(|p| p.name() == "openai").cloned();
    }

    let has_litellm = providers.iter().any(|p| p.name() == "litellm");
    if has_litellm && !model_id.starts_with("claude-") {
        return providers.iter().find(|p| p.name() == "litellm").cloned();
    }

    let has_openwebui = providers.iter().any(|p| p.name() == "openwebui");
    if has_openwebui && !model_id.starts_with("claude-") {
        return providers.iter().find(|p| p.name() == "openwebui").cloned();
    }
    None
}

fn looks_openai_model(model_id: &str) -> bool {
    model_id.starts_with("gpt-")
        || model_id.starts_with("o1")
        || model_id.starts_with("o3")
        || model_id.starts_with("o4")
}

fn looks_codex_model(model_id: &str) -> bool {
    let id = model_id
        .rsplit('/')
        .next()
        .unwrap_or(model_id)
        .to_ascii_lowercase();
    id.contains("codex")
}
