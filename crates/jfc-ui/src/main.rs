mod agents;
mod app;
mod attachments;
mod auto_mode;
mod compact;
mod config;
mod context;
mod cost;
mod diagnostics;
mod diagnostics_producer;
mod inline_tools;
mod input;
mod lsp_client;
mod lsp_rpc;
mod markdown;
mod mentions;
mod message_view;
mod provider;
mod providers;
mod query;
mod render;
mod scheduler;
mod session;
mod spinner;
mod stream;
mod tasks;
mod theme;
mod toast;
mod tools;
mod types;
mod worktrees;

use std::{io, sync::Arc, time::Duration};

use clap::Parser;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event, KeyEventKind, KeyboardEnhancementFlags, PopKeyboardEnhancementFlags,
        PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use app::{App, AppEvent, PendingApproval, SPINNER, TICK_MS};
use provider::{ModelId, Provider, ProviderId};
use providers::{AnthropicOAuthProvider, AnthropicProvider, OpenWebUIProvider};
use types::*;

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

    /// Initial prompt to send (non-interactive if specified)
    #[arg(long, short = 'p', value_name = "PROMPT")]
    prompt: Option<String>,

    /// Model to use (overrides ANTHROPIC_MODEL env var)
    #[arg(long, short = 'm', value_name = "MODEL")]
    model: Option<String>,
}

/// Session to load at startup based on CLI args
enum StartupSession {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
    let _trace_guard = init_tracing();

    let init = build_providers();
    let providers = init.providers;
    let active_idx = init.active_idx;
    // Determine startup session from CLI flags (before consuming cli fields)
    let startup_session = cli.startup_session();
    let initial_prompt = cli.prompt;

    // CLI --model overrides env var
    let model = cli.model.map(ModelId::from).unwrap_or(init.model);
    let oauth_handle = init.oauth;
    let provider = providers[active_idx].clone();

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

