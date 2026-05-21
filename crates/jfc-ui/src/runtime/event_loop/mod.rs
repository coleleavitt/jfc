use std::{io, sync::Arc, time::Duration};

use crossterm::{cursor::SetCursorStyle, event, execute};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use crate::app::{ANIM_TICK_MS, App, IDLE_TICK_MS};
use crate::runtime::{
    APP_EVENT_BUFFER, AppEvent, EventReceiver, EventSender, GoalEvent, ProviderEvent, StreamEvent,
    StreamRequestOverrides, TaskEvent, TeamEvent, ToolEvent, UiEvent, draw_synchronized,
    handle_goal_verdict, restore_persistent_background_agents, set_terminal_title,
};
use crate::types::*;
use crate::{config, diagnostics_producer, lsp_client, session, slate, stream};
use jfc_provider::{ModelId, Provider, ProviderId};

mod guards;
mod handlers;
mod narration_retry;

pub(crate) async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    providers: Vec<Arc<dyn Provider>>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    oauth_handle: Option<Arc<crate::providers::AnthropicOAuthProvider>>,
    startup_session: crate::StartupSession,
    initial_prompt: Option<String>,
    initial_permission_mode: Option<crate::app::PermissionMode>,
) -> anyhow::Result<()> {
    let (tx, mut rx): (EventSender, EventReceiver) = mpsc::channel(APP_EVENT_BUFFER);
    // Make the channel reachable from non-Task code paths (bounty
    // solver/validator agents, future cron-triggered work) so they
    // emit the same TaskStarted/AgentChunk/TaskCompleted events the
    // fan UI + ctrl+X panel render. Mirrors register_active_provider.
    crate::tools::register_event_sender(tx.clone());
    tracing::info!(target: "jfc::ui::events", "registered AppEvent sender for non-Task agent paths");
    let mut app = App::new(provider, model);
    app.providers = providers.clone();
    crate::claude_status::spawn_status_poll(tx.clone());
    // v141 parity: when the caller passed `--permission-mode`, apply
    // it before any user prompt so the first turn already runs under
    // the requested mode. Without this the user would have to
    // Shift+Tab inside the TUI on every boot.
    if let Some(mode) = initial_permission_mode {
        tracing::info!(
            target: "jfc::ui",
            ?mode,
            "applying --permission-mode at startup"
        );
        app.permission_mode = mode;
    }
    // Apply the user's persisted theme choice from
    // ~/.config/jfc/config.toml. Unknown / missing names fall back
    // silently to the default dark theme set by App::new.
    if let Some(name) = crate::config::load().theme.as_deref()
        && let Some(theme) = crate::theme::Theme::by_name(name)
    {
        tracing::info!(target: "jfc::ui::theme", theme = %name, "applied persisted theme");
        app.theme = theme;
        // The render cache stores `Vec<Line<'static>>` with syntect highlight
        // colors baked in from the previous theme. Switching themes without
        // invalidating would serve stale-colored lines until each entry is
        // naturally evicted by the LRU. At boot the cache is empty so this is
        // a no-op, but we keep symmetry with the `/theme` handler so future
        // refactors don't introduce a regression.
        tracing::debug!(target: "jfc::render::cache", "theme switch — invalidating cache");
        app.render_cache.borrow_mut().clear();
        crate::markdown::clear_highlight_cache();
    }
    if let Some(name) = crate::config::load().output_style.as_deref() {
        let parsed = crate::output_style::OutputStyle::from_str_loose(name);
        tracing::info!(
            target: "jfc::ui::output_style",
            style = %parsed.name(),
            "applied persisted output style"
        );
        app.output_style = parsed;
        crate::output_style::set_active(parsed);
    }

    // v132 Finch onboarding — first-run UI for users with no prior
    // session. Drops the help overlay automatically so they see the
    // keybindings + slash command catalog before typing. Suppressed
    // when the Finch feature gate is off (default for established
    // users). The gate flips itself off after the first successful
    // turn so the overlay doesn't repeat.
    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Finch) {
        let session_dir_empty = std::fs::read_dir(jfc_session::sessions_dir())
            .map(|mut it| it.next().is_none())
            .unwrap_or(true);
        if session_dir_empty {
            app.show_help = true;
            tracing::info!(
                target: "jfc::onboarding",
                "Finch onboarding active — showing help overlay"
            );
        }
    }

    // Wire the Slate router from config. Default OFF — `slate_enabled = false`
    // in `~/.config/jfc/config.toml` means `app.slate = None` and every turn
    // uses the pinned `app.model` (legacy behavior). When ON, each user
    // submission consults the router to pick a per-turn model based on the
    // classifier's `QueryClass`. See `crates/jfc-ui/src/slate.rs`.
    {
        let cfg = config::load();
        if cfg.slate_enabled {
            let rules = config::slate_rules_from_config(&cfg);
            let rule_count = rules.len();
            let router = slate::SlateRouter::new(rules);
            tracing::info!(
                target: "jfc::slate",
                rule_count,
                "slate router enabled"
            );
            app.slate = Some(router);
        } else {
            tracing::debug!(
                target: "jfc::slate",
                "slate router disabled (default) — every turn uses pinned model"
            );
        }
    }

    // Handle --continue / --resume flags
    match startup_session {
        crate::StartupSession::Fresh => {}
        crate::StartupSession::Continue => {
            // `--continue` is cwd-scoped (codex-rs / v126 parity). The
            // user can pass `--continue --global` later if we add the
            // flag; for now the cwd default is what they actually want.
            let cwd_str = std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string());
            let id = match jfc_session::most_recent_session_for_cwd(cwd_str.as_deref()).await {
                Some(id) => Some(id),
                None => jfc_session::most_recent_session().await, // legacy fallback
            };
            if let Some(session_id) = id
                && let Some((messages, saved_model)) =
                    session::load_session_with_model(&session_id).await
            {
                tracing::info!(
                    target: "jfc::session",
                    session_id = %session_id,
                    message_count = messages.len(),
                    saved_model = ?saved_model,
                    cwd = ?cwd_str,
                    "continuing most recent session"
                );
                app.messages = messages;
                app.current_session_id = Some(session_id.clone());
                // Re-open task store so tasks from the resumed session are loaded.
                app.task_store = jfc_session::TaskStore::open(session_id.as_str());
                // Rebuild any active stop-condition from the goal
                // sidecar — without this, /continue forgets the
                // user's goal and the next EndTurn settles silently.
                if let Some(goal) = crate::goal::load_sidecar(session_id.as_str()) {
                    tracing::info!(
                        target: "jfc::goal",
                        session_id = %session_id,
                        condition = %goal.condition,
                        iterations = goal.iterations,
                        "restored goal from sidecar"
                    );
                    app.goal = Some(goal);
                }
                if let Some(model_id) = saved_model {
                    if let Some(p) = crate::provider_for_model(&app.providers, &model_id) {
                        tracing::info!(
                            target: "jfc::session",
                            model = %model_id,
                            routed_provider = %p.name(),
                            "rerouting active provider to match saved session model"
                        );
                        app.provider = p;
                    }
                    app.model = model_id.into();
                }
                app.recompute_token_estimate();
                // Warm the tool-height cache so the first render frame
                // doesn't visibly spike — without this the first paint
                // computes heights for every terminal-state tool from
                // scratch (each height = one full `tool_body_lines`
                // build), and on a 100-tool conversation that's a
                // noticeable hitch right after the UI appears. We use
                // the current terminal width as the best-guess inner
                // width; render::messages may use a slightly different
                // value (sidebars open/closed) but mismatched widths
                // just produce a few extra cache entries — correctness
                // is unaffected and they get evicted by the LRU.
                if let Ok((cols, _rows)) = crossterm::terminal::size() {
                    let inner_w = (cols as usize).saturating_sub(5);
                    crate::message_view::warm_tool_height_cache_for_messages(
                        &app.messages,
                        inner_w,
                    );
                }
            }
        }
        crate::StartupSession::Resume(session_id) => {
            let session_id = crate::ids::SessionId::new(session_id);
            if let Some((messages, saved_model)) =
                session::load_session_with_model(&session_id).await
            {
                tracing::info!(
                    target: "jfc::session",
                    session_id = %session_id,
                    message_count = messages.len(),
                    saved_model = ?saved_model,
                    "resuming specific session"
                );
                let session_cwd = jfc_session::load_session_metadata(&session_id)
                    .await
                    .and_then(|m| m.cwd);
                let current_cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(msg) =
                    jfc_session::cwd_mismatch_message(session_cwd.as_deref(), &current_cwd)
                {
                    tracing::warn!(
                        target: "jfc::session",
                        session_id = %session_id,
                        "{msg}"
                    );
                }
                app.messages = messages;
                app.current_session_id = Some(session_id.clone());
                // Re-open task store so tasks from the resumed session are loaded.
                app.task_store = jfc_session::TaskStore::open(session_id.as_str());
                // Rebuild any active stop-condition from the goal sidecar.
                if let Some(goal) = crate::goal::load_sidecar(session_id.as_str()) {
                    tracing::info!(
                        target: "jfc::goal",
                        session_id = %session_id,
                        condition = %goal.condition,
                        iterations = goal.iterations,
                        "restored goal from sidecar"
                    );
                    app.goal = Some(goal);
                }
                if let Some(model_id) = saved_model {
                    if let Some(p) = crate::provider_for_model(&app.providers, &model_id) {
                        tracing::info!(
                            target: "jfc::session",
                            model = %model_id,
                            routed_provider = %p.name(),
                            "rerouting active provider to match saved session model"
                        );
                        app.provider = p;
                    }
                    app.model = model_id.into();
                }
                app.recompute_token_estimate();
                // Same cache warm-up as the --continue branch — see comment
                // there. Pre-paying the height computation here means the
                // first render frame doesn't hitch.
                if let Ok((cols, _rows)) = crossterm::terminal::size() {
                    let inner_w = (cols as usize).saturating_sub(5);
                    crate::message_view::warm_tool_height_cache_for_messages(
                        &app.messages,
                        inner_w,
                    );
                }
            } else {
                tracing::warn!(
                    target: "jfc::session",
                    session_id = %session_id,
                    "session not found, starting fresh"
                );
            }
        }
    }
    restore_persistent_background_agents(&mut app);

    // Apply persisted reasoning_effort from config.toml. MUST run AFTER
    // the --continue/--resume block above (which may switch `app.model` to
    // the session's saved model) so the effort resolves for the ACTUAL
    // model in use, not the initial CLI-provided one.
    {
        let cfg = crate::config::load();
        let effort_str = resolve_effort_for_model(&cfg, &app.model);
        if let Some(level) = effort_str
            .as_deref()
            .and_then(crate::effort::ReasoningEffort::from_str_loose)
        {
            tracing::info!(
                target: "jfc::ui::effort",
                effort = %level,
                model = %app.model,
                "applied persisted reasoning_effort (post-session-restore)"
            );
            app.effort_state.set(level);
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
            let _ = tx
                .send(AppEvent::Provider(ProviderEvent::ModelsLoaded {
                    provider: name,
                    models,
                }))
                .await;
        });
    }

    // Kick off OAuth profile fetch — needed for v126-equivalent seat-tier model gating
    // (XwH() in cli.js) and for showing the subscription type / email in the status bar.
    // Best-effort: a failure here just leaves seat_tier None, which means "no filter".
    let oauth_for_snapshot = oauth_handle.clone();
    if let Some(oauth) = oauth_handle {
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(profile) = oauth.fetch_profile().await {
                let _ = tx
                    .send(AppEvent::Provider(ProviderEvent::ProfileLoaded {
                        seat_tier: profile.seat_tier,
                        subscription_type: profile.subscription_type,
                        email: profile.email,
                    }))
                    .await;
            }
        });
    }

    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = event::EventStream::new();
            while let Some(Ok(ev)) = reader.next().await {
                let _ = tx.send(AppEvent::Ui(UiEvent::Term(ev))).await;
            }
        });
    }

    {
        let tx = tx.clone();
        let wants_anim = app.wants_animation_frame.clone();
        tokio::spawn(async move {
            loop {
                let ms = if wants_anim.load(std::sync::atomic::Ordering::Relaxed) {
                    ANIM_TICK_MS
                } else {
                    IDLE_TICK_MS
                };
                tokio::time::sleep(Duration::from_millis(ms)).await;
                let _ = tx.try_send(AppEvent::Ui(UiEvent::Tick));
            }
        });
    }

    // Forward teammate runner events into the main event channel.
    {
        let tx = tx.clone();
        let mut teammate_rx = app.teammate_event_rx.take().unwrap();
        tokio::spawn(async move {
            while let Some(ev) = teammate_rx.recv().await {
                let _ = tx.send(AppEvent::Team(TeamEvent::Runner(ev))).await;
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
    // into `ProviderEvent::DiagnosticsUpdated`. Gated by `JFC_DISABLE_LSP=1`.
    // `maybe_spawn_lsp_clients` is fire-and-forget — startup never
    // blocks on the handshake. If the binary isn't on PATH, the spawn
    // task silently returns and we fall back to the cargo-check
    // producer above.
    {
        let tx_lsp = tx.clone();
        let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
        lsp_client::maybe_spawn_lsp_clients(cwd, tx_lsp);
    }

    // MCP servers from `[mcp.<name>]` config blocks. Spawn happens in
    // a background task so startup isn't blocked by a slow `npx install`
    // — the streaming layer pulls advertised tools dynamically via
    // `tools::all_tool_defs_with_mcp()` so the model sees servers as
    // soon as they finish handshaking. Gated by `JFC_DISABLE_MCP=1`.
    {
        let registry = crate::mcp::McpRegistry::new();
        crate::tools::register_mcp_registry(registry.clone());
        let mcp_configs = crate::config::load().mcp;
        tokio::spawn(async move {
            crate::mcp::register_servers_from_config(&registry, &mcp_configs).await;
        });
    }

    app.sync_task_completions();
    draw_synchronized(terminal, &mut app)?;
    // Initial terminal title — updates whenever the model or session
    // changes.
    set_terminal_title(&app);

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
        app.last_stream_event_at = Some(now);
        app.streaming_last_token_at = Some(now);
        app.turn_started_at = Some(now);
        app.turn_start_cost = crate::cost::total_cost(&app.usage_by_model);
        app.last_usage_output = 0;
        app.usage_apply_baseline = (0, 0, 0, 0);

        // Create session if not resuming one
        let session_id = app
            .current_session_id
            .clone()
            .unwrap_or_else(jfc_session::generate_session_id);
        {
            let sid = session_id.clone();
            let msgs = app.messages.clone();
            let cwd = app.cwd.clone();
            let model = app.model.clone();
            tokio::spawn(async move {
                session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
            });
        }
        app.current_session_id = Some(session_id.clone());

        let provider = app.provider.clone();
        let messages = stream::build_provider_messages(&app.messages[..assistant_idx]);
        // Slate per-turn routing for the `--prompt` startup path.
        let model = if let Some(ref router) = app.slate {
            router.route(&prompt, app.model.clone())
        } else {
            app.model.clone()
        };
        let tx_clone = tx.clone();
        let interrupt = app.interrupt_flag.clone();
        // wg-async: --prompt startup spawns a stream that holds critical
        // state (SSE conn + tx). Wire the cancel token in so an early
        // ESC can drop it cleanly.
        app.cancel_token = tokio_util::sync::CancellationToken::new();
        let cancel = app.cancel_token.clone();
        let prev_msg_id = app.last_response_id.take();
        let overrides = StreamRequestOverrides {
            background_reminders: app.take_background_reminders(),
            ..Default::default()
        };
        let tx_guard = tx.clone();
        // Outer JoinHandle parked on App so the watchdog can forcefully
        // abort a wedged stream task (see App::active_stream_handle).
        let handle = tokio::spawn(async move {
            let result = tokio::spawn(async move {
                stream::stream_response(
                    provider,
                    messages,
                    model,
                    tx_clone,
                    interrupt,
                    cancel,
                    prev_msg_id,
                    overrides,
                )
                .await;
            })
            .await;
            if let Err(join_err) = result {
                let msg = if join_err.is_panic() {
                    format!("stream task panicked: {join_err}")
                } else {
                    format!("stream task cancelled: {join_err}")
                };
                let _ = tx_guard
                    .send(AppEvent::Stream(StreamEvent::Error(msg)))
                    .await;
            }
        });
        app.active_stream_handle = Some(handle);
    }

    // Track when we last drew to implement frame-rate limiting.
    // The UI only redraws at most once per IDLE_TICK_MS (80ms = 12.5 FPS idle,
    // but input events always get a draw). This prevents the render loop
    // from starving input processing when 100s of StreamChunk events/sec
    // flood the channel during fast streaming.
    // Frame-rate cap: ~120 FPS upper bound (8ms minimum between draws). Bursts
    // of events from streaming (StreamChunk fires per token) coalesce into one
    // draw — the user's terminal can't keep up with 1000+ FPS anyway and each
    // unnecessary `Backend::flush` is a synchronous stdout write.
    const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);
    let mut last_draw = std::time::Instant::now();
    let mut pending_draw = false;

    'main_loop: loop {
        // Burst-recv: block on the first event, then drain everything currently
        // queued without re-awaiting. Process them all, draw once at the end.
        // This collapses N rapid stream chunks into 1 frame instead of N frames.
        let mut events: Vec<AppEvent> = match rx.recv().await {
            Some(e) => vec![e],
            None => break,
        };
        while let Ok(extra) = rx.try_recv() {
            events.push(extra);
        }

        // Track whether any event in this burst dirties the screen. Pure Tick
        // events with no streaming/animation skip the draw entirely — eliminates
        // ~12.5 idle redraws per second.
        let mut needs_draw = std::mem::take(&mut pending_draw);
        let mut should_quit = false;

        for ev in events {
            // Tick alone doesn't dirty the screen; everything else does. The
            // streaming-animation guard below re-enables Tick-driven redraws
            // when there's actually motion to show.
            if !ev.is_tick() {
                needs_draw = true;
            }

            match ev {
                // ── Terminal input (key, paste, mouse) ───────────────────
                AppEvent::Ui(UiEvent::Term(ev)) => {
                    if handlers::input::handle_term_event(&mut app, ev, &tx).await? {
                        should_quit = true;
                        break;
                    }
                }

                // ── Team events ─────────────────────────────────────────
                AppEvent::Team(ev) => {
                    handlers::team::handle_team_event(&mut app, &tx, ev).await;
                }

                // ── Tick ────────────────────────────────────────────────
                AppEvent::Ui(UiEvent::Tick) => {
                    if handlers::tick::handle_tick(&mut app, &tx, oauth_for_snapshot.as_ref()).await
                    {
                        needs_draw = true;
                    }
                }

                // ── Stream: chunk / tool-input / redacted / response-id ─
                AppEvent::Stream(StreamEvent::Chunk { text, reasoning }) => {
                    handlers::stream_chunk::handle_chunk(&mut app, text, reasoning);
                }
                AppEvent::Stream(StreamEvent::ToolInputDelta(byte_len)) => {
                    handlers::stream_chunk::handle_tool_input_delta(&mut app, byte_len);
                }
                AppEvent::Stream(StreamEvent::RedactedThinking(data)) => {
                    handlers::stream_chunk::handle_redacted_thinking(&mut app, data);
                }
                AppEvent::Stream(StreamEvent::ResponseId(id)) => {
                    handlers::stream_chunk::handle_response_id(&mut app, id);
                }

                // ── Stream: tool announcement ───────────────────────────
                AppEvent::Stream(StreamEvent::Tool(tool)) => {
                    handlers::stream_tool::handle_stream_tool(&mut app, &tx, tool).await;
                }
                AppEvent::Tool(ToolEvent::ClassifierDecision {
                    tool,
                    blocked,
                    reason,
                }) => {
                    handlers::stream_tool::handle_classifier_decision(
                        &mut app, &tx, tool, blocked, reason,
                    )
                    .await;
                }
                AppEvent::Stream(StreamEvent::ServerToolResult {
                    tool_use_id,
                    tool_kind,
                    content,
                }) => {
                    handlers::stream_tool::handle_server_tool_result(
                        &mut app,
                        tool_use_id,
                        tool_kind,
                        content,
                    );
                }

                // ── Stream: done ────────────────────────────────────────
                AppEvent::Stream(StreamEvent::Done(stop_reason)) => {
                    handlers::stream_done::handle_stream_done(&mut app, &tx, stop_reason).await;
                }

                // ── Stream: error ───────────────────────────────────────
                AppEvent::Stream(StreamEvent::Error(e)) => {
                    handlers::stream_error::handle_stream_error(&mut app, &tx, e).await;
                }

                // ── Stream: usage ───────────────────────────────────────
                AppEvent::Stream(StreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                }) => {
                    handlers::stream_usage::handle_stream_usage(
                        &mut app,
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                    );
                }

                // ── Stream: metadata ────────────────────────────────────
                AppEvent::Stream(StreamEvent::SystemPromptLen(len)) => {
                    handlers::ui_actions::handle_system_prompt_len(&mut app, len);
                }
                AppEvent::Stream(StreamEvent::RequestMetadata(meta)) => {
                    handlers::ui_actions::handle_request_metadata(&mut app, meta);
                }

                // ── Provider events ─────────────────────────────────────
                AppEvent::Provider(ev) => {
                    handlers::provider::handle_provider_event(&mut app, ev);
                }

                // ── Tool execution events ───────────────────────────────
                AppEvent::Tool(ToolEvent::OutputChunk { tool_id, chunk }) => {
                    handlers::tools::handle_output_chunk(&mut app, tool_id, chunk);
                }
                AppEvent::Tool(ToolEvent::Result { tool_id, result }) => {
                    handlers::tools::handle_tool_result(&mut app, tool_id, result);
                }
                AppEvent::Tool(ToolEvent::AllComplete) => {
                    handlers::tools::handle_all_complete(&mut app, &tx).await;
                }

                // ── Goal evaluation ─────────────────────────────────────
                AppEvent::Goal(GoalEvent::Verdict { ok, reason }) => {
                    handle_goal_verdict(&mut app, &tx, ok, reason).await;
                }

                // ── Compaction events ───────────────────────────────────
                AppEvent::Compaction(ev) => {
                    handlers::compaction::handle_compaction_event(&mut app, &tx, ev).await;
                }

                // ── UI actions ──────────────────────────────────────────
                AppEvent::Ui(UiEvent::EnterPlanModeRequested { reason }) => {
                    handlers::ui_actions::handle_enter_plan_mode(&mut app, reason);
                }
                AppEvent::Ui(UiEvent::Submit(text)) => {
                    handlers::ui_actions::handle_submit(&mut app, text, &tx).await?;
                }
                AppEvent::Ui(UiEvent::Toast { kind, text }) => {
                    handlers::ui_actions::handle_toast(&mut app, kind, text);
                }
                AppEvent::Ui(UiEvent::LoadSession(session_id)) => {
                    handlers::ui_actions::handle_load_session(&mut app, session_id).await;
                }
                AppEvent::Ui(UiEvent::ExitPlanModeRequested { plan }) => {
                    handlers::ui_actions::handle_exit_plan_mode(&mut app, plan);
                }

                // ── Task (subagent) events ──────────────────────────────
                AppEvent::Task(TaskEvent::AgentChunk { task_id, text }) => {
                    handlers::task::handle_agent_chunk(&mut app, task_id, text);
                }
                AppEvent::Task(TaskEvent::Started {
                    task_id,
                    description,
                    model_used,
                    max_input_tokens,
                    is_detached,
                    parent_task_id,
                }) => {
                    handlers::task::handle_task_started(
                        &mut app,
                        task_id,
                        description,
                        model_used,
                        max_input_tokens,
                        is_detached,
                        parent_task_id,
                    );
                }
                AppEvent::Task(TaskEvent::Progress {
                    task_id,
                    last_tool,
                    elapsed_ms,
                    tool_use_count,
                    input_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    output_tokens,
                }) => {
                    handlers::task::handle_task_progress(
                        &mut app,
                        task_id,
                        last_tool,
                        elapsed_ms,
                        tool_use_count,
                        input_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        output_tokens,
                    );
                }
                AppEvent::Task(TaskEvent::Completed {
                    task_id,
                    summary,
                    elapsed_ms,
                }) => {
                    handlers::task::handle_task_completed(
                        &mut app, &tx, task_id, summary, elapsed_ms,
                    )
                    .await;
                }
                AppEvent::Task(TaskEvent::Failed { task_id, error }) => {
                    handlers::task::handle_task_failed(&mut app, &tx, task_id, error).await;
                }
            }
        }

        if should_quit {
            break 'main_loop;
        }

        // Streaming/compaction needs continuous redraws to show progress
        // (border-comet animation, spinner). Re-arm the dirty flag so a
        // bare Tick can drive the next frame. Also re-arm when tools are
        // pending or approval is active — without this, the screen stalls
        // between StreamDone and the next stream start (the user has to
        // move their cursor to trigger a redraw).
        let want_streaming_cursor = app.is_streaming
            || app.compacting_started_at.is_some()
            || !app.pending_tool_calls.is_empty()
            || app.pending_approval.is_some()
            || !app.approval_queue.is_empty()
            || app.background_tasks.values().any(|bt| bt.status.is_alive())
            || app.turn_started_at.is_some();
        if want_streaming_cursor {
            needs_draw = true;
        }

        let elapsed_since_draw = last_draw.elapsed();
        if needs_draw && elapsed_since_draw >= FRAME_BUDGET {
            // `terminal.draw` flushes stdout synchronously; `block_in_place`
            // tells the multi-threaded runtime to migrate other tasks off this
            // worker so they keep running while we hold the I/O.
            tokio::task::block_in_place(|| -> io::Result<()> {
                app.sync_task_completions();
                draw_synchronized(terminal, &mut app)?;
                set_terminal_title(&app);
                let _ = execute!(
                    io::stdout(),
                    if want_streaming_cursor {
                        SetCursorStyle::SteadyBlock
                    } else {
                        SetCursorStyle::BlinkingUnderScore
                    }
                );
                Ok(())
            })?;
            last_draw = std::time::Instant::now();
        } else if needs_draw {
            // Preserve dirty state across the frame cap. Without this, a final
            // StreamDone/TaskCompleted event that lands immediately after a
            // draw can be skipped, then the following idle Tick does not dirty
            // the screen because streaming has ended. The user only sees the
            // completed state after pressing a key.
            pending_draw = true;
        }
    }

    Ok(())
}

