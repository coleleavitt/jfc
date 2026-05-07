use std::{io, sync::Arc, time::Duration};

use crossterm::{
    cursor::SetCursorStyle,
    event::{self, Event, KeyEventKind},
    execute,
    terminal::{BeginSynchronizedUpdate, EndSynchronizedUpdate, SetTitle},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use crate::app::{self, App, AppEvent, PendingApproval, TICK_MS};
use crate::provider::{ModelId, Provider, ProviderId};
use crate::types::*;
use crate::{
    attachments, config, diagnostics_producer, input, lsp_client, message_view, render,
    session, slate, stream, tasks, toast, types,
};

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

async fn drain_queued_prompts(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
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
        input::run_slash_command(app, &text).await;
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
    // Slate per-turn routing for the queued-prompt drain path. Mirrors
    // `input::handle_submit` so a queued prompt sees the same routing as a
    // freshly typed one — without this, queued submissions silently bypassed
    // Slate and used `app.model`.
    let model = if let Some(ref router) = app.slate {
        router.route(&text, app.model.clone())
    } else {
        app.model.clone()
    };
    let tx = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    tokio::spawn(async move {
        stream::stream_response(provider, messages, model, tx, interrupt).await;
    });
}

pub(crate) async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    providers: Vec<Arc<dyn Provider>>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    oauth_handle: Option<Arc<crate::providers::AnthropicOAuthProvider>>,
    startup_session: super::StartupSession,
    initial_prompt: Option<String>,
) -> anyhow::Result<()> {
    // Bounded channel capacity for the main AppEvent loop. 1024 accommodates
    // typical streaming bursts (50-200 chunks) with headroom for concurrent tool
    // result floods, while bounding memory at ~1024 × sizeof(AppEvent).
    const APP_EVENT_BUFFER: usize = 1024;
    let (tx, mut rx) = mpsc::channel::<AppEvent>(APP_EVENT_BUFFER);
    // Make the channel reachable from non-Task code paths (bounty
    // solver/validator agents, future cron-triggered work) so they
    // emit the same TaskStarted/AgentChunk/TaskCompleted events the
    // fan UI + ctrl+X panel render. Mirrors register_active_provider.
    crate::tools::register_event_sender(tx.clone());
    tracing::info!(target: "jfc::ui::events", "registered AppEvent sender for non-Task agent paths");
    let mut app = App::new(provider, model);
    app.providers = providers.clone();
    // Apply the user's persisted theme choice from
    // ~/.config/jfc/config.toml. Unknown / missing names fall back
    // silently to the default dark theme set by App::new.
    if let Some(name) = crate::config::load().theme.as_deref()
        && let Some(theme) = crate::theme::Theme::by_name(name)
    {
        tracing::info!(target: "jfc::ui::theme", theme = %name, "applied persisted theme");
        app.theme = theme;
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
        let session_dir_empty = std::fs::read_dir(crate::session::sessions_dir())
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
        super::StartupSession::Fresh => {}
        super::StartupSession::Continue => {
            // `--continue` is cwd-scoped (codex-rs / v126 parity). The
            // user can pass `--continue --global` later if we add the
            // flag; for now the cwd default is what they actually want.
            let cwd_str = std::env::current_dir()
                .ok()
                .map(|p| p.display().to_string());
            let id = match session::most_recent_session_for_cwd(cwd_str.as_deref()).await {
                Some(id) => Some(id),
                None => session::most_recent_session().await, // legacy fallback
            };
            if let Some(session_id) = id {
                if let Some((messages, saved_model)) =
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
                    app.task_store = crate::tasks::TaskStore::open(&session_id);
                    if let Some(model_id) = saved_model {
                        if let Some(p) = super::provider_for_model(&app.providers, &model_id) {
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
                }
            }
        }
        super::StartupSession::Resume(session_id) => {
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
                let session_cwd = session::load_session_metadata(&session_id)
                    .await
                    .and_then(|m| m.cwd);
                let current_cwd = std::env::current_dir()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if let Some(msg) =
                    session::cwd_mismatch_message(session_cwd.as_deref(), &current_cwd)
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
                app.task_store = crate::tasks::TaskStore::open(&session_id);
                if let Some(model_id) = saved_model {
                    if let Some(p) = super::provider_for_model(&app.providers, &model_id) {
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
            let _ = tx
                .send(AppEvent::ModelsLoaded {
                    provider: name,
                    models,
                })
                .await;
        });
    }

    // Kick off OAuth profile fetch — needed for v126-equivalent seat-tier model gating
    // (XwH() in cli.js) and for showing the subscription type / email in the status bar.
    // Best-effort: a failure here just leaves seat_tier None, which means "no filter".
    if let Some(oauth) = oauth_handle {
        let tx = tx.clone();
        tokio::spawn(async move {
            if let Ok(profile) = oauth.fetch_profile().await {
                let _ = tx
                    .send(AppEvent::ProfileLoaded {
                        seat_tier: profile.seat_tier,
                        subscription_type: profile.subscription_type,
                        email: profile.email,
                    })
                    .await;
            }
        });
    }

    {
        let tx = tx.clone();
        tokio::spawn(async move {
            let mut reader = event::EventStream::new();
            while let Some(Ok(ev)) = reader.next().await {
                let _ = tx.send(AppEvent::Term(ev)).await;
            }
        });
    }

    {
        let tx = tx.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_millis(TICK_MS)).await;
                // Tick is non-critical; safe to drop — next tick arrives shortly.
                let _ = tx.try_send(AppEvent::Tick);
            }
        });
    }

    // Forward teammate runner events into the main event channel.
    {
        let tx = tx.clone();
        let mut teammate_rx = app.teammate_event_rx.take().unwrap();
        tokio::spawn(async move {
            while let Some(ev) = teammate_rx.recv().await {
                let _ = tx.send(AppEvent::TeammateEvent(ev)).await;
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
        app.streaming_last_token_at = Some(now);
        app.turn_started_at = Some(now);
        app.last_usage_output = 0;
        app.usage_apply_baseline = (0, 0, 0, 0);

        // Create session if not resuming one
        let session_id = app
            .current_session_id
            .clone()
            .unwrap_or_else(session::generate_session_id);
        {
            let sid = session_id.clone();
            let msgs = app.messages.clone();
            let cwd = app.cwd.clone();
            let model = app.model.clone();
            tokio::spawn(async move {
                session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
            });
        }
        app.current_session_id = Some(session_id);

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
        tokio::spawn(async move {
            stream::stream_response(provider, messages, model, tx_clone, interrupt).await;
        });
    }

    // Track when we last drew to implement frame-rate limiting.
    // The UI only redraws at most once per TICK_MS (80ms = 12.5 FPS idle,
    // but input events always get a draw). This prevents the render loop
    // from starving input processing when 100s of StreamChunk events/sec
    // flood the channel during fast streaming.
    // Frame-rate cap: ~120 FPS upper bound (8ms minimum between draws). Bursts
    // of events from streaming (StreamChunk fires per token) coalesce into one
    // draw — the user's terminal can't keep up with 1000+ FPS anyway and each
    // unnecessary `Backend::flush` is a synchronous stdout write.
    const FRAME_BUDGET: std::time::Duration = std::time::Duration::from_millis(8);
    let mut last_draw = std::time::Instant::now();

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
        let mut needs_draw = false;
        let mut should_quit = false;

        for ev in events {
            // Tick alone doesn't dirty the screen; everything else does. The
            // streaming-animation guard below re-enables Tick-driven redraws
            // when there's actually motion to show.
            if !matches!(ev, AppEvent::Tick) {
                needs_draw = true;
            }

            match ev {
                // Accept Press *and* Repeat so holding ↑/↓ keeps moving in the picker.
                // The kitty keyboard protocol (enabled via REPORT_EVENT_TYPES at startup)
                // delivers separate Repeat events while a key is held — without this filter
                // they would be discarded. Release events still fall through.
                AppEvent::Term(Event::Key(k))
                    if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) =>
                {
                    if input::handle_key(&mut app, k, &tx).await? {
                        should_quit = true;
                        break;
                    }
                }
                AppEvent::Term(Event::Paste(text)) => {
                    // Try image clipboard first — when the user pastes a
                    // screenshot the OS sends a bracketed-paste *event*
                    // with empty/garbage text, but the actual image is
                    // available via arboard's `get_image()`. If that
                    // succeeds we attach it; otherwise fall through to
                    // the text path. Mirrors v126's clipboard-image flow.
                    let attached_image = match attachments::read_clipboard_image() {
                        Ok(Some(att)) => {
                            toast::push_with_cap(
                                &mut app.toasts,
                                toast::Toast::new(
                                    toast::ToastKind::Info,
                                    format!("📎 image attached ({} bytes)", att.bytes.len()),
                                ),
                            );
                            app.pending_attachments.push(att);
                            true
                        }
                        Ok(None) => false,
                        Err(e) => {
                            tracing::debug!(target: "jfc::input", error = %e, "image paste check failed");
                            false
                        }
                    };
                    // Always insert the text — it may be a path or
                    // contextual prose alongside the image.
                    if !attached_image || !text.is_empty() {
                        app.textarea.insert_str(&text);
                    }
                }
                AppEvent::Term(Event::Mouse(mouse)) => {
                    use crossterm::event::{MouseButton, MouseEventKind};
                    match mouse.kind {
                        MouseEventKind::ScrollUp => app.scroll_up(3),
                        MouseEventKind::ScrollDown => app.scroll_down(3),
                        MouseEventKind::Drag(MouseButton::Left) => {
                            // Drag-scroll: convert vertical delta into
                            // scroll_offset adjustments. Anchor on the
                            // first Drag event and re-anchor on every
                            // subsequent one so the next delta is one
                            // row's worth, not cumulative. Up-drag scrolls
                            // up (look at older content); down-drag
                            // scrolls down. Gated to the messages area —
                            // dragging in the input bar still selects.
                            let in_messages =
                                app.messages_rect.borrow().as_ref().is_some_and(|r| {
                                    mouse.column >= r.x
                                        && mouse.column < r.x + r.width
                                        && mouse.row >= r.y
                                        && mouse.row < r.y + r.height
                                });
                            if in_messages {
                                if let Some(anchor) = app.drag_anchor_y {
                                    let delta = mouse.row as i32 - anchor as i32;
                                    if delta > 0 {
                                        app.scroll_up(delta as usize);
                                    } else if delta < 0 {
                                        app.scroll_down((-delta) as usize);
                                    }
                                }
                                app.drag_anchor_y = Some(mouse.row);
                            }
                        }
                        MouseEventKind::Up(_) => {
                            app.drag_anchor_y = None;
                        }
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
                            // Toast click → dismiss. Toasts render newest-
                            // first; row 0 corresponds to the last entry
                            // in `app.toasts`, row 1 to the second-to-last,
                            // etc. (See `iter().rev().take(h)` in
                            // `toast_overlay`.) Pop the matched toast.
                            let toast_hit = app
                                .toasts_rect
                                .borrow()
                                .as_ref()
                                .filter(|r| {
                                    mouse.column >= r.x
                                        && mouse.column < r.x + r.width
                                        && mouse.row >= r.y
                                        && mouse.row < r.y + r.height
                                })
                                .map(|r| {
                                    let local = mouse.row.saturating_sub(r.y) as usize;
                                    local
                                });
                            if let Some(local_row) = toast_hit {
                                if local_row < app.toasts.len() {
                                    let drop_idx = app.toasts.len() - 1 - local_row;
                                    app.toasts.remove(drop_idx);
                                }
                                continue;
                            }

                            // Sidebar session-row click: convert pixel
                            // coordinates back to a session index using
                            // the same row math the renderer uses.
                            let sidebar_hit = app
                                .sidebar_rect
                                .borrow()
                                .as_ref()
                                .filter(|r| {
                                    mouse.column >= r.x
                                        && mouse.column < r.x + r.width
                                        && mouse.row >= r.y
                                        && mouse.row < r.y + r.height
                                })
                                .copied();
                            let mut handled_in_sidebar = false;
                            if let Some(rect) = sidebar_hit {
                                // Inside borders: subtract 1 row top/bottom.
                                let local_row = mouse.row.saturating_sub(rect.y + 1);
                                // Skip the empty/no-sessions placeholder row.
                                if !app.session_meta.is_empty() {
                                    let cwd = app.cwd.clone();
                                    let (this_project, other) = crate::session::group_by_cwd(
                                        app.session_meta.clone(),
                                        Some(cwd.as_str()),
                                    );
                                    // Walk rows: header rows are 1 each; rest are sessions.
                                    let mut row = 0u16;
                                    let mut session_idx: Option<usize> = None;
                                    if !this_project.is_empty() {
                                        row += 1; // "── This project ──" header
                                        for (i, _) in this_project.iter().enumerate() {
                                            if row == local_row {
                                                session_idx = Some(i);
                                            }
                                            row += 1;
                                        }
                                    }
                                    if !other.is_empty() {
                                        row += 1; // "── Other projects ──" header
                                        for (i, _) in other.iter().enumerate() {
                                            if row == local_row {
                                                session_idx = Some(this_project.len() + i);
                                            }
                                            row += 1;
                                        }
                                    }
                                    if let Some(idx) = session_idx {
                                        let ordered: Vec<String> = this_project
                                            .into_iter()
                                            .chain(other.into_iter())
                                            .map(|s| s.id)
                                            .collect();
                                        if let Some(id) = ordered.get(idx).cloned() {
                                            if let Some(messages) =
                                                crate::session::load_session(&id).await
                                            {
                                                app.messages = messages;
                                                app.switch_session(Some(id));
                                                app.streaming_text.clear();
                                                app.streaming_reasoning.clear();
                                                app.streaming_response_bytes = 0;
                                                app.streaming_assistant_idx = None;
                                                app.session_selected = idx;
                                                app.session_list_state.select(Some(idx));
                                                app.scroll_to_bottom();
                                                handled_in_sidebar = true;
                                            }
                                        }
                                    }
                                }
                            }
                            if handled_in_sidebar {
                                // Sidebar consumed the click; skip the
                                // tool/yank fallthrough.
                            } else if let Some(group_key) = hit
                                .as_ref()
                                .and_then(|s| s.strip_prefix("group:"))
                                .map(str::to_owned)
                            {
                                // Click on a collapsed tool-group header.
                                // Toggle expansion — flips the next render
                                // between the single-line "▶ N reads"
                                // teaser and individual tool blocks.
                                if !app.tool_group_expanded.remove(&group_key) {
                                    app.tool_group_expanded.insert(group_key);
                                }
                            } else if let Some(tool_id) = hit {
                                const DOUBLE_CLICK_MS: u128 = 350;
                                let now = std::time::Instant::now();
                                let is_double_click = match &app.last_tool_click {
                                    Some((prev_id, prev_at))
                                        if prev_id == &tool_id
                                            && now.duration_since(*prev_at).as_millis()
                                                < DOUBLE_CLICK_MS =>
                                    {
                                        true
                                    }
                                    _ => false,
                                };
                                for msg in &mut app.messages {
                                    for part in &mut msg.parts {
                                        if let MessagePart::Tool(tc) = part {
                                            if tc.id == tool_id {
                                                if is_double_click {
                                                    // Toggle pin. Pinning
                                                    // forces expanded; unpinning
                                                    // leaves expanded as-is so
                                                    // the user can collapse with
                                                    // a subsequent single click.
                                                    tc.pinned = !tc.pinned;
                                                    if tc.pinned {
                                                        tc.expanded = true;
                                                    }
                                                } else {
                                                    tc.expanded = !tc.expanded;
                                                }
                                            }
                                        }
                                    }
                                }
                                app.last_tool_click = Some((tool_id, now));
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
                AppEvent::TeammateEvent(teammate_ev) => {
                    use crate::swarm::runner::TeammateEvent;
                    match teammate_ev {
                        TeammateEvent::Idle {
                            task_id,
                            agent_id: _,
                            agent_name,
                            reason,
                            summary,
                        } => {
                            tracing::info!(
                                "[Swarm] Teammate {agent_name} went idle (reason: {reason:?})"
                            );
                            // Mark the BackgroundTask Idle so the task
                            // panel stops showing "Receiving output…" forever
                            // and the subagent tree can render the agent
                            // dimmer. Without this transition the panel
                            // pinned to the bottom looking alive even
                            // though the teammate had already sent its
                            // message and stopped producing chunks.
                            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                                if matches!(bt.status, crate::types::TaskLifecycle::Running) {
                                    bt.status = crate::types::TaskLifecycle::Idle;
                                }
                                bt.last_tool = None;
                            }
                            // Surface to the user as a toast — without this
                            // the user has no way to tell that a teammate
                            // finished its turn and is waiting. Summary
                            // (when present) is the model's own one-line
                            // recap, which reads better than the raw reason.
                            let msg = match (summary.as_deref(), reason.as_deref()) {
                                (Some(s), _) if !s.is_empty() => {
                                    format!("⏸ {agent_name} idle — {s}")
                                }
                                (_, Some(r)) if !r.is_empty() => {
                                    format!("⏸ {agent_name} idle ({r})")
                                }
                                _ => format!("⏸ {agent_name} is idle"),
                            };
                            toast::push_with_cap(
                                &mut app.toasts,
                                toast::Toast::new(toast::ToastKind::Info, msg),
                            );
                        }
                        TeammateEvent::Progress {
                            task_id,
                            agent_id: _,
                            token_count,
                            tool_use_count,
                            last_tool,
                        } => {
                            // Update background task state for UI display.
                            // Revive an Idle task back to Running — the agent
                            // is producing tool-progress events again, so it
                            // is no longer idle.
                            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                                if matches!(bt.status, crate::types::TaskLifecycle::Idle) {
                                    bt.status = crate::types::TaskLifecycle::Running;
                                }
                                bt.last_tool = last_tool;
                                // The teammate event already gives us a
                                // single combined token figure; route it
                                // into `latest_input_tokens` so the fan UI
                                // shows it without overwriting the
                                // per-turn output sum. (Teammates don't
                                // emit input/output separately yet.)
                                bt.latest_input_tokens = token_count;
                                bt.tool_use_count = tool_use_count as u32;

                                // v132 per-agent token budget enforcement.
                                // When the agent's total tokens exceed
                                // its configured ceiling, set its status
                                // to Failed and surface a kill toast
                                // exactly once. We don't actually SIGKILL
                                // the in-flight tokio task here — that
                                // requires the swarm interrupt path —
                                // but the fan UI / approval flow uses
                                // `bt.status` to decide whether to keep
                                // accepting work, so flipping it stops
                                // the bleed.
                                if let (Some(cap), false) =
                                    (bt.max_input_tokens, bt.budget_killed)
                                {
                                    let total = bt.latest_input_tokens
                                        + bt.cumulative_output_tokens;
                                    if total > cap {
                                        bt.budget_killed = true;
                                        bt.status =
                                            crate::types::TaskLifecycle::Failed;
                                        bt.error = Some(format!(
                                            "killed: token budget {cap} exceeded ({total} used)"
                                        ));
                                        let agent = bt
                                            .description
                                            .lines()
                                            .next()
                                            .unwrap_or(&bt.task_id)
                                            .to_owned();
                                        let total_for_msg = total;
                                        toast::push_with_cap(
                                            &mut app.toasts,
                                            toast::Toast::new(
                                                toast::ToastKind::Error,
                                                format!(
                                                    "Agent {agent} killed: budget {cap} exceeded ({total_for_msg} tokens)"
                                                ),
                                            ),
                                        );
                                    }
                                }
                            }
                            // Mark this teammate as the live one for the
                            // spinner-area tree highlight.
                            app.last_active_agent_task = Some(task_id);
                        }
                        TeammateEvent::TextDelta {
                            task_id,
                            agent_id: _,
                            delta,
                        } => {
                            // A new text delta means the teammate is producing
                            // output again — revive Idle → Running so the
                            // task panel resumes its "Receiving output…" spinner.
                            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                                if matches!(bt.status, crate::types::TaskLifecycle::Idle) {
                                    bt.status = crate::types::TaskLifecycle::Running;
                                }
                            }
                            // Translate to AgentChunk so the existing
                            // chunk handler (with coalescing rules and
                            // BackgroundTask.messages append) handles it
                            // — same path as one-shot subagents.
                            let _ = tx
                                .send(AppEvent::AgentChunk {
                                    task_id,
                                    text: delta,
                                })
                                .await;
                        }
                        TeammateEvent::Completed { task_id, agent_id } => {
                            tracing::info!("[Swarm] Teammate {agent_id} completed");
                            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                                bt.status = crate::types::TaskLifecycle::Completed;
                            }
                            // Mark the member inactive on the team file so a
                            // later `set_member_active(true)` (e.g. an agent
                            // that gets re-spawned) can observe the prior
                            // state and the roster reflects who's currently
                            // running.
                            if let Some(team_name) = app.team_context.team_name.clone() {
                                // agent_id is "name@team" — `set_member_active`
                                // matches on the bare name field.
                                let member_name = agent_id
                                    .split_once('@')
                                    .map(|(n, _)| n.to_owned())
                                    .unwrap_or_else(|| agent_id.clone());
                                tokio::spawn(async move {
                                    let _ = crate::swarm::team_helpers::set_member_active(
                                        &team_name,
                                        &member_name,
                                        false,
                                    )
                                    .await;
                                });
                            }
                        }
                        TeammateEvent::Failed {
                            task_id,
                            agent_id,
                            error,
                        } => {
                            tracing::warn!("[Swarm] Teammate {agent_id} failed: {error}");
                            if let Some(bt) = app.background_tasks.get_mut(&task_id) {
                                bt.status = crate::types::TaskLifecycle::Completed;
                                bt.error = Some(error);
                            }
                            if let Some(team_name) = app.team_context.team_name.clone() {
                                let member_name = agent_id
                                    .split_once('@')
                                    .map(|(n, _)| n.to_owned())
                                    .unwrap_or_else(|| agent_id.clone());
                                tokio::spawn(async move {
                                    let _ = crate::swarm::team_helpers::set_member_active(
                                        &team_name,
                                        &member_name,
                                        false,
                                    )
                                    .await;
                                });
                            }
                        }
                        TeammateEvent::MessageSent {
                            from,
                            to,
                            text,
                            summary,
                        } => {
                            tracing::info!("[Swarm] Message from {from} → {to}");
                            // Route the outbound message to the recipient's
                            // mailbox so its polling loop picks it up. Mirrors
                            // v126's `sendMessageToTeammate` (cli.js around
                            // 396870) — the producing teammate writes; the
                            // recipient consumes via `read_mailbox`. Without
                            // this, the SendMessage tool was a no-op past
                            // logging.
                            let team_name = app.team_context.team_name.clone().unwrap_or_default();
                            if team_name.is_empty() {
                                tracing::warn!(
                                    "[Swarm] MessageSent dropped — no active team_context"
                                );
                            } else {
                                let recipient = to.clone();
                                let msg = crate::swarm::types::MailboxMessage {
                                    from: from.clone(),
                                    text: text.clone(),
                                    timestamp: chrono::Utc::now().to_rfc3339(),
                                    color: None,
                                    summary: summary.clone(),
                                    read: false,
                                };
                                tokio::spawn(async move {
                                    if let Err(e) = crate::swarm::mailbox::write_to_mailbox(
                                        &recipient, msg, &team_name,
                                    )
                                    .await
                                    {
                                        tracing::warn!(
                                            "[Swarm] Failed to deliver message {from} → {to}: {e}"
                                        );
                                    }
                                });
                            }
                        }
                    }
                }
                AppEvent::Tick => {
                    app.spinner_frame = (app.spinner_frame + 1) % crate::app::SPINNER.len();
                    // Auto-clear expired toasts every tick. Cheap (O(N) over
                    // a tiny vec capped at MAX_TOASTS) and the only reliable
                    // place to do it — toasts have no creation-time timer.
                    toast::prune_expired(&mut app.toasts, std::time::Instant::now());

                    // v132 OnHeartbeat — fire every ~30s so registered
                    // handlers (telemetry batchers, MCP keep-alive, daemon
                    // wakeup probes) actually run. Async fire because we
                    // don't care about the result — short-circuit logic
                    // would block the UI thread.
                    let now = std::time::Instant::now();
                    let heartbeat_due = app
                        .last_heartbeat_at
                        .map(|t| now.duration_since(t).as_secs() >= 30)
                        .unwrap_or(true);
                    if heartbeat_due {
                        app.last_heartbeat_at = Some(now);
                        let session_id = app
                            .current_session_id
                            .clone()
                            .unwrap_or_else(|| "<no-session>".to_owned());
                        crate::hooks::fire_async(
                            crate::hooks::HookPoint::OnHeartbeat,
                            &crate::hooks::HookContext::for_session(&session_id),
                        );
                    }

                    // v132 MCP `notifications/tools/list_changed` —
                    // detect inbound notifications by comparing the
                    // process-global refresh counter against our last-
                    // seen value. On change, emit a toast + system-
                    // reminder so the user knows the tool catalog
                    // mutated and the model picks up the change next
                    // turn.
                    let cur_refresh = crate::mcp::registry::refresh_counter();
                    if cur_refresh > app.last_mcp_refresh_seen {
                        app.last_mcp_refresh_seen = cur_refresh;
                        toast::push_with_cap(
                            &mut app.toasts,
                            toast::Toast::new(
                                toast::ToastKind::Info,
                                "MCP server pushed tools/list_changed — catalog refreshed",
                            ),
                        );
                        crate::system_reminder::append_to_last_user(
                            &mut app.messages,
                            "An MCP server announced `tools/list_changed`. The tool \
                             catalog may have changed; if you were about to call a \
                             specific MCP tool, re-check it exists.",
                        );
                    }

                    // Refresh the worktree count at most once per second
                    // so the status-bar `⌥ N wt` badge reflects /worktree
                    // create|remove and agent-isolation churn without
                    // shelling to `git worktree list` on every render.
                    let now = std::time::Instant::now();
                    let due = app
                        .worktree_count_last_refresh
                        .map(|t| now.duration_since(t).as_millis() >= 1_000)
                        .unwrap_or(true);
                    if due {
                        let cwd = std::env::current_dir().unwrap_or_default();
                        app.worktree_count = match crate::worktrees::list_worktrees_async(&cwd).await {
                            // Entry 0 is the primary checkout — subtract it
                            // so the badge counts agent-isolated trees only.
                            Ok(list) => list.len().saturating_sub(1),
                            Err(_) => 0,
                        };
                        app.worktree_count_last_refresh = Some(now);
                    }

                    // Git branch refresh — every 5s. Reads `.git/HEAD`
                    // directly (no shell-out): faster, doesn't spawn a
                    // subprocess, and "ref: refs/heads/<branch>" is the
                    // dominant form in normal workflows. Detached HEAD
                    // (HEAD = sha) gets reported as "(detached)".
                    let git_due = app
                        .git_branch_last_refresh
                        .map(|t| now.duration_since(t).as_millis() >= 5_000)
                        .unwrap_or(true);
                    if git_due {
                        let cwd = std::env::current_dir().unwrap_or_default();
                        app.git_branch = read_git_branch(&cwd).await;
                        app.git_branch_last_refresh = Some(now);
                    }

                    // Resolve any pending teammate permission requests at
                    // ~1Hz (12 ticks × 80ms). The teammate runner blocks
                    // on `poll_for_response` after writing a request; if
                    // nothing ever resolves, the call times out at 5
                    // minutes and the tool fails. This loop provides the
                    // leader-side response: apply the leader's own
                    // permission_mode to the request and write a resolution
                    // file the teammate's poll picks up.
                    if app.team_context.is_active() && app.spinner_frame % 12 == 0 {
                        if let Some(team_name) = app.team_context.team_name.clone() {
                            let mode = app.permission_mode;
                            let tx_swarm = tx.clone();
                            tokio::spawn(async move {
                                let pending =
                                    crate::swarm::permission_sync::read_pending_permissions(
                                        &team_name,
                                    )
                                    .await;
                                for req in pending {
                                    if !matches!(
                                        req.status,
                                        crate::swarm::types::PermissionRequestStatus::Pending
                                    ) {
                                        continue;
                                    }
                                    let mutation = matches!(
                                        req.tool_name.as_str(),
                                        "Bash" | "Write" | "Edit" | "ApplyPatch"
                                    );
                                    // Three outcomes:
                                    //   Some(true)  → auto-approve
                                    //   Some(false) → auto-deny
                                    //   None        → defer to the user
                                    let auto: Option<bool> = match mode {
                                        crate::app::PermissionMode::BypassPermissions => Some(true),
                                        crate::app::PermissionMode::Plan => Some(false),
                                        crate::app::PermissionMode::AcceptEdits => {
                                            if matches!(req.tool_name.as_str(), "Bash") {
                                                None
                                            } else {
                                                Some(true)
                                            }
                                        }
                                        crate::app::PermissionMode::Default
                                        | crate::app::PermissionMode::Auto => {
                                            if mutation {
                                                // Mutations need a human in
                                                // Default/Auto. Surface to
                                                // the user via toast +
                                                // /swarm-approve|deny.
                                                None
                                            } else {
                                                Some(true)
                                            }
                                        }
                                    };
                                    match auto {
                                        Some(approve) => {
                                            let resolution =
                                                crate::swarm::types::PermissionResolution {
                                                    decision: if approve {
                                                        crate::swarm::types::PermissionDecision::Approved
                                                    } else {
                                                        crate::swarm::types::PermissionDecision::Rejected
                                                    },
                                                    resolved_by: "leader".to_owned(),
                                                    feedback: if approve {
                                                        None
                                                    } else {
                                                        Some(format!(
                                                            "Auto-denied by leader permission_mode={:?}",
                                                            mode
                                                        ))
                                                    },
                                                    updated_input: None,
                                                    permission_updates: Vec::new(),
                                                };
                                            if let Err(e) =
                                                crate::swarm::permission_sync::resolve_permission(
                                                    &req.id,
                                                    &resolution,
                                                    &team_name,
                                                )
                                                .await
                                            {
                                                tracing::warn!(
                                                    target: "jfc::swarm",
                                                    error = %e,
                                                    request_id = %req.id,
                                                    "failed to resolve permission request"
                                                );
                                            }
                                        }
                                        None => {
                                            // User-gate path: surface a
                                            // toast (once per request id).
                                            // The toast tells the user
                                            // exactly which slash command
                                            // resolves it.
                                            let toast_text = format!(
                                                "🔒 {} wants to {} — /swarm-approve {} or /swarm-deny {}",
                                                req.worker_name, req.tool_name, req.id, req.id,
                                            );
                                            let _ = tx_swarm
                                                .send(AppEvent::Toast {
                                                    kind: crate::toast::ToastKind::Warning,
                                                    text: toast_text,
                                                })
                                                .await;
                                        }
                                    }
                                }
                            });
                        }
                    }

                    // Poll leader inbox for teammate messages every ~1s (12 ticks * 80ms).
                    // Only active when a team is running.
                    if app.team_context.is_active() && app.spinner_frame % 12 == 0 {
                        if let Some(ref team_name) = app.team_context.team_name {
                            let team_name = team_name.clone();
                            let tx_inbox = tx.clone();
                            tokio::spawn(async move {
                                let messages =
                                    crate::swarm::runner::poll_leader_inbox(&team_name).await;
                                for msg in messages {
                                    // Hand off to the main thread which has
                                    // mutable access to `app.messages` —
                                    // injects into the transcript AND shows
                                    // a toast in one place. Mirrors v126's
                                    // `<teammate-message>` injection.
                                    let _ = tx_inbox
                                        .send(AppEvent::TeammateInbox {
                                            from: msg.from,
                                            text: msg.text,
                                            summary: msg.summary,
                                        })
                                        .await;
                                }
                            });
                        }
                    }
                }
                AppEvent::StreamChunk { text, reasoning } => {
                    // Reset the stall clock on every chunk so the spinner's
                    // sub-status (`warming up` / `thinking` / `almost done`)
                    // reflects time-since-last-byte, not time-since-stream-start.
                    let now = std::time::Instant::now();
                    app.streaming_last_token_at = Some(now);
                    // Stamp for the right-edge token-rain animation. The
                    // renderer reads this each frame and lights one cell
                    // in the rain column with intensity proportional to
                    // recency (full at 0ms, dark at 800ms+).
                    app.last_token_arrival = Some(now);
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
                        // v132 idle prefetch: scan the chunk for path
                        // references and stash them so subsequent Read
                        // tool calls hit the prefetch cache instead of
                        // doing fresh I/O. Async-spawned reads happen
                        // out-of-band so the UI thread never blocks on
                        // disk. Bounded to 8 prefetches per chunk so
                        // the model spitting out a directory listing
                        // doesn't fan out 200 background reads.
                        let prefetch_targets = crate::idle_prefetch::extract_candidates(&chunk);
                        for path in prefetch_targets.into_iter().take(8) {
                            if crate::idle_prefetch::get(&path, None, None).is_some() {
                                continue;
                            }
                            tokio::spawn(async move {
                                if let Ok(body) = tokio::fs::read_to_string(&path).await {
                                    crate::idle_prefetch::put(&path, None, None, body);
                                }
                            });
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
                            let decision = crate::auto_mode::classify(
                                provider.as_ref(),
                                &model,
                                &cfg,
                                &history,
                                &tool_for_task,
                            )
                            .await;
                            let _ = tx_cls
                                .send(AppEvent::ClassifierDecision {
                                    tool: tool_for_task,
                                    blocked: decision.should_block(),
                                    reason: decision.reason,
                                })
                                .await;
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

                    // OpenWebUI / LiteLLM / some third-party gateways
                    // leak `<tool_call>` XML into the assistant text
                    // instead of using OpenAI's `tool_calls` array.
                    // Detect the leaked markup and surface a toast so
                    // the user knows their gateway is misconfigured —
                    // jfc's renderer can't currently dispatch from
                    // inline markup. Mirrors the pattern v132 uses
                    // for `tengu_streaming_*` warnings.
                    if let Some(last) = app.messages.last() {
                        let text: String = last
                            .parts
                            .iter()
                            .filter_map(|p| {
                                if let crate::types::MessagePart::Text(t) = p {
                                    Some(t.as_str())
                                } else {
                                    None
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        if crate::inline_tools::contains_inline_tools(&text) {
                            let segments = crate::inline_tools::parse(&text);
                            let tool_calls = segments
                                .iter()
                                .filter(|s| matches!(s, crate::inline_tools::Segment::ToolCall { .. }))
                                .count();
                            tracing::warn!(
                                target: "jfc::stream::inline_tools",
                                tool_calls,
                                "assistant text contains inline <tool_call> markup — \
                                 the upstream gateway is emitting tool calls as text, \
                                 not as the OpenAI `tool_calls` field. They won't \
                                 dispatch."
                            );
                            crate::toast::push_with_cap(
                                &mut app.toasts,
                                crate::toast::Toast::new(
                                    crate::toast::ToastKind::Warning,
                                    format!(
                                        "Detected {tool_calls} inline `<tool_call>` block(s) \
                                         in the response — your OpenWebUI/LiteLLM gateway is \
                                         emitting tool calls as text, not via OpenAI tool_calls. \
                                         Check the gateway config."
                                    ),
                                ),
                            );
                        }
                    }
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
                    let turn_done = stop_reason == crate::provider::StopReason::EndTurn
                        && app.pending_approval.is_none()
                        && app.approval_queue.is_empty()
                        && app.pending_tool_calls.is_empty();
                    if turn_done {
                        // v132 session auto-naming — fire on the first
                        // assistant-turn completion if no title is set
                        // yet. We dispatch a non-blocking tokio task so
                        // the UI doesn't stall waiting on the naming
                        // call. Best-effort: failures are logged but
                        // don't surface to the user (the fallback title
                        // is still readable).
                        let user_turn_count = app
                            .messages
                            .iter()
                            .filter(|m| matches!(m.role, types::Role::User))
                            .count();
                        if user_turn_count == 1 {
                            let first_user = app
                                .messages
                                .iter()
                                .find(|m| matches!(m.role, types::Role::User))
                                .and_then(|m| {
                                    m.parts.iter().find_map(|p| match p {
                                        types::MessagePart::Text(t) => Some(t.clone()),
                                        _ => None,
                                    })
                                });
                            let first_assistant = app
                                .messages
                                .iter()
                                .find(|m| matches!(m.role, types::Role::Assistant))
                                .and_then(|m| {
                                    m.parts.iter().find_map(|p| match p {
                                        types::MessagePart::Text(t) => Some(t.clone()),
                                        _ => None,
                                    })
                                });
                            if let (Some(sid), Some(u), Some(a)) = (
                                app.current_session_id.clone(),
                                first_user,
                                first_assistant,
                            ) {
                                if let Some((p, m)) = crate::tools::snapshot_active_provider() {
                                    tokio::spawn(async move {
                                        let _ = crate::session_naming::generate_and_save(
                                            sid, p, m, u, a,
                                        )
                                        .await;
                                    });
                                }
                            }
                        }
                        if let (Some(start), Some(idx)) =
                            (app.turn_started_at, app.streaming_assistant_idx)
                        {
                            let elapsed = std::time::Instant::now().duration_since(start);
                            let label = crate::spinner::format_finished(elapsed);
                            // v132 per-turn cost surfacing: append the
                            // turn's incremental cost to the elapsed footer
                            // so the user sees "Cooked for 2m / $0.04". We
                            // approximate per-turn cost from the most-
                            // recent message_delta usage (already populated
                            // into usage_by_model). Skipped when no model
                            // is registered (no pricing match).
                            let turn_cost =
                                crate::cost::total_cost(&app.usage_by_model);
                            let label = if turn_cost > 0.0 {
                                format!(
                                    "{label} / {}",
                                    crate::cost::fmt_cost(turn_cost)
                                )
                            } else {
                                label
                            };
                            // Pull the assistant's text body for the
                            // notification preview before re-borrowing
                            // mutably to stamp the elapsed footer.
                            let preview = app
                                .messages
                                .get(idx)
                                .and_then(|m| {
                                    m.parts.iter().find_map(|p| match p {
                                        types::MessagePart::Text(s) if !s.is_empty() => {
                                            Some(s.clone())
                                        }
                                        _ => None,
                                    })
                                })
                                .unwrap_or_default();
                            if let Some(msg) = app.messages.get_mut(idx) {
                                msg.elapsed = Some(label);
                            }
                            crate::notifications::notify_turn_complete(elapsed, &preview);
                        }
                        // Push this turn's total token count onto the
                        // sparkline history. `last_usage_input` reflects
                        // the API's wire-truth count (cumulative across
                        // the turn) and `last_usage_output` is the model's
                        // generated count. Together they give a per-turn
                        // sense of "how much work did this take."
                        let turn_total = (app.last_usage_input as u64)
                            .saturating_add(app.last_usage_output as u64);
                        if turn_total > 0 {
                            if app.token_history.len() >= app::TOKEN_HISTORY_CAP {
                                app.token_history.pop_front();
                            }
                            app.token_history.push_back(turn_total);
                        }
                    }
                    app.streaming_started_at = None;
                    app.streaming_last_token_at = None;

                    // v132 cost-budget surfacing. When the user has set a
                    // session budget and we cross 80% / 100%, post a toast
                    // once per threshold so they can choose to stop or
                    // switch to a cheaper model. We never hard-block (an
                    // in-flight investigation shouldn't be killed mid-turn
                    // by an estimate); the toast is the user's signal.
                    if let Some(budget_usd) = config::load().session_cost_budget_usd {
                        if budget_usd > 0.0 {
                            let spent = crate::cost::total_cost(&app.usage_by_model);
                            let pct = ((spent / budget_usd) * 100.0).round() as u8;
                            let cross = |th: u8| pct >= th && app.cost_budget_warned_at < th;
                            if cross(100) {
                                app.cost_budget_warned_at = 100;
                                crate::toast::push_with_cap(
                                    &mut app.toasts,
                                    crate::toast::Toast::new(
                                        crate::toast::ToastKind::Error,
                                        format!(
                                            "Session cost {} exceeds budget {} — consider /quit or switching models",
                                            crate::cost::fmt_cost(spent),
                                            crate::cost::fmt_cost(budget_usd),
                                        ),
                                    ),
                                );
                            } else if cross(80) {
                                app.cost_budget_warned_at = 80;
                                crate::toast::push_with_cap(
                                    &mut app.toasts,
                                    crate::toast::Toast::new(
                                        crate::toast::ToastKind::Warning,
                                        format!(
                                            "Session cost {} at {pct}% of {} budget",
                                            crate::cost::fmt_cost(spent),
                                            crate::cost::fmt_cost(budget_usd),
                                        ),
                                    ),
                                );
                            }
                        }
                    }

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
                    if stop_reason == crate::provider::StopReason::EndTurn
                        && app.pending_approval.is_none()
                        && app.approval_queue.is_empty()
                        && app.pending_tool_calls.is_empty()
                    {
                        app.turn_started_at = None;
                    }

                    // Auto-save session after each assistant turn completes
                    if let Some(ref session_id) = app.current_session_id {
                        let sid = session_id.clone();
                        let msgs = app.messages.clone();
                        let cwd = app.cwd.clone();
                        let model = app.model.clone();
                        tokio::spawn(async move {
                            session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
                        });
                        app.last_session_save_at = Some(std::time::Instant::now());
                    }
                    // v126 queued-prompt drain on plain end_turn: model finished
                    // without tools to call → if anything's queued, fire it now.
                    if stop_reason == crate::provider::StopReason::EndTurn
                        && app.pending_approval.is_none()
                        && app.approval_queue.is_empty()
                        && app.pending_tool_calls.is_empty()
                        && !app.queued_prompts.is_empty()
                    {
                        drain_queued_prompts(&mut app, &tx).await;
                    }
                    if stop_reason == crate::provider::StopReason::ToolUse {
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
                                app.teammate_event_tx.clone(),
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
                    // v132 mid-stream auto-compact: stream.rs prefixes
                    // its `auto-compact:` sentinel when the API rejected
                    // the prompt for size reasons. We force a compact
                    // and re-queue the last user prompt instead of
                    // surfacing the failure to the user — they shouldn't
                    // have to manually trigger /compact + retype every
                    // time the estimator drifts.
                    let auto_compact_signal = e.starts_with("auto-compact:");
                    if auto_compact_signal {
                        app.force_compact_pending = true;
                        toast::push_with_cap(
                            &mut app.toasts,
                            toast::Toast::new(
                                toast::ToastKind::Warning,
                                "Auto-compacting (prompt exceeded model window)…",
                            ),
                        );
                        // Try to recover the last user prompt so we can
                        // re-queue it after compaction.
                        let last_user_text = app
                            .messages
                            .iter()
                            .rfind(|m| matches!(m.role, types::Role::User))
                            .and_then(|m| {
                                m.parts.iter().find_map(|p| match p {
                                    types::MessagePart::Text(t) if !t.trim().is_empty() => {
                                        Some(t.clone())
                                    }
                                    _ => None,
                                })
                            });
                        if let Some(text) = last_user_text {
                            let tx_compact = tx.clone();
                            tokio::spawn(async move {
                                tokio::time::sleep(std::time::Duration::from_millis(150))
                                    .await;
                                let _ = tx_compact
                                    .send(AppEvent::Submit(text))
                                    .await;
                            });
                        }
                    }
                    app.is_streaming = false;
                    app.streaming_started_at = None;
                    app.streaming_last_token_at = None;
                    app.thinking_started_at = None;
                    app.thinking_ended_at = None;
                    app.streaming_text.clear();
                    app.streaming_reasoning.clear();
                    app.streaming_response_bytes = 0;
                    app.streaming_assistant_idx = None;
                    // Clear the turn clock and any pending tool calls so the
                    // spinner row stops rendering. Without this, the
                    // `show_spinner` condition stays true (it checks
                    // `turn_started_at.is_some()` and `!pending_tool_calls.is_empty()`)
                    // and the spinner/counter keeps animating after an
                    // interrupt or network error.
                    app.turn_started_at = None;
                    app.pending_tool_calls.clear();
                    // Reset the interrupt flag so background tasks or the
                    // next auto-retry don't see a stale `true`.
                    app.interrupt_flag
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    if !auto_compact_signal {
                        app.messages.push(ChatMessage::assistant(format!(
                            "**Error:** {e}\n\n_Press Ctrl+R to retry the last prompt._"
                        )));
                        // Surface as a toast too so the user sees the failure
                        // even if they aren't looking at the bottom of the
                        // transcript when it lands. Cap to 120 chars so a
                        // multi-paragraph error stays readable in the strip.
                        let mut preview_cap = e.len().min(120);
                        while preview_cap > 0 && !e.is_char_boundary(preview_cap) {
                            preview_cap -= 1;
                        }
                        let preview = &e[..preview_cap];
                        toast::push_with_cap(
                            &mut app.toasts,
                            toast::Toast::new(
                                toast::ToastKind::Error,
                                format!("Stream error: {preview}"),
                            ),
                        );
                    }
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
                    // v126's tokenCountWithEstimation uses input + cache_creation +
                    // cache_read + output (all four count against the context window).
                    // Previously this only summed input + output, under-reporting by
                    // the cache contribution — which can be 50-80% of context on
                    // prompt-cache-heavy sessions.
                    app.tool_ctx.approx_tokens = input_tokens as usize
                        + output_tokens as usize
                        + cache_read_tokens as usize
                        + cache_write_tokens as usize;
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
                    // Mirror the snapshot into the global so `stream_response`
                    // can inject diagnostics into the system prompt without
                    // having to touch every call site to thread through an
                    // `&[DiagnosticEntry]` parameter.
                    crate::diagnostics::set_global_snapshot(entries.clone());
                    app.diagnostics = entries;
                    // Toast-on-transition was disabled by user request — the
                    // dim summary row above the spinner already surfaces the
                    // count, and Ctrl+O opens the full panel. Spawning a
                    // separate toast on launch (when cargo-check produced
                    // its initial set) doubled the noise. The transition
                    // toast is intentionally left commented out rather than
                    // deleted so it can be reinstated behind a setting if
                    // wanted later.
                    // let was_empty = app.diagnostics.is_empty();
                    // let is_empty = entries.is_empty();
                    // ...
                }
                AppEvent::ToolOutputChunk { tool_id, chunk } => {
                    // Append streaming output to the tool's live preview.
                    // This fires line-by-line for bash commands, giving
                    // real-time visibility into long-running processes.
                    for msg in &mut app.messages {
                        for part in &mut msg.parts {
                            if let MessagePart::Tool(tc) = part {
                                if tc.id == tool_id {
                                    // Append to existing output or create new
                                    match &mut tc.output {
                                        ToolOutput::Text(s) => {
                                            s.push_str(&chunk);
                                            s.push('\n');
                                        }
                                        _ => {
                                            tc.output = ToolOutput::Text(format!("{chunk}\n"));
                                        }
                                    }
                                    break;
                                }
                            }
                        }
                    }

                    // v132 Marsh (mid-stream bash output to model):
                    // accumulate the chunk into a pending buffer that
                    // stream.rs prepends as a `<system-reminder>` on
                    // the *next* outbound request. Not strictly mid-
                    // stream (the API call is already in flight) but
                    // ensures the model sees what bash printed by the
                    // time it next gets the wheel — close enough for
                    // the "I see the error, stop" feedback loop in
                    // agentic loops where each tool reply re-enters
                    // the model.
                    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Marsh) {
                        let _ = tool_id;
                        crate::feature_gates::marsh_push(chunk);
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
                                    // Stamp wall-clock duration as soon as
                                    // the result lands. The renderer reads
                                    // `tc.elapsed_ms` to draw a muted
                                    // "[2.3s]" badge after the title. Falls
                                    // back to None if `started_at` was lost
                                    // (e.g., resumed session) — the badge
                                    // just doesn't appear in that case.
                                    if let Some(start) = tc.started_at {
                                        tc.elapsed_ms = Some(start.elapsed().as_millis() as u64);
                                    }
                                    // Tool authors can attach a structured
                                    // DiffView (Edit, Write-overwrite) so
                                    // the renderer shows colorized hunks
                                    // instead of a flat success string.
                                    tc.output = if let Some(diff) = result.diff.clone() {
                                        ToolOutput::Diff(diff)
                                    } else if LargeText::should_collapse(&result.output) {
                                        ToolOutput::LargeText(LargeText::new(result.output.clone()))
                                    } else {
                                        ToolOutput::Text(result.output.clone())
                                    };
                                    if matches!(tc.output, ToolOutput::LargeText(_)) {
                                        tc.is_collapsed = true;
                                    }
                                    // Fresh tool output → reset the
                                    // path-yank cursor so the next
                                    // `Ctrl+L` starts from the newest
                                    // file:line ref.
                                    app.path_yank_cursor = 0;
                                    if result.is_error() {
                                        crate::notifications::notify_tool_failed(
                                            tc.kind.label(),
                                            &result.output,
                                        );
                                    }
                                    let new_status = if result.is_error() {
                                        ToolStatus::Failed
                                    } else {
                                        ToolStatus::Complete
                                    };
                                    tc.status = new_status;
                                    // Sparkle on success: stamp the tool
                                    // id so the renderer can flash a `✦`
                                    // for ~600ms next to its gutter, then
                                    // fade. Failures intentionally don't
                                    // sparkle — celebration on red would
                                    // be confusing.
                                    if matches!(new_status, ToolStatus::Complete) {
                                        app.recent_tool_completion =
                                            Some((tc.id.clone(), std::time::Instant::now()));
                                    }
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
                        let sid = session_id.clone();
                        let msgs = app.messages.clone();
                        let cwd = app.cwd.clone();
                        let model = app.model.clone();
                        tokio::spawn(async move {
                            session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
                        });
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
                    // Guard: don't spawn another compact if one is already in flight.
                    // Without this, every AllToolsComplete while context > threshold
                    // spawns a NEW compact task — if the provider doesn't support
                    // compaction (returns Unsupported), the tasks pile up at ~12/sec
                    // and spam 79K+ WARN lines per session. Only `manual` (/compact)
                    // bypasses the guard to let the user force a retry.
                    if app.compacting_started_at.is_some() && !manual {
                        tracing::debug!(
                            target: "jfc::compact",
                            "skipping post-response compact — one already in flight"
                        );
                    } else if app.compact_suppressed && !manual {
                        tracing::debug!(
                            target: "jfc::compact",
                            "skipping post-response compact — suppressed after permanent failure"
                        );
                    } else if manual
                        || crate::compact::should_compact(
                            app.tool_ctx.approx_tokens,
                            app.max_context_tokens,
                        )
                    {
                        if manual {
                            // /compact is the user's explicit override — clear
                            // BOTH the suppression flag AND the rapid-refill
                            // counter. Otherwise a previously tripped breaker
                            // would still fast-fail this manual attempt.
                            app.compact_suppressed = false;
                            app.tool_ctx.rapid_refill_count = 0;
                        }
                        tracing::info!(
                            target: "jfc::compact",
                            manual,
                            model = %app.model,
                            max_context_tokens = app.max_context_tokens,
                            message_count = app.messages.len(),
                            rapid_refill_count = app.tool_ctx.rapid_refill_count,
                            "post-response compaction triggered"
                        );
                        let _ = tx.send(AppEvent::CompactionStarted).await;
                        let messages = app.messages.clone();
                        let provider = Arc::clone(&app.provider);
                        let model = app.model.clone();
                        let mut tool_ctx = app.tool_ctx.clone();
                        let window = app.max_context_tokens;
                        let tx_compact = tx.clone();
                        let progress_tx = tx_compact.clone();
                        let on_progress: crate::compact::CompactProgressCb =
                            Box::new(move |chars| {
                                // CompactionProgress is non-critical; next progress update supersedes.
                                let _ = progress_tx.try_send(AppEvent::CompactionProgress {
                                    output_chars: chars,
                                });
                            });
                        tokio::spawn(async move {
                            let options = crate::provider::StreamOptions::new(model.clone());
                            tracing::debug!(
                                target: "jfc::compact",
                                model = %model,
                                window,
                                "spawned post-response compaction task"
                            );
                            let result = crate::compact::compact(
                                &messages,
                                provider.as_ref(),
                                &options,
                                &mut tool_ctx,
                                window,
                                Some(on_progress),
                            )
                            .await;
                            match result {
                                crate::compact::CompactResult::Success {
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
                                    let _ = tx_compact
                                        .send(AppEvent::CompactionDone {
                                            messages,
                                            tool_ctx,
                                            pre_tokens,
                                            post_tokens,
                                        })
                                        .await;
                                }
                                crate::compact::CompactResult::Unsupported => {
                                    tracing::info!(
                                        target: "jfc::compact",
                                        "post-response compaction skipped (provider unsupported)"
                                    );
                                    let _ = tx_compact
                                        .send(AppEvent::CompactionFailed(
                                            "Provider does not support compaction — \
                                     try /clear or switch to a provider with non-streaming support."
                                                .into(),
                                            None,
                                            false, // permanent: provider mismatch won't fix itself
                                        ))
                                        .await;
                                }
                                crate::compact::CompactResult::TooFewGroups => {
                                    tracing::info!(
                                        target: "jfc::compact",
                                        "post-response compaction skipped (single user turn)"
                                    );
                                    // Transient: the next user message creates a
                                    // second group, so auto-compaction can fire
                                    // again. Don't latch `compact_suppressed` —
                                    // otherwise a single huge agentic batch leaves
                                    // auto-compact dormant for the rest of the
                                    // session until the user remembers /compact.
                                    let _ = tx_compact.send(AppEvent::CompactionFailed(
                                    "Nothing to compact yet — only one conversation turn so far. \
                                     Auto-compact will retry after your next message."
                                        .into(),
                                    None,
                                    true, // transient: more user turns will unblock it
                                )).await;
                                }
                                crate::compact::CompactResult::CircuitBreakerTripped => {
                                    tracing::warn!(
                                        target: "jfc::compact",
                                        "post-response compaction: circuit breaker tripped"
                                    );
                                    let _ = tx_compact
                                        .send(AppEvent::CompactionFailed(
                                            "Circuit breaker tripped — compaction keeps refilling"
                                                .into(),
                                            None,
                                            false,
                                        ))
                                        .await;
                                }
                                crate::compact::CompactResult::Exhausted { attempts } => {
                                    tracing::warn!(
                                        target: "jfc::compact",
                                        attempts,
                                        "post-response compaction exhausted all attempts"
                                    );
                                    let _ = tx_compact
                                        .send(AppEvent::CompactionFailed(
                                            format!("Exhausted {attempts} compaction attempts"),
                                            None,
                                            false,
                                        ))
                                        .await;
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
                    if app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst) {
                        tracing::info!(
                            target: "jfc::stream",
                            "agentic loop NOT continuing — user requested interrupt"
                        );
                        // Clear so the next user submission starts fresh.
                        app.interrupt_flag
                            .store(false, std::sync::atomic::Ordering::SeqCst);
                        app.is_streaming = false;
                    } else if app.pending_approval.is_none()
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
                    app.compacting_output_chars = 0;
                    app.compacting_attempt_baseline = 0;
                    app.compacting_last_progress = 0;
                }
                AppEvent::CompactionProgress { output_chars } => {
                    // Live token feedback during compact streaming. Mirrors
                    // v126's PB7 addResponseLength → spinner refresh
                    // (cli.js:396989).
                    //
                    // `compact()` retries internally when post_tokens is
                    // still over the Blocked threshold or the model returns
                    // a truncated summary. Each retry streams a fresh
                    // response from 0 chars, so the per-attempt counter
                    // regresses. Detect that and bump a baseline so the
                    // spinner shows a monotonically-increasing total — the
                    // user sees the true work-done across attempts instead
                    // of a flickering counter that jumps `↓3k → ↓92 → ↓1k`.
                    if output_chars < app.compacting_last_progress {
                        app.compacting_attempt_baseline += app.compacting_last_progress;
                    }
                    app.compacting_last_progress = output_chars;
                    app.compacting_output_chars = app.compacting_attempt_baseline + output_chars;
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
                    app.compacting_output_chars = 0;
                    app.compacting_attempt_baseline = 0;
                    app.compacting_last_progress = 0;
                    app.compact_suppressed = false;
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
                AppEvent::CompactionFailed(reason, calibrated_tokens, transient) => {
                    tracing::warn!(
                        target: "jfc::compact",
                        %reason,
                        ?calibrated_tokens,
                        transient,
                        "compaction failed — surfacing toast to user"
                    );
                    if let Some(real_count) = calibrated_tokens {
                        app.tool_ctx.approx_tokens = real_count;
                    }
                    app.compacting_started_at = None;
                    app.compacting_output_chars = 0;
                    app.compacting_attempt_baseline = 0;
                    app.compacting_last_progress = 0;
                    // Permanent failures (provider unsupported, exhausted retries,
                    // breaker tripped) latch suppression so we stop spamming
                    // compact attempts on every AllToolsComplete; the user clears
                    // it explicitly with /compact. Transient failures (e.g.
                    // TooFewGroups) self-resolve as the conversation grows, so
                    // suppressing them would silently disable auto-compact for
                    // the rest of the session.
                    if !transient {
                        app.compact_suppressed = true;
                        crate::notifications::notify_compact_failed(&reason);
                    }
                    let toast_kind = if transient {
                        toast::ToastKind::Info
                    } else {
                        toast::ToastKind::Error
                    };
                    let toast_msg = if transient {
                        reason.clone()
                    } else {
                        format!("Compaction failed: {reason}")
                    };
                    toast::push_with_cap(&mut app.toasts, toast::Toast::new(toast_kind, toast_msg));
                }
                AppEvent::EnterPlanModeRequested { reason } => {
                    // Model-callable plan mode entry — the EnterPlanMode tool
                    // emits this. Flip the leader's permission mode and toast
                    // the reason so the user knows what triggered it.
                    app.permission_mode = crate::app::PermissionMode::Plan;
                    let preview: String = reason.chars().take(120).collect();
                    let body = if preview.is_empty() {
                        "Entered plan mode (model request)".to_owned()
                    } else {
                        format!("Plan mode: {preview}")
                    };
                    toast::push_with_cap(
                        &mut app.toasts,
                        toast::Toast::new(toast::ToastKind::Info, body),
                    );
                    // v132 mid-stream system-reminder so the next turn
                    // sees the mode flip explicitly. Without this the
                    // model only learns about the new permissions when
                    // a tool call gets denied — too late.
                    crate::system_reminder::append_to_last_user(
                        &mut app.messages,
                        "Permission mode is now `Plan` (read-only). Use ExitPlanMode \
                         with a finalized plan to proceed with edits.",
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
                AppEvent::TeammateInbox {
                    from,
                    text,
                    summary,
                } => {
                    // Append the teammate's message to the transcript as a
                    // user-role turn tagged with the teammate's name so it
                    // survives session save/load and the model sees it on
                    // its next request. v126 wraps these in a
                    // `<teammate-message from="…">…</teammate-message>` XML
                    // block; we use the same shape so the leader's system
                    // prompt rules for parsing teammate messages still
                    // apply.
                    let body = format!(
                        "<teammate-message from=\"{}\">\n{}\n</teammate-message>",
                        from, text
                    );
                    let mut msg = ChatMessage::user(body);
                    msg.agent_name = Some(from.clone());
                    app.messages.push(msg);
                    // Also surface a brief toast so the user notices the
                    // arrival without needing to scroll the transcript.
                    let preview = summary
                        .as_deref()
                        .filter(|s| !s.is_empty())
                        .map(|s| s.to_owned())
                        .unwrap_or_else(|| {
                            // Snap to a char boundary so multi-byte chars
                            // (emoji, accented) at byte 60 don't panic.
                            let mut cap = text.len().min(60);
                            while cap > 0 && !text.is_char_boundary(cap) {
                                cap -= 1;
                            }
                            text[..cap].to_owned()
                        });
                    toast::push_with_cap(
                        &mut app.toasts,
                        toast::Toast::new(toast::ToastKind::Info, format!("💬 {from}: {preview}")),
                    );
                    // Persist so a session reload doesn't lose the message.
                    if let Some(ref session_id) = app.current_session_id {
                        let sid = session_id.clone();
                        let msgs = app.messages.clone();
                        let cwd = app.cwd.clone();
                        let model = app.model.clone();
                        tokio::spawn(async move {
                            session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
                        });
                    }
                }
                AppEvent::AgentChunk { task_id, text } => {
                    // Subagent emitted a streaming text chunk — append to its
                    // task's message log so the task view shows live output
                    // rather than the "No messages yet" empty state. v126
                    // pipes nested-stream chunks the same way so the user
                    // can drill into a running agent and see what it's doing.
                    app.last_active_agent_task = Some(task_id.clone());
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
                            tool_use_count: 0,
                            latest_input_tokens: 0,
                            cumulative_output_tokens: 0,
                            model_used: Some(app.model.as_str().to_owned()),
                            max_input_tokens: None,
                            budget_killed: false,
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
                    tool_use_count,
                    input_tokens,
                    output_tokens,
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
                        if let Some(n) = tool_use_count {
                            bt.tool_use_count = n;
                        }
                        if let Some(n) = input_tokens {
                            bt.latest_input_tokens = n;
                        }
                        if let Some(n) = output_tokens {
                            // Cumulative — sum across every round-trip,
                            // matching v131's `cumulativeOutputTokens` field.
                            bt.cumulative_output_tokens =
                                bt.cumulative_output_tokens.saturating_add(n);
                        }
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
                AppEvent::TeammateSpawned {
                    name,
                    team_name,
                    agent_id,
                    color,
                    agent_type,
                    cwd,
                } => {
                    // Activate the team if this is the first teammate to
                    // join — switches the leader from "no team" to "running
                    // a team" so the teammate tree, send-message routing,
                    // and per-team context all light up.
                    if app.team_context.team_name.is_none() {
                        app.team_context.team_name = Some(team_name.clone());
                        app.team_context.team_file_path =
                            Some(crate::swarm::team_helpers::team_file_path(&team_name));
                        app.team_context.lead_agent_id = Some(crate::swarm::types::make_agent_id(
                            crate::swarm::TEAM_LEAD_NAME,
                            &team_name,
                        ));
                    }
                    // Register the teammate in the in-memory roster. The
                    // render code reads this to draw the teammate tree and
                    // power per-name lookups; previously the HashMap stayed
                    // empty regardless of how many teammates spawned.
                    app.team_context.teammates.insert(
                        agent_id.clone(),
                        crate::swarm::types::TeammateInfo {
                            name: name.clone(),
                            agent_type,
                            color,
                            cwd,
                            spawned_at: std::time::Instant::now(),
                            backend: crate::swarm::types::BackendType::InProcess,
                        },
                    );
                }
                AppEvent::ExitPlanModeRequested { plan } => {
                    // Surface the plan as an assistant transcript
                    // entry so the user can review it. Then transition
                    // out of Plan into AcceptEdits so the model can
                    // proceed with destructive edits in the next turn.
                    // v132's behaviour is identical: plan text is
                    // shown, mode flips, agent continues.
                    tracing::info!(
                        target: "jfc::ui::plan_mode",
                        plan_bytes = plan.len(),
                        from_mode = ?app.permission_mode,
                        "ExitPlanMode: surfacing plan + transitioning out of Plan"
                    );
                    let body = format!(
                        "**Plan presented (Plan Mode → Accept Edits)**\n\n---\n\n{plan}"
                    );
                    app.messages
                        .push(crate::types::ChatMessage::assistant(body));
                    if matches!(app.permission_mode, app::PermissionMode::Plan) {
                        app.permission_mode = app::PermissionMode::AcceptEdits;
                        crate::toast::push_with_cap(
                            &mut app.toasts,
                            crate::toast::Toast::new(
                                crate::toast::ToastKind::Success,
                                "Plan approved — mode: Accept Edits",
                            ),
                        );
                        crate::system_reminder::append_to_last_user(
                            &mut app.messages,
                            "Permission mode flipped from `Plan` to `AcceptEdits`. \
                             Edit/Write/Bash now auto-approve. Continue executing the plan.",
                        );
                    }
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
        }
    }

    Ok(())
}

fn draw_synchronized(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> io::Result<()> {
    let _ = execute!(io::stdout(), BeginSynchronizedUpdate);
    let res = terminal.draw(|f| render::frame(f, app));
    let _ = execute!(io::stdout(), EndSynchronizedUpdate);
    res.map(|_| ())
}

async fn read_git_branch(cwd: &std::path::Path) -> Option<String> {
    let mut dir = cwd.to_path_buf();
    loop {
        let head = dir.join(".git/HEAD");
        // Try the read directly; `read_to_string` returns Err if missing,
        // which is cheaper than a separate `metadata` probe + read.
        if let Ok(content) = tokio::fs::read_to_string(&head).await {
            let trimmed = content.trim();
            if let Some(rest) = trimmed.strip_prefix("ref: refs/heads/") {
                return Some(rest.to_owned());
            }
            return Some("(detached)".to_owned());
        }
        if !dir.pop() {
            return None;
        }
    }
}

fn set_terminal_title(app: &App) {
    use std::sync::Mutex;
    use std::sync::OnceLock;
    static LAST: OnceLock<Mutex<String>> = OnceLock::new();
    let last = LAST.get_or_init(|| Mutex::new(String::new()));
    let cwd_label = std::path::Path::new(app.cwd.as_str())
        .file_name()
        .and_then(|n| n.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| app.cwd.clone());
    // "(N new)" prefix is shown when the user has scrolled up from
    // the bottom of the transcript while content is arriving — the
    // count is the number of message lines pushed below the
    // viewport since the last time we were at the bottom. Streaming
    // alone (without scroll-away) doesn't trigger the badge, since
    // the user is already watching.
    let lines_below = app
        .total_lines
        .saturating_sub(app.scroll_offset + app.viewport_height);
    let prefix = if !app.follow_bottom && lines_below > 0 {
        format!("({} new) ", lines_below)
    } else if app.is_streaming {
        "● ".to_owned()
    } else {
        String::new()
    };
    let title = format!("{}jfc · {} · {}", prefix, app.model, cwd_label);
    let mut guard = match last.lock() {
        Ok(g) => g,
        Err(p) => p.into_inner(),
    };
    if *guard == title {
        return;
    }
    *guard = title.clone();
    let _ = execute!(io::stdout(), SetTitle(title));
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