    let result = run(
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

/// Initialize tracing so structured logs flow to `~/.config/jfc/logs/jfc.log`
/// (rolling daily). Returns the `WorkerGuard` from `tracing-appender::non_blocking`
/// — caller must hold it until process exit so buffered logs flush.
///
/// Falls back to a no-op `WorkerGuard` (writing to `io::sink`) when the log
/// directory can't be created (read-only home, permission errors). We never
/// log to stderr because that breaks the TUI's alternate screen.
fn init_tracing() -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::EnvFilter;

    let log_dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    let appender = tracing_appender::rolling::daily(&log_dir, "jfc.log");
    let (writer, guard) = tracing_appender::non_blocking(appender);

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

/// Copy the most recent assistant message to the system clipboard via arboard.
/// Used by Ctrl+Y in `input.rs` and the left-click handler in the main loop.
/// No-ops silently if no assistant message exists, or if the clipboard backend
/// is unavailable (headless container, sandboxed terminal).
fn yank_last_assistant(app: &App) {
    let Some(text) = app
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| {
            m.parts
                .iter()
                .filter_map(|p| match p {
                    MessagePart::Text(t) => Some(t.clone()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    match arboard::Clipboard::new() {
        Ok(mut cb) => {
            if let Err(e) = cb.set_text(text.clone()) {
                tracing::warn!(target: "jfc::ui::yank", error = %e, "set_text failed");
            } else {
                tracing::info!(
                    target: "jfc::ui::yank",
                    len = text.len(),
                    "yanked via mouse click"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::ui::yank",
                error = %e,
                "clipboard backend unavailable"
            );
        }
    }
}

/// Drain the next queued prompt and submit it as a new user turn. Mirrors
/// v126's `queued_command` attachment system — when the model finishes its
/// turn, we replay the user's queued input as if they'd just typed and hit
/// Enter. Pops one prompt per call; subsequent prompts surface naturally as
/// the next StreamDone fires.
///
/// The placeholder `⏳ <text>` user message we inserted at queue time gets
/// replaced by a clean `<text>` message when we drain — so the transcript
/// stays consistent with what the model actually sees.
async fn drain_queued_prompts(app: &mut App, tx: &mpsc::UnboundedSender<AppEvent>) {
    let Some(prompt) = app.queued_prompts.pop_front() else {
        return;
    };
    let crate::app::QueuedPrompt { text, is_meta } = prompt;
    tracing::info!(
        target: "jfc::ui::queue",
        remaining = app.queued_prompts.len(),
        len = text.len(),
        is_meta,
        "drain_queued_prompt"
    );

    // Replace the placeholder ("⏳ " for prose, "⚙ " for slash commands) with
    // the clean text so the transcript matches what gets sent to the API
    // (or what the slash-command handler executes against).
    let glyph = if is_meta { "⚙" } else { "⏳" };
    let placeholder = format!("{glyph} {text}");
    for msg in app.messages.iter_mut() {
        if msg.role == Role::User {
            for part in msg.parts.iter_mut() {
                if let MessagePart::Text(t) = part {
                    if *t == placeholder {
                        *t = text.clone();
                        break;
                    }
                }
            }
        }
    }

    if is_meta {
        // v126 isMeta: slash commands execute locally instead of streaming.
        // We don't even hit the API — just dispatch through the existing
        // slash command handler. Subsequent queued prompts surface
        // immediately because no new stream starts.
        input::run_slash_command(app, &text);
        // Recurse: another queued prompt may be ready right now.
        Box::pin(drain_queued_prompts(app, tx)).await;
        return;
    }

    // Regular prompt path: run the same submit pipeline as a fresh user
    // turn. We don't push *another* user message — the placeholder we just
    // patched above stands in. Build the assistant slot + spawn the stream.
    let assistant_idx = app.messages.len();
    app.tool_ctx.total_user_turns += 1;
    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    app.streaming_response_bytes = 0;
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.streaming_last_token_at = Some(now);
    // Set the user-level turn clock too — survives across agentic-loop
    // iterations so a 5-step turn doesn't keep snapping back to `0s`.
    app.turn_started_at = Some(now);
    // Wire-truth output_tokens are cumulative *per request* — Anthropic
    // restarts the counter at zero for each `messages` call. Reset our
    // mirror so the spinner doesn't carry the prior turn's leftover until
    // the next `message_delta` arrives. Same reasoning for the per-model
    // delta baseline — see `usage_apply_baseline` doc on `App`.
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);
    app.scroll_to_bottom();

    let provider = app.provider.clone();
    let messages = stream::build_provider_messages(&app.messages[..assistant_idx]);
    let model = app.model.clone();
    let tx = tx.clone();
    tokio::spawn(async move {
        stream::stream_response(provider, messages, model, tx).await;
    });
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
struct ProvidersInit {
    providers: Vec<Arc<dyn Provider>>,
    active_idx: usize,
    model: ModelId,
    oauth: Option<Arc<AnthropicOAuthProvider>>,
}

/// Build every provider that has usable config in this environment, plus pick which one
/// should be active at startup.
///
/// Active selection mirrors the prior single-provider precedence: explicit `ANTHROPIC_API_KEY`
/// wins, then `OPENWEBUI_BASE_URL`, then OAuth.
fn build_providers() -> ProvidersInit {
    let model = std::env::var("ANTHROPIC_MODEL")
        .or_else(|_| std::env::var("OPENWEBUI_MODEL"))
        .map(ModelId::from)
        .unwrap_or_else(|_| ModelId::from("claude-opus-4-5"));

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

    // OpenWebUI is registered as a candidate so its models show up in the picker, but
    // it only becomes the *default* when the user explicitly opts in via OPENWEBUI_BASE_URL.
    let openwebui = OpenWebUIProvider::new();
    if openwebui.has_usable_config() {
        providers.push(Arc::new(openwebui));
        if std::env::var("OPENWEBUI_BASE_URL").is_ok() {
            prefer.get_or_insert("openwebui");
        }
    }

    if providers.is_empty() {
        // Last-resort fallback so we don't panic on empty list — OAuth provider will
        // surface a clean "no accounts" error on first stream.
        providers.push(Arc::new(AnthropicOAuthProvider::new()));
        prefer = Some("anthropic-oauth");
    }

    let active_idx = prefer
        .and_then(|name| providers.iter().position(|p| p.name() == name))
        .unwrap_or(0);

    ProvidersInit {
        providers,
        active_idx,
        model,
        oauth: oauth_arc,
    }
}

async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    providers: Vec<Arc<dyn Provider>>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    oauth_handle: Option<Arc<AnthropicOAuthProvider>>,
    startup_session: StartupSession,
    initial_prompt: Option<String>,
) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = App::new(provider, model);
    app.providers = providers.clone();

    // Handle --continue / --resume flags
    match startup_session {
        StartupSession::Fresh => {}
        StartupSession::Continue => {
            // `--continue` is cwd-scoped (codex-rs / v126 parity). The
            // user can pass `--continue --global` later if we add the
            // flag; for now the cwd default is what they actually want.
            let cwd_str = std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string());
            let id = session::most_recent_session_for_cwd(cwd_str.as_deref())
                .or_else(session::most_recent_session); // legacy fallback
            if let Some(session_id) = id {
                if let Some(messages) = session::load_session(&session_id) {
                    tracing::info!(
                        target: "jfc::session",
                        session_id = %session_id,
                        message_count = messages.len(),
                        cwd = ?cwd_str,
                        "continuing most recent session"
                    );
                    app.messages = messages;
                    app.current_session_id = Some(session_id);
                    app.recompute_token_estimate();
                }
            }
        }
        StartupSession::Resume(session_id) => {
            if let Some(messages) = session::load_session(&session_id) {
                tracing::info!(
                    target: "jfc::session",
                    session_id = %session_id,
                    message_count = messages.len(),
                    "resuming specific session"
                );
                // CLI has no toast surface; emit a warn-level log so
                // the user sees the cwd mismatch in stderr/journalctl
                // and doesn't silently load a session from a different
                // project. Mirrors codex-rs `session_resume.rs:99-111`.
                let session_cwd = session::load_session_metadata(&session_id)
                    .and_then(|m| m.cwd);
                let current_cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(msg) = session::cwd_mismatch_message(
                    session_cwd.as_deref(),
                    &current_cwd,
                ) {
                    tracing::warn!(
                        target: "jfc::session",
                        session_id = %session_id,
                        "{msg}"
                    );
                }
                app.messages = messages;
                app.current_session_id = Some(session_id);
                app.recompute_token_estimate();
            } else {
                tracing::warn!(
                    target: "jfc::session",
                    session_id = %session_id,
                    "session not found, starting fresh"
                );
            }
        }
    }

    // Handle --prompt flag: queue an initial prompt to submit after startup
    let queued_initial_prompt = initial_prompt;

    // Kick off background model-list fetches so the picker reflects what each provider
    // actually serves (e.g., the user's OpenWebUI instance) instead of stale hardcoded
    // ids that produce "Model not found" at stream time.
    for p in &providers {
        let tx = tx.clone();
        let p = Arc::clone(p);
        let name = ProviderId::from(p.name());
        tokio::spawn(async move {
            let models = p.fetch_models().await.unwrap_or_default();
            let _ = tx.send(AppEvent::ModelsLoaded {
                provider: name,
                models,
            });
        });
    }

    // Kick off OAuth profile fetch — needed for v126-equivalent seat-tier model gating
    // (XwH() in cli.js) and for showing the subscription type / email in the status bar.
    // Best-effort: a failure here just leaves seat_tier None, which means "no filter".
    if let Some(oauth) = oauth_handle {
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(profile) = oauth.fetch_profile().await {
                let _ = tx.send(AppEvent::ProfileLoaded {
                    seat_tier: profile.seat_tier,
                    subscription_type: profile.subscription_type,
                    email: profile.email,
                });
            }
        });
    }

    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = event::EventStream::new();
            while let Some(Ok(ev)) = reader.next().await {
                let _ = tx.send(AppEvent::Term(ev));
            }
        });
    }

    {
        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
                let _ = tx.send(AppEvent::Tick);
            }
        });
    }

    // Initial `cargo check` so the diagnostic row populates without
    // waiting for `/check`. Skipped via `JFC_DISABLE_CARGO_CHECK=1` for
    // CI / non-Rust workspaces. Best-effort — `run_once` silently no-ops
    // if cargo isn't on PATH or the cwd isn't a cargo project.
    if !matches!(
        std::env::var("JFC_DISABLE_CARGO_CHECK").as_deref(),
        Ok("1") | Ok("true")
    ) {
        let tx_diag = tx.clone();
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        tokio::spawn(async move {
            diagnostics_producer::run_once(cwd, tx_diag).await;
        });
    }

    // Real LSP client: spawns rust-analyzer (Cargo.toml present) or zls
    // (build.zig present) and routes `textDocument/publishDiagnostics`
    // into `AppEvent::DiagnosticsUpdated`. Gated by `JFC_DISABLE_LSP=1`.
    // `maybe_spawn_lsp_clients` is fire-and-forget — startup never
    // blocks on the handshake. If the binary isn't on PATH, the spawn
    // task silently returns and we fall back to the cargo-check
    // producer above.
    {
        let tx_lsp = tx.clone();
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        lsp_client::maybe_spawn_lsp_clients(cwd, tx_lsp);
    }

    app.sync_task_completions();
    terminal.draw(|f| render::frame(f, &mut app))?;

    // Submit initial prompt if provided via --prompt flag
    if let Some(prompt) = queued_initial_prompt {
        // Use the same logic as handle_submit but without waiting for user input
        let assistant_idx = app.messages.len() + 1;
        app.messages.push(ChatMessage::user(prompt.clone()));
        app.tool_ctx.total_user_turns += 1;
        app.messages.push(ChatMessage::assistant(String::new()));
        app.streaming_assistant_idx = Some(assistant_idx);
        app.is_streaming = true;
        let now = std::time::Instant::now();
        app.streaming_started_at = Some(now);
        app.streaming_last_token_at = Some(now);
        app.turn_started_at = Some(now);
        app.last_usage_output = 0;
        app.usage_apply_baseline = (0, 0, 0, 0);

        // Create session if not resuming one
        let session_id = app
            .current_session_id
            .clone()
            .unwrap_or_else(session::generate_session_id);
        session::save_session(&session_id, &app.messages, Some(app.cwd.as_str()));
        app.current_session_id = Some(session_id);

        let provider = app.provider.clone();
        let messages = stream::build_provider_messages(&app.messages[..assistant_idx]);
        let model = app.model.clone();
        let tx_clone = tx.clone();
        tokio::spawn(async move {
            stream::stream_response(provider, messages, model, tx_clone).await;
        });
    }

    loop {
        let ev = match rx.recv().await {
            Some(e) => e,
            None => break,
        };

        match ev {
            // Accept Press *and* Repeat so holding ↑/↓ keeps moving in the picker.
            // The kitty keyboard protocol (enabled via REPORT_EVENT_TYPES at startup)
            // delivers separate Repeat events while a key is held — without this filter
            // they would be discarded. Release events still fall through.
            AppEvent::Term(Event::Key(k))
                if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
            {
                if input::handle_key(&mut app, k, &tx).await? {
                    break;
                }
            }
            AppEvent::Term(Event::Paste(text)) => {
                app.textarea.insert_str(&text);
            }
            AppEvent::Term(Event::Mouse(mouse)) => {
                use crossterm::event::{MouseButton, MouseEventKind};
                match mouse.kind {
                    MouseEventKind::ScrollUp => app.scroll_up(3),
                    MouseEventKind::ScrollDown => app.scroll_down(3),
                    // Left-click on the message pane copies the assistant
                    // message under the cursor to the clipboard. ratatui
                    // doesn't expose hit-testing, so we approximate: any
                    // click outside the input area + sidebar copies the
                    // most recent assistant text. (Full message-by-position
                    // hit detection would require tracking each message's
                    // y-range during render, which is the next iteration.)
                    MouseEventKind::Down(MouseButton::Left) => {
                        // First, see if the click landed on a tool block —
                        // each visible tool is registered in
                        // `app.tool_hit_regions` by the renderer. Toggling
                        // `expanded` flips the body between preview and
                        // full content. Mirrors v126's per-tool expand
                        // affordance (cmd-click on iTerm2; we use a plain
                        // click since non-iTerm terminals don't surface
                        // the cmd modifier the same way).
                        let hit = message_view::find_tool_at(
                            &app.tool_hit_regions.borrow(),
                            mouse.column,
                            mouse.row,
                        )
                        .map(str::to_owned);
                        if let Some(tool_id) = hit {
                            for msg in &mut app.messages {
                                for part in &mut msg.parts {
                                    if let MessagePart::Tool(tc) = part {
                                        if tc.id == tool_id {
                                            tc.expanded = !tc.expanded;
                                        }
                                    }
                                }
                            }
                        } else {
                            let in_input = mouse.row as usize
                                >= app
                                    .viewport_height
                                    .saturating_add(app.scroll_offset)
                                    .saturating_sub(2);
                            if !in_input {
                                yank_last_assistant(&app);
                            }
                        }
                    }
                    _ => {}
                }
            }
            AppEvent::Term(_) => {}
            AppEvent::Tick => {
                app.spinner_frame = (app.spinner_frame + 1) % SPINNER.len();
                // Auto-clear expired toasts every tick. Cheap (O(N) over
                // a tiny vec capped at MAX_TOASTS) and the only reliable
                // place to do it — toasts have no creation-time timer.
                toast::prune_expired(&mut app.toasts, std::time::Instant::now());
            }
            AppEvent::StreamChunk { text, reasoning } => {
                // Reset the stall clock on every chunk so the spinner's
                // sub-status (`warming up` / `thinking` / `almost done`)
                // reflects time-since-last-byte, not time-since-stream-start.
                let now = std::time::Instant::now();
                app.streaming_last_token_at = Some(now);
                // v126 responseLengthRef: accumulate ALL content bytes for the
                // spinner's chars/4 token estimate.
                if let Some(ref t) = text {
                    app.streaming_response_bytes += t.len();
                }
                if let Some(ref r) = reasoning {
                    app.streaming_response_bytes += r.len();
                }
                if let Some(chunk) = text {
                    // First text byte after a thinking phase ⇒ thinking
                    // ended. Mirrors v126's HcH transition from
                    // `streamMode = "thinking"` to `"responding"` —
                    // cli.js:413612 captures the duration here so the
                    // spinner can switch from `thinking…` to
                    // `thought for Ns`. Only set on the first transition;
                    // a turn that toggles back into thinking later (rare
                    // — the API doesn't really do this) keeps the first
                    // duration so the timer doesn't reset visibly.
                    if app.thinking_started_at.is_some() && app.thinking_ended_at.is_none() {
                        app.thinking_ended_at = Some(now);
                    }
                    app.streaming_text.push_str(&chunk);
                    if let Some(idx) = app.streaming_assistant_idx {
                        if let Some(msg) = app.messages.get_mut(idx) {
                            match msg
                                .parts
                                .iter_mut()
                                .find(|p| matches!(p, MessagePart::Text(_)))
                            {
                                Some(MessagePart::Text(t)) => t.push_str(&chunk),
                                _ => msg.parts.push(MessagePart::Text(chunk)),
                            }
                        }
                    }
                }
                if let Some(chunk) = reasoning {
                    // First reasoning byte ⇒ thinking started. Mirrors
                    // v126's HcH content_block_start type=thinking
                    // transition (cli.js:413610). Subsequent chunks just
                    // extend the streaming buffer; the spinner reads
                    // `thinking_started_at` to know we're in
                    // thinking-mode.
                    if app.thinking_started_at.is_none() {
                        app.thinking_started_at = Some(now);
                    }
                    app.streaming_reasoning.push_str(&chunk);
                    if let Some(idx) = app.streaming_assistant_idx {
                        if let Some(msg) = app.messages.get_mut(idx) {
                            match msg
                                .parts
                                .iter_mut()
                                .find(|p| matches!(p, MessagePart::Reasoning(_)))
                            {
                                Some(MessagePart::Reasoning(t)) => t.push_str(&chunk),
                                _ => msg.parts.push(MessagePart::Reasoning(chunk)),
                            }
                        }
                    }
                }
                // Follow content as it streams *only when the user is
                // already pinned to the bottom*. `app.follow_bottom` is
                // set true on submit and on any explicit scroll-to-bottom;
                // it goes false the moment the user scrolls up. Without
                // this gate, scrolling up to read prior context during a
                // long stream would yank you back to the bottom on every
                // chunk. v126 has the same "stick when at bottom" rule.
                if app.follow_bottom {
                    app.scroll_to_bottom();
                }
            }
            AppEvent::ToolInputDelta(byte_len) => {
                // Tool input JSON streaming — accumulate bytes for the spinner's
                // token estimate and reset the stall timer. Matches v126's
                // accumulation of input_json_delta into responseLengthRef.
                app.streaming_response_bytes += byte_len;
                app.streaming_last_token_at = Some(std::time::Instant::now());
            }
            AppEvent::StreamTool(tool) => {
                // Trace every StreamTool entry so next-run diagnostics show
                // exactly which routing path each tool took. Without this,
                // tools that take the auto-mode or no-approval branches are
                // invisible in logs (only the approval path was traced),
                // making bugs like "tool stuck Pending" undiagnosable.
                tracing::info!(
                    target: "jfc::ui::tool",
                    tool_kind = tool.kind.label(),
                    tool_id = %tool.id,
                    auto_mode = app.auto_mode.enabled,
                    needs_approval = app.tool_needs_approval(&tool),
                    streaming_idx = ?app.streaming_assistant_idx,
                    "StreamTool received"
                );
                // v126 auto-mode: when enabled, every tool call is sent to a
                // classifier LLM that returns block/allow with a reason. The
                // user is never prompted. Disabled (default) → original flow.
                if app.auto_mode.enabled {
                    tracing::info!(
                        target: "jfc::ui::tool",
                        tool_id = %tool.id,
                        "route=auto_mode_classifier"
                    );
                    let provider = Arc::clone(&app.provider);
                    let model = app.model.clone();
                    let cfg = app.auto_mode.clone();
                    let history = app.messages.clone();
                    let tx_cls = tx.clone();
                    let tool_for_task = tool.clone();
                    tokio::spawn(async move {
                        let decision = auto_mode::classify(
                            provider.as_ref(),
                            &model,
                            &cfg,
                            &history,
                            &tool_for_task,
                        )
                        .await;
                        let _ = tx_cls.send(AppEvent::ClassifierDecision {
                            tool: tool_for_task,
                            blocked: decision.should_block(),
                            reason: decision.reason,
                        });
                    });
                } else if app.tool_needs_approval(&tool) {
                    // Insert the tool into the assistant message *immediately*
                    // with status Pending so the user can SEE that the model
                    // wants to call N tools — without this, only the assistant
                    // text rendered and queued tools were invisible until each
                    // got dispatched. The dispatch path mutates the same
                    // ToolCall entry by id when ToolResult arrives, flipping
                    // status to Complete/Failed and setting output.
                    if let Some(idx) = app.streaming_assistant_idx {
                        if let Some(msg) = app.messages.get_mut(idx) {
                            msg.parts.push(MessagePart::Tool(tool.clone()));
                        }
                    }
                    // First approvable tool fills `pending_approval`; every
                    // subsequent one queues behind it. The decide-handlers in
                    // input.rs pop the next from `approval_queue` after each
                    // verdict so the modal cycles through them in order.
                    let kind_label = tool.kind.label();
                    let tool_id = tool.id.clone();
                    if app.pending_approval.is_none() {
                        tracing::info!(
                            target: "jfc::ui::approval",
                            tool_kind = kind_label,
                            tool_id = %tool_id,
                            "modal_opened"
                        );
                        app.pending_approval = Some(PendingApproval { tool, selected: 0 });
                    } else {
                        tracing::info!(
                            target: "jfc::ui::approval",
                            tool_kind = kind_label,
                            tool_id = %tool_id,
                            queue_depth = app.approval_queue.len() + 1,
                            "queued_behind_modal"
                        );
                        app.approval_queue.push_back(tool);
                    }
                } else {
                    tracing::info!(
                        target: "jfc::ui::tool",
                        tool_kind = tool.kind.label(),
                        tool_id = %tool.id,
                        pending_total = app.pending_tool_calls.len() + 1,
                        "route=auto_dispatch (no approval needed)"
                    );
                    if let Some(idx) = app.streaming_assistant_idx {
                        if let Some(msg) = app.messages.get_mut(idx) {
                            msg.parts.push(MessagePart::Tool(tool.clone()));
                        }
                    }
                    app.pending_tool_calls.push(tool);
                }
            }
            AppEvent::ClassifierDecision {
                mut tool,
                blocked,
                reason,
            } => {
                if blocked {
                    tool.status = ToolStatus::Failed;
                    tool.output = ToolOutput::Text(format!(
                        "Auto-mode classifier blocked this tool call.\n\nReason: {reason}"
                    ));
                    if let Some(idx) = app.streaming_assistant_idx {
                        if let Some(msg) = app.messages.get_mut(idx) {
                            msg.parts.push(MessagePart::Tool(tool));
                        }
                    }
                } else {
                    if let Some(idx) = app.streaming_assistant_idx {
                        if let Some(msg) = app.messages.get_mut(idx) {
                            msg.parts.push(MessagePart::Tool(tool.clone()));
                        }
                    }
                    app.pending_tool_calls.push(tool);
                }
            }
            AppEvent::StreamDone(stop_reason) => {
                tracing::info!(
                    target: "jfc::stream",
                    ?stop_reason,
                    pending_tool_count = app.pending_tool_calls.len(),
                    pending_approval = app.pending_approval.is_some(),
                    approval_queue = app.approval_queue.len(),
                    "AppEvent::StreamDone received"
                );
                app.is_streaming = false;
                // v126's "Cooked for Nm Ns" post-turn footer: stamp the
                // assistant message with a randomized past-tense verb +
                // formatted duration the moment the stream resolves. The
                // renderer reads `msg.elapsed` and prints it under the
                // assistant's content. Mirrors cli.js:341376
                // (`${A} for ${w}` where A = past-tense verb, w = duration).
                // Stamp `Cooked for Nm Ns` only on the *final* message of
                // the user turn — i.e. when `stop_reason == EndTurn` with
                // nothing pending. Otherwise every sub-stream of a 5-step
                // agentic loop got its own footer (`Brewed for 2s`,
                // `Brewed for 3s`, ...). v126 stamps once per turn so the
                // user sees the cumulative `Brewed for 5m 10s` on the
                // turn's last message. The duration is read off
                // `turn_started_at` (still set at this point — we only
                // clear it in the next block once the EndTurn condition
                // is verified) so it covers tools + thinking + final text.
                let turn_done = stop_reason == provider::StopReason::EndTurn
                    && app.pending_approval.is_none()
                    && app.approval_queue.is_empty()
                    && app.pending_tool_calls.is_empty();
                if turn_done {
                    if let (Some(start), Some(idx)) =
                        (app.turn_started_at, app.streaming_assistant_idx)
                    {
                        let elapsed = std::time::Instant::now().duration_since(start);
                        let label = spinner::format_finished(elapsed);
                        if let Some(msg) = app.messages.get_mut(idx) {
                            msg.elapsed = Some(label);
                        }
                    }
                }
                app.streaming_started_at = None;
                app.streaming_last_token_at = None;
                // If thinking started but never transitioned to text
                // (e.g. the assistant only produced thinking + tool calls
                // and no visible text), stamp the end now so the spinner
                // shows `thought for Ns` next iteration instead of a
                // stuck `thinking…` from the last reasoning chunk.
                if app.thinking_started_at.is_some() && app.thinking_ended_at.is_none() {
                    app.thinking_ended_at = Some(std::time::Instant::now());
                }
                app.streaming_text.clear();
                app.streaming_reasoning.clear();
                // Only reset the cumulative token counter when the turn is
                // truly done. During agentic loops (ToolUse stop_reason), the
                // counter should keep accumulating so the spinner shows the
                // full turn's token estimate.
                if turn_done {
                    app.streaming_response_bytes = 0;
                }
                // Clear the user-turn clock only when the loop has
                // genuinely concluded — EndTurn stop reason AND no
                // tools pending. ToolUse means an agentic continuation
                // is about to fire and the turn timer must keep running.
                if stop_reason == provider::StopReason::EndTurn
                    && app.pending_approval.is_none()
                    && app.approval_queue.is_empty()
                    && app.pending_tool_calls.is_empty()
                {
                    app.turn_started_at = None;
                }

                // Auto-save session after each assistant turn completes
                if let Some(ref session_id) = app.current_session_id {
                    session::save_session(session_id, &app.messages, Some(app.cwd.as_str()));
                }
                // v126 queued-prompt drain on plain end_turn: model finished
                // without tools to call → if anything's queued, fire it now.
                if stop_reason == provider::StopReason::EndTurn
                    && app.pending_approval.is_none()
                    && app.approval_queue.is_empty()
                    && app.pending_tool_calls.is_empty()
                    && !app.queued_prompts.is_empty()
                {
                    drain_queued_prompts(&mut app, &tx).await;
                }
                if stop_reason == provider::StopReason::ToolUse {
                    if !app.pending_tool_calls.is_empty() {
                        let calls = std::mem::take(&mut app.pending_tool_calls);
                        tracing::info!(
                            target: "jfc::stream",
                            n = calls.len(),
                            kinds = ?calls.iter().map(|t| t.kind.label()).collect::<Vec<_>>(),
                            "stream_done dispatching auto-routed batch"
                        );
                        update_task_activities(&mut app, &calls);
                        stream::dispatch_tools_batched(
                            calls,
                            &tx,
                            std::sync::Arc::clone(&app.dedup_cache),
                            Some(std::sync::Arc::clone(&app.task_store)),
                            std::sync::Arc::clone(&app.provider),
                            app.model.clone(),
                        );
                    } else if app.pending_approval.is_some() || !app.approval_queue.is_empty() {
                        tracing::info!(
                            target: "jfc::stream",
                            pending_modal = app.pending_approval.is_some(),
                            queue_depth = app.approval_queue.len(),
                            "stream_done waiting on approval pipeline"
                        );
                        // Tool awaiting user approval — keep streaming_assistant_idx
                        // alive so the approved/denied tool can be inserted into the
                        // correct message. AllToolsComplete fires after approval.
                    } else {
                        // Upstream returned finish_reason="tool_calls" but sent
                        // zero tool_call delta chunks (transient LiteLLM/Bedrock
                        // failure). The assistant message that was pre-pushed to
                        // history is empty and un-replyable; strip it so the
                        // next user turn doesn't send a broken conversation turn.
                        tracing::warn!(
                            target: "jfc::stream",
                            streaming_idx = ?app.streaming_assistant_idx,
                            "stream_done ToolUse with no tools — stripping dangling assistant turn"
                        );
                        if let Some(idx) = app.streaming_assistant_idx {
                            if idx < app.messages.len() {
                                let msg = &app.messages[idx];
                                let is_empty = msg.parts.is_empty()
                                    || msg.parts.iter().all(|p| {
                                        matches!(p, MessagePart::Text(t) if t.trim().is_empty())
                                    });
                                if is_empty {
                                    app.messages.remove(idx);
                                }
                            }
                        }
                        app.streaming_assistant_idx = None;
                        app.scroll_to_bottom();
                    }
                } else {
                    app.pending_tool_calls.clear();
                    app.streaming_assistant_idx = None;
                    app.scroll_to_bottom();
                }
            }
            AppEvent::StreamError(e) => {
                tracing::error!(
                    target: "jfc::stream",
                    error = %e,
                    "AppEvent::StreamError — resetting stream state"
                );
                app.is_streaming = false;
                app.streaming_started_at = None;
                app.streaming_last_token_at = None;
                app.thinking_started_at = None;
                app.thinking_ended_at = None;
                app.streaming_text.clear();
                app.streaming_reasoning.clear();
                app.streaming_response_bytes = 0;
                app.streaming_assistant_idx = None;
                app.messages
                    .push(ChatMessage::assistant(format!("**Error:** {e}")));
                app.scroll_to_bottom();
            }
            AppEvent::StreamUsage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => {
                // Anthropic sends *cumulative* token counts in every
                // `message_delta` event (sse.rs:212-218 — see also
                // anthropic-messaging spec). Naively calling `add_delta`
                // on each event triple-counts: a 10-delta turn ending at
                // 2000 output tokens would push 1+5+10+25+...+2000 into
                // the per-model bucket, producing 5-15× inflated totals
                // (the user's "84,284 in" with `ctx 28k / 200k` is this
                // bug). Compute the genuine delta against the per-turn
                // baseline before adding.
                app.last_usage_input = input_tokens;
                app.last_usage_output = output_tokens;
                app.tool_ctx.approx_tokens = input_tokens as usize + output_tokens as usize;
                // Stamp the cumulative usage onto the streaming
                // assistant message. v126 attaches usage to each
                // assistant message (cli.js:416673) so on resume
                // `Wd(messages)` (cli.js:197282) can walk back to
                // recover the gauge total. We do the same: at
                // resume time the picker reads the last message's
                // `usage` rather than a default of 0.
                if let Some(idx) = app.streaming_assistant_idx {
                    if let Some(msg) = app.messages.get_mut(idx) {
                        msg.usage = Some(crate::types::ModelUsage {
                            input_tokens: input_tokens as u64,
                            output_tokens: output_tokens as u64,
                            cache_read_tokens: cache_read_tokens as u64,
                            cache_write_tokens: cache_write_tokens as u64,
                            cost_usd: None,
                        });
                    }
                }
                let model_key = app.model.as_str().to_owned();
                let cum = (
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                );
                app.usage_apply_baseline = app
                    .usage_by_model
                    .entry(model_key)
                    .or_default()
                    .apply_cumulative(cum, app.usage_apply_baseline);
            }
            AppEvent::McpUpdated { servers } => {
                app.mcp_servers = servers;
            }
            AppEvent::LspUpdated { servers } => {
                app.lsp_servers = servers;
            }
            AppEvent::DiagnosticsUpdated { entries } => {
                // Toast on transitions: empty → non-empty fires a warning
                // (or error if any entry is severity Error). Non-empty →
                // empty fires a success ("All diagnostics cleared"). The
                // user notices state changes without having to read the
                // dim row above the spinner.
                let was_empty = app.diagnostics.is_empty();
                let is_empty = entries.is_empty();
                let had_errors = entries
                    .iter()
                    .any(|e| matches!(e.severity, crate::diagnostics::Severity::Error));
                app.diagnostics = entries;
                if was_empty && !is_empty {
                    let kind = if had_errors {
                        toast::ToastKind::Error
                    } else {
                        toast::ToastKind::Warning
                    };
                    let summary = crate::diagnostics::format_summary(
                        app.diagnostics.len(),
                        crate::diagnostics::count_files(&app.diagnostics),
                    );
                    if let Some(s) = summary {
                        toast::push_with_cap(&mut app.toasts, toast::Toast::new(kind, s));
                    }
                } else if !was_empty && is_empty {
                    toast::push_with_cap(
                        &mut app.toasts,
                        toast::Toast::new(toast::ToastKind::Success, "All diagnostics cleared"),
                    );
                }
            }
            AppEvent::ToolResult { tool_id, result } => {
                tracing::info!(
                    target: "jfc::stream",
                    tool_id = %tool_id,
                    is_error = result.is_error(),
                    output_len = result.output.len(),
                    "tool_result received"
                );
                let mut found = false;
                for msg in &mut app.messages {
                    for part in &mut msg.parts {
                        if let MessagePart::Tool(tc) = part {
                            if tc.id == tool_id {
                                tc.output = if LargeText::should_collapse(&result.output) {
                                    ToolOutput::LargeText(LargeText::new(result.output.clone()))
                                } else {
                                    ToolOutput::Text(result.output.clone())
                                };
                                if LargeText::should_collapse(&result.output) {
                                    tc.is_collapsed = true;
                                }
                                tc.status = if result.is_error() {
                                    ToolStatus::Failed
                                } else {
                                    ToolStatus::Complete
                                };
                                found = true;
                                break;
                            }
                        }
                    }
                    if found {
                        break;
                    }
                }
                // Persist on every ToolResult so reload reflects tool outputs.
                // Without this, sessions saved at submit time carry empty
                // assistant placeholders + Pending tools — replaying them
                // shows a user prompt with nothing under it. v126 cli.js
                // saves on every state mutation; jfc previously only saved
                // at submit + StreamDone, missing the post-tool state.
                if let Some(ref session_id) = app.current_session_id {
                    session::save_session(session_id, &app.messages, Some(app.cwd.as_str()));
                }
            }
            AppEvent::AllToolsComplete => {
                tracing::info!(
                    target: "jfc::stream",
                    message_count = app.messages.len(),
                    model = %app.model,
                    "AppEvent::AllToolsComplete"
                );
                // Terminal bell when a tool batch completes — matches
                // v126's `iterm2_with_bell` / `terminal_bell` behavior
                // (cli.js:46704). Many users have iTerm2 / WezTerm /
                // Ghostty configured to badge or notify on bell, so this
                // gives a "your input is needed / a long task finished"
                // hint without us having to hand-roll desktop notifications.
                // Suppress when the user opted out via env (matches
                // v126's `notifications_disabled` setting).
                if !matches!(
                    std::env::var("JFC_DISABLE_BELL").as_deref(),
                    Ok("1") | Ok("true")
                ) {
                    use std::io::Write;
                    // Best-effort write — ignore failures; bell is cosmetic.
                    let _ = std::io::stderr().write_all(b"\x07");
                    let _ = std::io::stderr().flush();
                }
                let manual = std::mem::take(&mut app.force_compact_pending);
                if manual || compact::should_compact(&app.messages, app.max_context_tokens) {
                    tracing::info!(
                        target: "jfc::compact",
                        manual,
                        model = %app.model,
                        max_context_tokens = app.max_context_tokens,
                        message_count = app.messages.len(),
                        rapid_refill_count = app.tool_ctx.rapid_refill_count,
                        "post-response compaction triggered"
                    );
                    let _ = tx.send(AppEvent::CompactionStarted);
                    let messages = app.messages.clone();
                    let provider = Arc::clone(&app.provider);
                    let model = app.model.clone();
                    let mut tool_ctx = app.tool_ctx.clone();
                    let window = app.max_context_tokens;
                    let tx_compact = tx.clone();
                    tokio::spawn(async move {
                        let options = provider::StreamOptions::new(model.clone());
                        tracing::debug!(
                            target: "jfc::compact",
                            model = %model,
                            window,
                            "spawned post-response compaction task"
                        );
                        let result = compact::compact(
                            &messages,
                            provider.as_ref(),
                            &options,
                            &mut tool_ctx,
                            window,
                        )
                        .await;
                        match result {
                            compact::CompactResult::Success {
                                messages,
                                pre_tokens,
                                post_tokens,
                            } => {
                                tracing::info!(
                                    target: "jfc::compact",
                                    pre_tokens, post_tokens,
                                    saved = pre_tokens.saturating_sub(post_tokens),
                                    "post-response compaction succeeded — sending CompactionDone"
                                );
                                let _ = tx_compact.send(AppEvent::CompactionDone {
                                    messages,
                                    tool_ctx,
                                    pre_tokens,
                                    post_tokens,
                                });
                            }
                            compact::CompactResult::Unsupported
                            | compact::CompactResult::TooFewGroups => {
                                tracing::debug!(
                                    target: "jfc::compact",
                                    "post-response compaction skipped (unsupported/too few groups)"
                                );
                            }
                            compact::CompactResult::CircuitBreakerTripped => {
                                tracing::warn!(
                                    target: "jfc::compact",
                                    "post-response compaction: circuit breaker tripped"
                                );
                                let _ = tx_compact.send(AppEvent::CompactionFailed(
                                    "Circuit breaker tripped — compaction keeps refilling".into(),
                                    None,
                                ));
                            }
                            compact::CompactResult::Exhausted { attempts } => {
                                tracing::warn!(
                                    target: "jfc::compact",
                                    attempts,
                                    "post-response compaction exhausted all attempts"
                                );
                                let _ = tx_compact.send(AppEvent::CompactionFailed(format!(
                                    "Exhausted {attempts} compaction attempts"
                                ), None));
                            }
                        }
                    });
                }
                // Gate the agentic continuation on the approval pipeline being
                // empty. Without this, dispatching tool 0 fires
                // AllToolsComplete (1 tool finished, last message has 1
                // Complete part → should_continue_loop=true), the loop sends a
                // *new* request, and tools 1..N still queued for approval get
                // inserted into the wrong assistant turn — the conversation
                // visibly stalls. From the v126 log: 5 bash tools synthesized
                // then conversation died after first approval. Holding the
                // continuation here lets the user finish all approvals first.
                if app.pending_approval.is_none()
                    && app.approval_queue.is_empty()
                    && stream::should_continue_loop(&app.messages)
                {
                    tracing::info!(
                        target: "jfc::stream",
                        "agentic loop continuing — tools complete, no pending approvals"
                    );
                    stream::continue_agentic_loop(&mut app, &tx).await;
                } else if !app.is_streaming
                    && app.pending_approval.is_none()
                    && app.approval_queue.is_empty()
                    && app.pending_tool_calls.is_empty()
                {
                    tracing::debug!(
                        target: "jfc::stream",
                        "turn fully ended — draining queued prompts"
                    );
                    // Turn fully ended (model stopped, no more agentic loop
                    // iterations, no pending tools). v126 input queue: drain
                    // any prompts the user typed during streaming.
                    drain_queued_prompts(&mut app, &tx).await;
                }
            }
            AppEvent::CompactionStarted => {
                // Drives the `Compacting…` spinner — without this, the UI
                // freezes on a long pre-submit compact and the user
                // assumes their keystroke was eaten.
                tracing::debug!(target: "jfc::compact", "CompactionStarted event received — showing spinner");
                app.compacting_started_at = Some(std::time::Instant::now());
            }
            AppEvent::CompactionDone {
                messages,
                tool_ctx,
                pre_tokens,
                post_tokens,
            } => {
                let saved = pre_tokens.saturating_sub(post_tokens);
                tracing::info!(
                    target: "jfc::compact",
                    pre_tokens, post_tokens, saved,
                    new_message_count = messages.len(),
                    "applying compaction result to app state"
                );
                app.messages = messages;
                app.tool_ctx = tool_ctx;
                app.tool_ctx.approx_tokens = post_tokens;
                app.compacting_started_at = None;
                // Surface the compaction outcome to the user via a toast
                // — they don't have to scroll to see the boundary marker.
                let saved_k = saved / 1000;
                toast::push_with_cap(
                    &mut app.toasts,
                    toast::Toast::new(
                        toast::ToastKind::Success,
                        format!("Compacted — saved ~{saved_k}k tokens"),
                    ),
                );
            }
            AppEvent::CompactionFailed(reason, calibrated_tokens) => {
                tracing::warn!(
                    target: "jfc::compact",
                    %reason,
                    ?calibrated_tokens,
                    "compaction failed — surfacing toast to user"
                );
                if let Some(real_count) = calibrated_tokens {
                    app.tool_ctx.approx_tokens = real_count;
                }
                app.compacting_started_at = None;
                toast::push_with_cap(
                    &mut app.toasts,
                    toast::Toast::new(
                        toast::ToastKind::Error,
                        format!("Compaction failed: {reason}"),
                    ),
                );
            }
            AppEvent::Submit(text) => {
                // Re-fire after pre-submit compaction. Reuses the same
                // dispatch path as a typed prompt so message persistence,
                // streaming setup, and session save all run identically.
                tracing::debug!(
                    target: "jfc::input",
                    text_len = text.len(),
                    "AppEvent::Submit (re-queued after compaction)"
                );
                input::handle_submit_text(&mut app, text, &tx).await?;
            }
            AppEvent::Toast { kind, text } => {
                // Push onto the auto-expiring strip with the kind's
                // default TTL. Capped at `MAX_TOASTS` to bound memory
                // when a long-running compaction or classifier spams.
                toast::push_with_cap(&mut app.toasts, toast::Toast::new(kind, text));
            }
            AppEvent::AgentChunk { task_id, text } => {
                // Subagent emitted a streaming text chunk — append to its
                // task's message log so the task view shows live output
                // rather than the "No messages yet" empty state. v126
                // pipes nested-stream chunks the same way so the user
                // can drill into a running agent and see what it's doing.
                if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                    // Coalesce with the previous chunk when both came in
                    // rapid succession AND the previous entry doesn't end
                    // with a newline — so a single conceptual paragraph
                    // streamed across many chunks renders as one paragraph
                    // instead of one entry per delta.
                    let coalesce = bt
                        .messages
                        .last()
                        .map(|s| !s.ends_with('\n') && !s.starts_with('['))
                        .unwrap_or(false);
                    if coalesce {
                        if let Some(last) = bt.messages.last_mut() {
                            last.push_str(&text);
                        }
                    } else {
                        bt.messages.push(text);
                    }
                }
            }
            AppEvent::ModelsLoaded { provider, models } => {
                app.model_picker_query_cache.clear();
                app.provider_models.insert(provider, models);
                app.sync_selected_context_window();
                if app.show_model_picker {
                    app.model_picker_models = input::collect_all_models(&app);
                }
            }
            AppEvent::ProfileLoaded {
                seat_tier,
                subscription_type,
                email,
            } => {
                app.seat_tier = seat_tier;
                app.subscription_type = subscription_type;
                app.account_email = email;
                if app.show_model_picker {
                    app.model_picker_models = input::collect_all_models(&app);
                }
            }
            AppEvent::TaskStarted {
                task_id,
                description,
            } => {
                tracing::info!(
                    target: "jfc::task",
                    %task_id, %description,
                    "TaskStarted"
                );
                use types::{TaskLifecycle, TaskStatusPart};
                app.background_tasks.insert(
                    task_id.clone(),
                    app::BackgroundTask {
                        task_id: task_id.clone(),
                        description: description.clone(),
                        status: TaskLifecycle::Running,
                        started_at: std::time::Instant::now(),
                        summary: None,
                        error: None,
                        last_tool: None,
                        messages: Vec::new(),
                    },
                );
                let part = MessagePart::TaskStatus(TaskStatusPart {
                    task_id,
                    description,
                    status: TaskLifecycle::Running,
                    summary: None,
                    error: None,
                    elapsed_ms: None,
                });
                if let Some(idx) = app.streaming_assistant_idx {
                    if let Some(msg) = app.messages.get_mut(idx) {
                        msg.parts.push(part);
                    }
                } else if let Some(msg) = app.messages.last_mut() {
                    msg.parts.push(part);
                }
            }
            AppEvent::TaskProgress {
                task_id,
                last_tool,
                elapsed_ms,
            } => {
                if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                    if let Some(ref tool) = last_tool {
                        // Append a one-line activity entry to the task's
                        // message log so `messages_task_view` shows what
                        // the agent has done. Without this the task view
                        // renders "No messages yet" for the entire run.
                        // Full subagent StreamChunk routing is a bigger
                        // refactor; this is the minimum that makes the
                        // task view useful right now.
                        let elapsed_s = elapsed_ms / 1000;
                        bt.messages.push(format!("[{elapsed_s}s] {tool}"));
                    }
                    bt.last_tool = last_tool;
                }
                for msg in &mut app.messages {
                    for part in &mut msg.parts {
                        if let MessagePart::TaskStatus(ts) = part {
                            if ts.task_id == task_id {
                                ts.elapsed_ms = Some(elapsed_ms);
                            }
                        }
                    }
                }
            }
            AppEvent::TaskCompleted {
                task_id,
                summary,
                elapsed_ms,
            } => {
                tracing::info!(
                    target: "jfc::task",
                    %task_id, elapsed_ms,
                    summary_len = summary.len(),
                    "TaskCompleted"
                );
                use types::TaskLifecycle;
                if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                    bt.status = TaskLifecycle::Completed;
                    bt.summary = Some(summary.clone());
                    let elapsed_s = elapsed_ms / 1000;
                    bt.messages
                        .push(format!("[{elapsed_s}s] ✓ done — {summary}"));
                }
                for msg in &mut app.messages {
                    for part in &mut msg.parts {
                        if let MessagePart::TaskStatus(ts) = part {
                            if ts.task_id == task_id {
                                ts.status = TaskLifecycle::Completed;
                                ts.summary = Some(summary.clone());
                                ts.elapsed_ms = Some(elapsed_ms);
                            }
                        }
                    }
                }
            }
            AppEvent::TaskFailed { task_id, error } => {
                tracing::warn!(
                    target: "jfc::task",
                    %task_id,
                    error_preview = %&error[..error.len().min(200)],
                    "TaskFailed"
                );
                use types::TaskLifecycle;
                if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                    bt.status = TaskLifecycle::Failed;
                    bt.error = Some(error.clone());
                }
                for msg in &mut app.messages {
                    for part in &mut msg.parts {
                        if let MessagePart::TaskStatus(ts) = part {
                            if ts.task_id == task_id {
                                ts.status = TaskLifecycle::Failed;
                                ts.error = Some(error.clone());
                            }
                        }
                    }
                }
            }
        }

        app.sync_task_completions();
        terminal.draw(|f| render::frame(f, &mut app))?;
    }

    Ok(())
}

fn update_task_activities(app: &mut app::App, calls: &[types::ToolCall]) {
    let in_progress: Vec<tasks::TaskId> = app
        .task_store
        .list(tasks::DeletedFilter::Exclude)
        .iter()
        .filter(|t| matches!(t.status, tasks::TaskStatus::InProgress))
        .map(|t| t.id.clone())
        .collect();
    if in_progress.is_empty() {
        return;
    }
    let description = calls
        .iter()
        .map(|c| format!("{}: {}", c.kind.label(), c.input.summary()))
        .collect::<Vec<_>>()
        .join(", ");
    for tid in in_progress {
        app.task_activities.insert(tid, description.clone());
    }
}