/// Walk the config to pick the right `reasoning_effort` for `model`.
///
/// Precedence (first hit wins):
///   1. `[agents.<exact-model-id>]` — direct match on the full model id
///   2. `[agents.<bare-model-id>]` — match the model id without provider prefix
///   3. `[default]` — fallback effort if no agent block matches
///
/// Returns `None` when none of those layers define an effort, so we leave
/// the runtime at "server default" instead of forcing medium.
fn resolve_effort_for_model(cfg: &crate::config::Config, model: &str) -> Option<String> {
    // 1: exact model id (e.g. "anthropic/claude-opus-4-7")
    if let Some(agent) = cfg.agents.get(model)
        && let Some(ref e) = agent.reasoning_effort
    {
        return Some(e.clone());
    }
    // 2: bare id after the provider slash (e.g. "claude-opus-4-7")
    let bare = model.rsplit('/').next().unwrap_or(model);
    if bare != model
        && let Some(agent) = cfg.agents.get(bare)
        && let Some(ref e) = agent.reasoning_effort
    {
        return Some(e.clone());
    }
    // 3: [default] block
    cfg.default.reasoning_effort.clone()
}

#[cfg(test)]
mod effort_resolve_tests {
    use super::*;
    use crate::config::{AgentConfig, Config};

    fn cfg_with(default_effort: Option<&str>, agents: &[(&str, &str)]) -> Config {
        let mut cfg = Config::default();
        cfg.default.reasoning_effort = default_effort.map(String::from);
        for (name, effort) in agents {
            cfg.agents.insert(
                (*name).to_string(),
                AgentConfig {
                    reasoning_effort: Some((*effort).to_string()),
                    ..Default::default()
                },
            );
        }
        cfg
    }

    #[test]
    fn falls_back_to_default_when_no_agent_match_normal() {
        let cfg = cfg_with(Some("high"), &[]);
        assert_eq!(
            resolve_effort_for_model(&cfg, "anthropic/claude-opus-4-7"),
            Some("high".to_string())
        );
    }

    #[test]
    fn exact_qualified_match_wins_over_default_normal() {
        let cfg = cfg_with(Some("low"), &[("anthropic/claude-opus-4-7", "max")]);
        assert_eq!(
            resolve_effort_for_model(&cfg, "anthropic/claude-opus-4-7"),
            Some("max".to_string())
        );
    }

    #[test]
    fn bare_model_match_wins_over_default_normal() {
        let cfg = cfg_with(Some("low"), &[("claude-opus-4-7", "xhigh")]);
        assert_eq!(
            resolve_effort_for_model(&cfg, "anthropic/claude-opus-4-7"),
            Some("xhigh".to_string())
        );
    }

    #[test]
    fn returns_none_when_nothing_configured_robust() {
        let cfg = cfg_with(None, &[]);
        assert_eq!(
            resolve_effort_for_model(&cfg, "anthropic/claude-opus-4-7"),
            None
        );
    }
}
