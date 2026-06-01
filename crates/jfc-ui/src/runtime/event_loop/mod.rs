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

fn prioritize_terminal_events(events: &mut Vec<AppEvent>) {
    if events.len() < 2
        || !events
            .iter()
            .any(|event| matches!(event, AppEvent::Ui(UiEvent::Term(_))))
    {
        return;
    }

    let mut terminal_events = Vec::new();
    let mut other_events = Vec::with_capacity(events.len());
    for event in events.drain(..) {
        if matches!(event, AppEvent::Ui(UiEvent::Term(_))) {
            terminal_events.push(event);
        } else {
            other_events.push(event);
        }
    }
    terminal_events.extend(other_events);
    *events = terminal_events;
}

pub(crate) async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    providers: Vec<Arc<dyn Provider>>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    oauth_handle: Option<Arc<crate::providers::AnthropicOAuthProvider>>,
    startup_session: crate::StartupSession,
    initial_prompt: Option<String>,
    initial_permission_mode: Option<crate::app::PermissionMode>,
    cli_config: crate::CliRuntimeConfig,
) -> anyhow::Result<()> {
    let (tx, mut rx): (EventSender, EventReceiver) = mpsc::channel(APP_EVENT_BUFFER);
    let (term_tx, mut term_rx) = mpsc::unbounded_channel::<event::Event>();
    // Make the channel reachable from non-Task code paths (bounty
    // solver/validator agents, future cron-triggered work) so they
    // emit the same TaskStarted/AgentChunk/TaskCompleted events the
    // fan UI + ctrl+X panel render. Mirrors register_active_provider.
    crate::tools::register_event_sender(tx.clone());
    tracing::info!(target: "jfc::ui::events", "registered AppEvent sender for non-Task agent paths");
    let mut app = App::new(provider, model);
    app.providers = providers.clone();
    // Transfer CLI-flag-derived runtime config onto the App. Done here
    // (not inside `App::new`) so unit tests that build a bare `App` can
    // skip the plumbing and so the flag → field mapping lives next to
    // the flag parser instead of buried deep in app/state.rs. Wiring
    // the *consumers* of these fields (stream builder, permission
    // gate, MCP init, session save) is a separate change — the App
    // fields are `#[allow(dead_code)]` until those land.
    app.max_turns = cli_config.max_turns;
    app.max_budget_usd = cli_config.max_budget_usd;
    app.allowed_tools = cli_config.allowed_tools;
    app.disallowed_tools = cli_config.disallowed_tools;
    app.cli_system_prompt = cli_config.system_prompt;
    app.dangerously_skip_permissions = cli_config.dangerously_skip_permissions;
    app.json_mode = cli_config.json_mode;
    app.extra_dirs = cli_config.extra_dirs;
    app.cli_max_thinking_tokens = cli_config.max_thinking_tokens;
    app.cli_thinking_display = cli_config.thinking_display;
    app.no_session_persistence = cli_config.no_session_persistence;
    app.cli_task_budget = cli_config.task_budget;
    app.mcp_config_path = cli_config.mcp_config_path;
    app.cowork = cli_config.cowork;
    app.local_advisor_model = cli_config.local_advisor_model.clone();
    app.advisor_enabled = app.advisor_enabled || app.local_advisor_model.is_some();
    app.server_advisor_model = cli_config.server_advisor_model.clone();
    app.custom_betas = cli_config.custom_betas;
    app.fine_grained_tool_streaming = cli_config.fine_grained_tool_streaming;
    app.strict_tool_schemas = cli_config.strict_tool_schemas;
    let startup_config = config::load_arc();

    // Remote-control auto-start: from --remote-control flag or config.
    let rc_wanted = cli_config.remote_control
        || startup_config
            .remote_control
            .as_ref()
            .is_some_and(|rc| rc.auto_start);
    if rc_wanted {
        let rc_port = startup_config
            .remote_control
            .as_ref()
            .map(|rc| rc.port)
            .unwrap_or(jfc_remote::protocol::DEFAULT_PORT);
        match crate::remote_host::RemoteHost::start(rc_port, tx.clone()).await {
            Ok(host) => {
                tracing::info!(
                    target: "jfc::remote",
                    addr = %host.addr(),
                    token = %host.token,
                    "remote-control auto-started at launch"
                );
                app.remote_host = Some(host);
            }
            Err(e) => {
                tracing::warn!(
                    target: "jfc::remote",
                    error = %e,
                    "failed to auto-start remote-control"
                );
            }
        }
    }

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
    // `--dangerously-skip-permissions` overrides any explicit
    // `--permission-mode` (the user asked for "no prompts ever";
    // bypass is the strongest mode and the closest match). Logged
    // loud — this is the foot-gun flag.
    if app.dangerously_skip_permissions {
        tracing::warn!(
            target: "jfc::ui",
            "--dangerously-skip-permissions: forcing permission mode to BypassPermissions"
        );
        app.permission_mode = crate::app::PermissionMode::BypassPermissions;
    }
    // Apply the user's persisted theme choice from
    // ~/.config/jfc/config.toml. Unknown / missing names fall back
    // silently to the default dark theme set by App::new.
    if let Some(name) = startup_config.theme.as_deref()
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
    if let Some(name) = startup_config.output_style.as_deref() {
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

    // AutoDefaultNudge: show a one-time notice that auto is the default
    // permission mode. Only fires when the gate is enabled AND the
    // marker file `~/.config/jfc/auto_nudge_seen` does not exist.
    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::AutoDefaultNudge) {
        let marker = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("jfc")
            .join("auto_nudge_seen");
        if !marker.exists() {
            app.messages.push(crate::types::ChatMessage::assistant(
                "\u{2139}\u{fe0f} Auto mode is now the default permission mode. Use /permissions to change.".to_string(),
            ));
            // Create the marker file so the nudge doesn't repeat.
            if let Some(parent) = marker.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&marker, "seen");
            tracing::info!(
                target: "jfc::auto_nudge",
                marker = %marker.display(),
                "auto-default-nudge shown and marker created"
            );
        }
    }

    // Wire the Slate router from config. Default OFF — `slate_enabled = false`
    // in `~/.config/jfc/config.toml` means `app.slate = None` and every turn
    // uses the pinned `app.model` (legacy behavior). When ON, each user
    // submission consults the router to pick a per-turn model based on the
    // classifier's `QueryClass`. See `crates/jfc-ui/src/slate.rs`.
    {
        if startup_config.slate_enabled {
            let rules = config::slate_rules_from_config(&startup_config);
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
            // Prefer a session from *this* project (cwd-scoped, codex-rs /
            // v126 parity). Only when the current cwd has no sessions at all
            // do we fall back to the globally-most-recent — and we flag that
            // so the user isn't silently dropped into an unrelated project's
            // transcript (the "`--continue` resumed the wrong repo" footgun).
            let mut continued_foreign_cwd = false;
            let id = match jfc_session::most_recent_session_for_cwd(cwd_str.as_deref()).await {
                Some(id) => Some(id),
                None => {
                    let fallback = jfc_session::most_recent_session().await;
                    if fallback.is_some() {
                        continued_foreign_cwd = true;
                    }
                    fallback
                }
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
                // Task store: prefer the project store (`.jfc/tasks.json`)
                // that `App::new` already opened — it survives across every
                // session in the repo. ONLY fall back to the per-session store
                // (`~/.config/jfc/tasks/<session>.json`) when there's no git
                // root. Unconditionally reopening the per-session store here
                // was the `--continue` subj/desc-resurrection bug: it clobbered
                // the live project store with stale placeholder rows.
                if !matches!(app.git_root, Some(Some(_))) {
                    app.task_store = jfc_session::TaskStore::open(session_id.as_str());
                }
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
                    if let Some(resolved) = crate::resolve_provider_model(&app.providers, &model_id)
                    {
                        tracing::info!(
                            target: "jfc::session",
                            model = %model_id,
                            routed_provider = %resolved.provider.name(),
                            "rerouting active provider to match saved session model"
                        );
                        app.provider = resolved.provider;
                        app.model = resolved.model;
                    } else {
                        app.model = model_id.into();
                    }
                }
                app.recompute_token_estimate();
                // If we fell back to the globally-most-recent session because
                // this cwd had none of its own, the resumed transcript belongs
                // to a *different* project. Surface that instead of silently
                // dropping the user into an unrelated repo's history.
                if continued_foreign_cwd {
                    tracing::warn!(
                        target: "jfc::session",
                        cwd = ?cwd_str,
                        "no session for this cwd — continued the globally-most-recent session from another project"
                    );
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(
                            crate::toast::ToastKind::Warning,
                            "No session for this directory — continued the most recent session from \
                             another project. Use `--resume <id>` or start fresh if that's not what you wanted."
                                .to_string(),
                        ),
                    );
                }
                let hl_cache_path = std::env::current_dir()
                    .unwrap_or_default()
                    .join(".jfc/highlight-heights.json");
                let hl_loaded = jfc_markdown::load_highlight_line_counts(&hl_cache_path);
                if hl_loaded > 0 {
                    tracing::debug!(
                        target: "jfc::session",
                        hl_loaded,
                        "pre-seeded highlight line-count cache"
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
                // Same project-store-first rule as --continue (see above):
                // keep the project `.jfc/tasks.json` and only fall back to the
                // per-session store when no git root exists. Prevents the
                // resumed session's old per-session placeholders from
                // overwriting the live project task list.
                if !matches!(app.git_root, Some(Some(_))) {
                    app.task_store = jfc_session::TaskStore::open(session_id.as_str());
                }
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
                    if let Some(resolved) = crate::resolve_provider_model(&app.providers, &model_id)
                    {
                        tracing::info!(
                            target: "jfc::session",
                            model = %model_id,
                            routed_provider = %resolved.provider.name(),
                            "rerouting active provider to match saved session model"
                        );
                        app.provider = resolved.provider;
                        app.model = resolved.model;
                    } else {
                        app.model = model_id.into();
                    }
                }
                app.recompute_token_estimate();
                let hl_cache_path = std::env::current_dir()
                    .unwrap_or_default()
                    .join(".jfc/highlight-heights.json");
                let hl_loaded = jfc_markdown::load_highlight_line_counts(&hl_cache_path);
                if hl_loaded > 0 {
                    tracing::debug!(
                        target: "jfc::session",
                        hl_loaded,
                        "pre-seeded highlight line-count cache from disk (resume)"
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
        crate::StartupSession::Fork(source_id) => {
            // Fork: load messages from the source session, but mint a new session ID.
            let source_session_id = crate::ids::SessionId::new(source_id.clone());
            if let Some((messages, _saved_model)) =
                session::load_session_with_model(&source_session_id).await
            {
                let new_id = crate::ids::SessionId::new(uuid::Uuid::new_v4().to_string());
                tracing::info!(
                    target: "jfc::session",
                    source = %source_session_id,
                    new_session = %new_id,
                    message_count = messages.len(),
                    "forking session"
                );
                app.messages = messages;
                app.current_session_id = Some(new_id);
                app.recompute_token_estimate();
            } else {
                // Try loading from teleport export
                let export_path =
                    std::path::Path::new(".jfc/teleport").join(format!("{source_id}.json"));
                if export_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&export_path)
                        && let Ok(export) = serde_json::from_str::<serde_json::Value>(&content)
                    {
                        let new_id = crate::ids::SessionId::new(uuid::Uuid::new_v4().to_string());
                        // Load messages from the export
                        if let Some(msgs) = export.get("messages").and_then(|m| m.as_array()) {
                            for msg in msgs {
                                let role =
                                    msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                                let content =
                                    msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                let chat_msg = if role == "assistant" {
                                    crate::types::ChatMessage::assistant(content.to_owned())
                                } else {
                                    crate::types::ChatMessage::user(content.to_owned())
                                };
                                app.messages.push(chat_msg);
                            }
                        }
                        app.current_session_id = Some(new_id);
                        app.recompute_token_estimate();
                    }
                } else {
                    tracing::warn!(
                        target: "jfc::session",
                        source_id = %source_id,
                        "fork source not found, starting fresh"
                    );
                }
            }
        }
    }
    restore_persistent_background_agents(&mut app);

    // Check for pending historian transcripts from previous sessions.
    crate::learn_lifecycle::on_session_start(&app.cwd);

    // Apply persisted reasoning_effort from config.toml. MUST run AFTER
    // the --continue/--resume block above (which may switch `app.model` to
    // the session's saved model) so the effort resolves for the ACTUAL
    // model in use, not the initial CLI-provided one.
    {
        let cfg = crate::config::load_arc();
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
        if let Some(temperature) = crate::exploration::temperature_from_env()
            .or_else(|| crate::exploration::resolve_temperature_for_model(&cfg, &app.model))
        {
            tracing::info!(
                target: "jfc::exploration",
                temperature,
                model = %app.model,
                "applied persisted/session temperature (post-session-restore)"
            );
            let _ = app.temperature_state.set(temperature);
        }
        app.exploration_state
            .configure(crate::exploration::ExplorationSettings::from_config(&cfg));
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
            _ = tx
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
                _ = tx
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
        tokio::spawn(async move {
            let mut reader = event::EventStream::new();
            while let Some(ev) = reader.next().await {
                match ev {
                    Ok(ev) => {
                        _ = term_tx.send(ev);
                    }
                    Err(error) => {
                        tracing::warn!(
                            target: "jfc::ui::input",
                            %error,
                            "terminal event read failed; keeping input reader alive"
                        );
                    }
                }
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
                _ = tx.try_send(AppEvent::Ui(UiEvent::Tick));
            }
        });
    }

    // Forward teammate runner events into the main event channel.
    {
        let tx = tx.clone();
        let mut teammate_rx = app.teammate_event_rx.take().unwrap();
        tokio::spawn(async move {
            while let Some(ev) = teammate_rx.recv().await {
                _ = tx.send(AppEvent::Team(TeamEvent::Runner(ev))).await;
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
        let mcp_configs = crate::config::load_arc().mcp.clone();
        let tx_mcp = tx.clone();
        tokio::spawn(async move {
            crate::mcp::register_servers_from_config(&registry, &mcp_configs).await;
            // Notify UI so the sidebar shows server status.
            let servers = registry
                .list()
                .await
                .iter()
                .map(|s| McpServerInfo {
                    name: s.name.clone(),
                    status: match s.status {
                        crate::mcp::McpServerStatus::Connected => McpStatus::Connected,
                        crate::mcp::McpServerStatus::Failed => McpStatus::Error,
                        crate::mcp::McpServerStatus::Disabled => McpStatus::Disabled,
                    },
                })
                .collect();
            _ = tx_mcp
                .send(AppEvent::Provider(ProviderEvent::McpUpdated { servers }))
                .await;
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
        let cfg = crate::config::load_arc();
        app.exploration_state.begin_turn(&prompt, &cfg);
        let tx_clone = tx.clone();
        let interrupt = app.interrupt_flag.clone();
        // wg-async: --prompt startup spawns a stream that holds critical
        // state (SSE conn + tx). Wire the cancel token in so an early
        // ESC can drop it cleanly.
        app.cancel_token = tokio_util::sync::CancellationToken::new();
        let cancel = app.cancel_token.clone();
        let prev_msg_id = app.last_response_id.take();
        // Refresh CLAUDE.md frontmatter disallowed tools before each turn.
        if let Ok(cwd_path) = std::env::current_dir() {
            let hierarchy = crate::context::ClaudeMdHierarchy::load(&cwd_path);
            app.claudemd_disallowed_tools = hierarchy.collect_disallowed_tools();
        }
        let overrides = StreamRequestOverrides {
            background_reminders: app.take_background_reminders(),
            disallowed_tools: app.effective_disallowed_tools(),
            allowed_tools: app.allowed_tools.clone(),
            custom_betas: app.custom_betas.clone(),
            fine_grained_tool_streaming: app.fine_grained_tool_streaming,
            strict_tool_schemas: app.strict_tool_schemas,
            task_budget: app.cli_task_budget,
            max_thinking_tokens: app.cli_max_thinking_tokens,
            thinking_display: app.cli_thinking_display.clone(),
            brief_mode: app.brief_mode,
            ..Default::default()
        };
        let tx_guard = tx.clone();
        // Inner task's abort handle parked on App so the watchdog can
        // forcefully abort the actual stream_response task (see
        // App::active_stream_handle). Aborting the outer supervisor would
        // only drop its JoinHandle to the inner task, detaching rather than
        // cancelling it.
        let inner = tokio::spawn(async move {
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
        });
        app.active_stream_handle = Some(inner.abort_handle());
        tokio::spawn(async move {
            if let Err(join_err) = inner.await {
                let msg = if join_err.is_panic() {
                    format!("stream task panicked: {join_err}")
                } else {
                    format!("stream task cancelled: {join_err}")
                };
                _ = tx_guard
                    .send(AppEvent::Stream(StreamEvent::Error(msg)))
                    .await;
            }
        });
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
    let mut term_events_open = true;

    'main_loop: loop {
        // Burst-recv: block on the first event, then drain everything currently
        // queued without re-awaiting. Process them all, draw once at the end.
        // This collapses N rapid stream chunks into 1 frame instead of N frames.
        let first_event = loop {
            if !term_events_open {
                break match rx.recv().await {
                    Some(e) => e,
                    None => break 'main_loop,
                };
            }
            tokio::select! {
                biased;
                term = term_rx.recv() => {
                    match term {
                        Some(ev) => break AppEvent::Ui(UiEvent::Term(ev)),
                        None => term_events_open = false,
                    }
                }
                app_event = rx.recv() => {
                    break match app_event {
                        Some(e) => e,
                        None => break 'main_loop,
                    };
                }
            }
        };
        let mut events: Vec<AppEvent> = vec![first_event];
        // Cap burst draining to prevent starvation: at most 256 events per
        // iteration so producers can't endlessly refill while we drain.
        const BURST_CAP: usize = 256;
        while events.len() < BURST_CAP {
            if term_events_open {
                match term_rx.try_recv() {
                    Ok(term) => {
                        events.push(AppEvent::Ui(UiEvent::Term(term)));
                        continue;
                    }
                    Err(mpsc::error::TryRecvError::Empty) => {}
                    Err(mpsc::error::TryRecvError::Disconnected) => {
                        term_events_open = false;
                    }
                }
            }
            match rx.try_recv() {
                Ok(extra) => events.push(extra),
                Err(_) => break,
            }
        }
        prioritize_terminal_events(&mut events);

        // Track whether any event in this burst dirties the screen. Pure Tick
        // events with no streaming/animation skip the draw entirely — eliminates
        // ~12.5 idle redraws per second.
        let mut needs_draw = std::mem::take(&mut pending_draw);
        let mut should_quit = false;
        let mut force_draw = false;

        for ev in events {
            // Mirror to remote-control clients. Non-blocking; returns early
            // when remote control is inactive.
            if let Some(ref rc) = app.remote_host
                && let Some(envelope) = crate::remote_host::mirror_event(&ev)
            {
                rc.mirror(envelope);
            }

            // Tick alone doesn't dirty the screen; everything else does. The
            // streaming-animation guard below re-enables Tick-driven redraws
            // when there's actually motion to show.
            if !ev.is_tick() {
                needs_draw = true;
            }

            match ev {
                // ── Terminal input (key, paste, mouse) ───────────────────
                AppEvent::Ui(UiEvent::Term(ev)) => {
                    // Human input should echo immediately. Stream/tool events
                    // can be coalesced behind FRAME_BUDGET, but deferring a
                    // keypress right after a session-load draw makes the first
                    // typed character appear "missing" until the next key/tick.
                    force_draw = true;
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
                AppEvent::Stream(StreamEvent::ThinkingTokens(tokens)) => {
                    handlers::stream_chunk::handle_thinking_tokens(&mut app, tokens);
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
                AppEvent::Tool(ToolEvent::SetInProgressToolUseIds { action, ids }) => {
                    handlers::tools::handle_set_in_progress_tool_use_ids(&mut app, action, ids);
                }
                AppEvent::Tool(ToolEvent::DeferredToolUse {
                    id,
                    name,
                    input_preview,
                    reason,
                }) => {
                    handlers::tools::handle_deferred_tool_use(
                        &mut app,
                        id,
                        name,
                        input_preview,
                        reason,
                    );
                }
                AppEvent::Tool(ToolEvent::UseSummary {
                    summary,
                    preceding_tool_use_ids,
                }) => {
                    handlers::tools::handle_tool_use_summary(
                        &mut app,
                        summary,
                        preceding_tool_use_ids,
                    );
                }
                AppEvent::Stream(StreamEvent::ServerToolResult {
                    tool_use_id,
                    tool_kind,
                    content,
                }) => {
                    handlers::stream_tool::handle_server_tool_result(
                        &mut app,
                        &tx,
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

                // ── Stream: fallback ────────────────────────────────────
                AppEvent::Stream(StreamEvent::FallbackTriggered {
                    original_model,
                    fallback_model,
                    reason,
                }) => {
                    handlers::stream_error::handle_fallback_triggered(
                        &mut app,
                        &original_model,
                        &fallback_model,
                        &reason,
                    );
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
                    handlers::tools::handle_tool_result(&mut app, &tx, tool_id, result);
                    if handlers::tools::should_recheck_completion_after_tool_result(&app) {
                        tracing::warn!(
                            target: "jfc::stream",
                            "ToolResult completed a turn after its AllComplete signal — rechecking continuation"
                        );
                        handlers::tools::handle_all_complete(&mut app, &tx).await;
                    }
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
                AppEvent::Ui(UiEvent::WorktreeCountLoaded(count)) => {
                    app.worktree_count = count;
                }
                AppEvent::Ui(UiEvent::RemoteApprovalResponse {
                    tool_use_id,
                    approved,
                }) => {
                    crate::input::handle_remote_approval_response(
                        &mut app,
                        &tx,
                        tool_use_id,
                        approved,
                    );
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
                AppEvent::WorkflowProgress(ev) => {
                    handlers::workflow::handle_workflow_progress(&mut app, ev);
                }
            }
        }

        // After processing all events in this burst, mirror derived state
        // to remote-control clients.
        if let Some(ref rc) = app.remote_host {
            // Session status (transition-only).
            let status = if app.is_streaming {
                jfc_remote::protocol::SessionState::Running
            } else if app.pending_approval.is_some() {
                jfc_remote::protocol::SessionState::WaitingApproval
            } else {
                jfc_remote::protocol::SessionState::Idle
            };
            rc.mirror_status(status);

            // Pending approval → PermissionRequest with diff preview.
            if let Some(ref approval) = app.pending_approval {
                let diff = crate::remote_host::tool_diff_preview(&approval.tool);
                rc.mirror_pending_approval(
                    approval.tool.id.as_ref(),
                    approval.tool.kind.label(),
                    approval.tool.input.summary(),
                    diff,
                );
            } else {
                rc.clear_pending_approval();
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
        if needs_draw && (force_draw || elapsed_since_draw >= FRAME_BUDGET) {
            // `terminal.draw` flushes stdout synchronously; `block_in_place`
            // tells the multi-threaded runtime to migrate other tasks off this
            // worker so they keep running while we hold the I/O.
            tokio::task::block_in_place(|| -> io::Result<()> {
                app.sync_task_completions();
                draw_synchronized(terminal, &mut app)?;
                set_terminal_title(&app);
                _ = execute!(
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

    // Post-session learning: fire historian to extract facts from this session's
    // transcript. Runs synchronously (blocking on exit is acceptable — it's a
    // single LLM call, ~2-5s) so the user's learning is captured before the
    // process exits. Best-effort: failures are logged, never surfaced.
    crate::learn_lifecycle::on_session_end(&app.messages, &app.cwd);

    Ok(())
}

/// Walk the config to pick the right `reasoning_effort` for `model`.
///
/// Precedence (first hit wins):
///   0. ANY layer's `ultracode = true` → force `"xhigh"` (matches CC 2.1.154's
///      `e$7` which returns "xhigh" whenever `settings.ultracode === true`
///      regardless of `effortLevel`). Checked in narrow-to-wide order so an
///      agent-level `ultracode = false` can opt out of a `[default]` ultracode.
///   1. `[agents.<exact-model-id>]` — direct match on the full model id
///   2. `[agents.<bare-model-id>]` — match the model id without provider prefix
///   3. `[default]` — fallback effort if no agent block matches
///
/// Returns `None` when none of those layers define an effort, so we leave
/// the runtime at "server default" instead of forcing medium.
fn resolve_effort_for_model(cfg: &crate::config::Config, model: &str) -> Option<String> {
    let bare = model.rsplit('/').next().unwrap_or(model);
    // 0: ultracode override — first explicit boolean wins (narrow → wide).
    // `Some(true)` forces xhigh; `Some(false)` opts out for that layer and
    // also short-circuits broader layers (mirrors the way explicit settings
    // override defaults in CC's settings merge).
    let ultracode_override = [
        cfg.agents.get(model).and_then(|a| a.ultracode),
        (bare != model)
            .then(|| cfg.agents.get(bare).and_then(|a| a.ultracode))
            .flatten(),
        cfg.default.ultracode,
    ]
    .into_iter()
    .find_map(|layer| layer);
    if let Some(ultracode) = ultracode_override {
        return if ultracode {
            Some("xhigh".to_owned())
        } else {
            None
        };
    }
    // 1: exact model id (e.g. "anthropic/claude-opus-4-7")
    if let Some(agent) = cfg.agents.get(model)
        && let Some(ref e) = agent.reasoning_effort
    {
        return Some(e.clone());
    }
    // 2: bare id after the provider slash (e.g. "claude-opus-4-7")
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
mod event_priority_tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;

    #[test]
    fn terminal_events_are_prioritized_within_burst_robust() {
        let mut events = vec![
            AppEvent::Stream(StreamEvent::Chunk {
                text: Some("first".to_owned()),
                reasoning: None,
            }),
            AppEvent::Ui(UiEvent::Tick),
            AppEvent::Ui(UiEvent::Term(Event::Key(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )))),
            AppEvent::Stream(StreamEvent::Chunk {
                text: Some("second".to_owned()),
                reasoning: None,
            }),
        ];

        prioritize_terminal_events(&mut events);

        assert!(matches!(&events[0], AppEvent::Ui(UiEvent::Term(_))));
        assert!(matches!(
            &events[1],
            AppEvent::Stream(StreamEvent::Chunk { .. })
        ));
        assert!(matches!(&events[2], AppEvent::Ui(UiEvent::Tick)));
        assert!(matches!(
            &events[3],
            AppEvent::Stream(StreamEvent::Chunk { .. })
        ));
    }
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

    // ── ultracode override (CC 2.1.154 e$7 parity) ─────────────────────────

    // Normal: [default] ultracode = true overrides every effort layer.
    #[test]
    fn ultracode_default_forces_xhigh_normal() {
        let mut cfg = cfg_with(Some("low"), &[("claude-opus-4-7", "medium")]);
        cfg.default.ultracode = Some(true);
        assert_eq!(
            resolve_effort_for_model(&cfg, "anthropic/claude-opus-4-7"),
            Some("xhigh".to_string()),
            "ultracode must force xhigh"
        );
    }

    // Normal: [agents.<id>] ultracode = true forces xhigh for that model
    // even when [default] has a different effort.
    #[test]
    fn ultracode_agent_forces_xhigh_normal() {
        let mut cfg = cfg_with(Some("low"), &[]);
        cfg.agents.insert(
            "claude-opus-4-7".into(),
            AgentConfig {
                ultracode: Some(true),
                reasoning_effort: Some("medium".into()),
                ..Default::default()
            },
        );
        assert_eq!(
            resolve_effort_for_model(&cfg, "anthropic/claude-opus-4-7"),
            Some("xhigh".to_string())
        );
    }

    // Robust: agent ultracode = false opts out of [default] ultracode and
    // also short-circuits effort fall-through (matches "explicit narrow
    // wins over broad default" semantics).
    #[test]
    fn ultracode_agent_false_opts_out_of_default_robust() {
        let mut cfg = cfg_with(Some("low"), &[]);
        cfg.default.ultracode = Some(true);
        cfg.agents.insert(
            "claude-opus-4-7".into(),
            AgentConfig {
                ultracode: Some(false),
                ..Default::default()
            },
        );
        assert_eq!(
            resolve_effort_for_model(&cfg, "anthropic/claude-opus-4-7"),
            None,
            "explicit agent ultracode=false must opt out of [default] ultracode"
        );
    }

    // Robust: ultracode = None at every layer is invisible — the normal
    // effort precedence applies unchanged. Pins backwards-compat with
    // existing configs.
    #[test]
    fn ultracode_unset_uses_normal_effort_precedence_robust() {
        let cfg = cfg_with(Some("high"), &[]);
        assert_eq!(
            resolve_effort_for_model(&cfg, "anthropic/claude-opus-4-7"),
            Some("high".to_string())
        );
    }
}
