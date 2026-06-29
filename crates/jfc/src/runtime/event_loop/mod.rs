use std::{io, sync::Arc, time::Duration};

use crossterm::{cursor::SetCursorStyle, event, execute};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use crate::app::{ANIM_TICK_MS, App, IDLE_TICK_MS};
use crate::runtime::{
    APP_EVENT_BUFFER, EngineEvent, EventReceiver, EventSender, ProviderEvent,
    StreamRequestOverrides, TeamEvent, draw_synchronized, restore_persistent_background_agents,
    set_terminal_title,
};
use jfc_core::*;
use jfc_engine::config;
use jfc_engine::diagnostics_producer;
use jfc_engine::lsp_client;
use jfc_engine::session;
use jfc_engine::slate;
use jfc_engine::stream;
use jfc_provider::{ModelId, Provider, ProviderId};

use crossterm::event::Event as TermEvent;

/// TUI-local frontend events: raw terminal input and the frame tick. These
/// never enter the engine — every other former `UiEvent` variant became a
/// [`ControlEvent`] or [`FrontendEvent`].
pub enum UiEvent {
    Term(TermEvent),
    Tick,
}

/// The TUI event-loop's merged event type: terminal input + frame ticks on
/// the frontend side, engine events on the other. TUI-only — this never
/// crosses into engine code.
pub enum AppEvent {
    Ui(UiEvent),
    Engine(EngineEvent),
}

impl AppEvent {
    pub fn is_tick(&self) -> bool {
        matches!(self, Self::Ui(UiEvent::Tick))
    }
}

fn app_event_kind(event: &AppEvent) -> &'static str {
    match event {
        AppEvent::Ui(UiEvent::Term(term)) => term_event_kind(term),
        AppEvent::Ui(UiEvent::Tick) => "ui.tick",
        AppEvent::Engine(engine) => engine_event_kind(engine),
    }
}

fn term_event_kind(event: &TermEvent) -> &'static str {
    match event {
        TermEvent::FocusGained => "ui.term.focus_gained",
        TermEvent::FocusLost => "ui.term.focus_lost",
        TermEvent::Key(_) => "ui.term.key",
        TermEvent::Mouse(_) => "ui.term.mouse",
        TermEvent::Paste(_) => "ui.term.paste",
        TermEvent::Resize(_, _) => "ui.term.resize",
    }
}

fn engine_event_kind(event: &EngineEvent) -> &'static str {
    match event {
        EngineEvent::ScopedStream { .. } => "engine.scoped_stream",
        EngineEvent::Stream(_) => "engine.stream",
        EngineEvent::Tool(_) => "engine.tool",
        EngineEvent::Compaction(_) => "engine.compaction",
        EngineEvent::Provider(_) => "engine.provider",
        EngineEvent::Task(_) => "engine.task",
        EngineEvent::Team(_) => "engine.team",
        EngineEvent::Goal(_) => "engine.goal",
        EngineEvent::Voice(_) => "engine.voice",
        EngineEvent::WorkflowProgress(_) => "engine.workflow_progress",
        EngineEvent::Control(_) => "engine.control",
        EngineEvent::Frontend(_) => "engine.frontend",
    }
}

fn event_burst_counts(events: &[AppEvent]) -> (u64, u64, u64) {
    let mut term = 0u64;
    let mut tick = 0u64;
    let mut engine = 0u64;
    for event in events {
        match event {
            AppEvent::Ui(UiEvent::Term(_)) => term = term.saturating_add(1),
            AppEvent::Ui(UiEvent::Tick) => tick = tick.saturating_add(1),
            AppEvent::Engine(_) => engine = engine.saturating_add(1),
        }
    }
    (term, tick, engine)
}

pub(crate) mod handlers;

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

fn effective_cwd_filter(cwd: &str) -> Option<&str> {
    (!cwd.is_empty()).then_some(cwd)
}

fn highlight_cache_path(cwd: &str) -> Option<std::path::PathBuf> {
    effective_cwd_filter(cwd).map(|cwd| {
        std::path::Path::new(cwd)
            .join(".jfc")
            .join("highlight-heights.json")
    })
}

pub(crate) async fn run(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    providers: Vec<Arc<dyn Provider>>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    oauth_handle: Option<Arc<jfc_engine::providers::AnthropicOAuthProvider>>,
    startup_session: crate::StartupSession,
    initial_prompt: Option<String>,
    initial_permission_mode: Option<crate::app::PermissionMode>,
    cli_config: crate::CliRuntimeConfig,
) -> anyhow::Result<()> {
    let _linkscope_loop = linkscope::trace("ui.event_loop");
    linkscope::record_rss("ui.event_loop.start");
    let (tx, mut rx): (EventSender, EventReceiver) = mpsc::channel(APP_EVENT_BUFFER);
    let (term_tx, mut term_rx) = mpsc::unbounded_channel::<event::Event>();
    // Frontend-local tick channel. Frame ticks never enter the engine bus —
    // capacity 2 + try_send preserves the old drop-when-busy coalescing.
    let (ui_tx, mut ui_rx) = mpsc::channel::<UiEvent>(2);
    // Make the channel reachable from non-Task code paths (bounty
    // solver/validator agents, future cron-triggered work) so they
    // emit the same TaskStarted/AgentChunk/TaskCompleted events the
    // fan UI + ctrl+X panel render. Mirrors register_active_provider.
    jfc_engine::tools::register_event_sender(tx.clone());
    tracing::info!(target: "jfc::ui::events", "registered AppEvent sender for non-Task agent paths");
    let mut app = App::new(provider, model);
    app.engine.providers = providers.clone();
    // Transfer CLI-flag-derived runtime config onto the App. Done here
    // (not inside `App::new`) so unit tests that build a bare `App` can
    // skip the plumbing and so the flag → field mapping lives next to
    // the flag parser instead of buried deep in app/state.rs. Wiring
    // the *consumers* of these fields (stream builder, permission
    // gate, MCP init, session save) lives across focused handlers.
    app.engine.max_turns = cli_config.max_turns;
    app.engine.max_budget_usd = cli_config.max_budget_usd;
    app.engine.allowed_tools = cli_config.allowed_tools;
    app.engine.disallowed_tools = cli_config.disallowed_tools;
    app.engine.cli_system_prompt = cli_config.system_prompt;
    app.engine.dangerously_skip_permissions = cli_config.dangerously_skip_permissions;
    app.json_mode = cli_config.json_mode;
    app.engine.extra_dirs = cli_config.extra_dirs;
    app.engine.cli_max_thinking_tokens = cli_config.max_thinking_tokens;
    app.engine.cli_thinking_display = cli_config.thinking_display;
    app.engine.no_session_persistence = cli_config.no_session_persistence;
    app.engine.cli_task_budget = cli_config.task_budget;
    app.engine.mcp_config_path = cli_config.mcp_config_path;
    app.engine.cowork = cli_config.cowork;
    app.engine.quiet_mode = cli_config.quiet;
    if let Some(ref cwd_override) = cli_config.cwd_override {
        if let Ok(canonical) = cwd_override.canonicalize() {
            app.engine.cwd = canonical.display().to_string();
        } else {
            app.engine.cwd = cwd_override.display().to_string();
        }
        let _ = app.reload_plugin_status_fresh();
    }
    app.engine.local_advisor_provider = cli_config.local_advisor_provider.clone();
    app.engine.local_advisor_model = cli_config.local_advisor_model.clone();
    app.engine.advisor_enabled =
        app.engine.advisor_enabled || app.engine.local_advisor_model.is_some();
    app.engine.server_advisor_model = cli_config.server_advisor_model.clone();
    app.engine.custom_betas = cli_config.custom_betas;
    app.engine.fine_grained_tool_streaming = cli_config.fine_grained_tool_streaming;
    app.engine.strict_tool_schemas = cli_config.strict_tool_schemas;
    app.plugins_disabled_by_managed_policy = cli_config.plugins_disabled_by_managed_policy;
    let startup_config = config::load_arc();

    // Feature: session GC — remove stale DB sessions at startup so the
    // transcript store doesn't grow unbounded. Fires as a background task
    // so it doesn't block the TUI from appearing. Respects `session_max_age_days`
    // (0 = disabled) and `session_min_keep`.
    {
        let max_age = startup_config.session_max_age_days;
        let min_keep = startup_config.session_min_keep;
        tokio::spawn(async move {
            match jfc_session::gc_old_sessions(max_age, min_keep).await {
                Ok(0) => {}
                Ok(n) => tracing::info!(
                    target: "jfc::session::gc",
                    deleted = n,
                    max_age_days = max_age,
                    min_keep,
                    "gc_old_sessions: pruned stale sessions"
                ),
                Err(e) => tracing::warn!(
                    target: "jfc::session::gc",
                    error = %e,
                    "gc_old_sessions: error during session GC"
                ),
            }
        });
    }

    // Opt-in council-verdict: config `council_verdict = true` OR the env var
    // (already applied in EngineState::default). Either source enables it.
    app.engine.council_verdict_enabled =
        app.engine.council_verdict_enabled || startup_config.council_verdict.unwrap_or(false);
    for dir in &startup_config.claude.permissions.additional_directories {
        let path = std::path::PathBuf::from(dir);
        if !app.engine.extra_dirs.contains(&path) {
            app.engine.extra_dirs.push(path);
        }
    }
    if let Some(sandbox) = startup_config.sandbox.as_ref() {
        let bash_sandbox = jfc_engine::sandbox::bash_sandbox_config_from_settings(sandbox);
        if bash_sandbox.enabled {
            jfc_engine::sandbox::install_bash_sandbox_config(bash_sandbox.clone());
        }
        app.engine.bash_sandbox = bash_sandbox;
    }
    let prompt_rewrite = startup_config.prompt_rewrite.clone().unwrap_or_default();
    app.engine.prompt_rewrite = prompt_rewrite.enabled.then_some(prompt_rewrite);
    app.engine.refusal_rewrite_retry_enabled = startup_config.refusal_rewrite_retry_enabled;
    app.engine.refusal_rewrite_retry_max = startup_config.refusal_rewrite_retry_max;

    // Remote-control auto-start: from --remote-control flag or config.
    let rc_disabled = startup_config
        .remote_control
        .as_ref()
        .is_some_and(|rc| rc.disabled);
    let rc_wanted = cli_config.remote_control
        || startup_config
            .remote_control
            .as_ref()
            .is_some_and(|rc| rc.auto_start);
    if rc_wanted && !rc_disabled {
        let rc_port = startup_config
            .remote_control
            .as_ref()
            .map(|rc| rc.port)
            .unwrap_or(jfc_remote::protocol::DEFAULT_PORT);
        match jfc_engine::remote_host::RemoteHost::start(rc_port, tx.clone()).await {
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

    // Token-audit dashboard: opt-in via `[dashboard]` config. Serves a local
    // web page that polls a JSON snapshot of context composition + per-model
    // usage/cost. A default launch starts nothing.
    if startup_config
        .dashboard
        .as_ref()
        .is_some_and(|dashboard| dashboard.enabled)
    {
        // Turn on in-process pipeline profiling (near-zero cost otherwise) so the
        // audit panel can show phase timings (turn.submit / turn.compact / ...).
        crate::cli::profiling::enable_linkscope_for_dashboard();
        let port = startup_config
            .dashboard
            .as_ref()
            .map(|dashboard| dashboard.port)
            .unwrap_or(jfc_config::DEFAULT_DASHBOARD_PORT);
        let handle = jfc_dashboard::new_handle();
        match jfc_dashboard::spawn(handle.clone(), &format!("127.0.0.1:{port}")) {
            Ok(server) => {
                tracing::info!(
                    target: "jfc::dashboard",
                    addr = %server.local_addr,
                    "token dashboard auto-started at launch"
                );
                jfc_engine::toast::push_with_cap(
                    &mut app.engine.toasts,
                    jfc_engine::toast::Toast::new(
                        jfc_engine::toast::ToastKind::Info,
                        format!("token dashboard → http://{}", server.local_addr),
                    ),
                );
                app.dashboard = Some(handle);
            }
            Err(error) => {
                tracing::warn!(
                    target: "jfc::dashboard",
                    %error,
                    "failed to start token dashboard"
                );
            }
        }
    }

    jfc_engine::claude_status::spawn_status_poll(tx.clone());
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
        app.engine.permission_mode = mode;
    }
    // `--dangerously-skip-permissions` overrides any explicit
    // `--permission-mode` (the user asked for "no prompts ever";
    // bypass is the strongest mode and the closest match). Logged
    // loud — this is the foot-gun flag.
    if app.engine.dangerously_skip_permissions {
        tracing::warn!(
            target: "jfc::ui",
            "--dangerously-skip-permissions: forcing permission mode to BypassPermissions"
        );
        app.engine.permission_mode = crate::app::PermissionMode::BypassPermissions;
    }
    // Apply the user's persisted theme choice from
    // ~/.config/jfc/config.toml. Unknown / missing names fall back
    // silently to the default Claude theme set by App::new.
    if let Some(name) = startup_config.theme.as_deref()
        && let Some(choice) = crate::theme::Theme::choice_by_name(name)
        && let Some(theme) = crate::theme::Theme::by_name(choice.name)
    {
        tracing::info!(target: "jfc::ui::theme", theme = %choice.name, "applied persisted theme");
        app.theme = theme;
        app.active_theme_name = choice.name.to_owned();
        // The render cache stores `Vec<Line<'static>>` with syntect highlight
        // colors baked in from the previous theme. Switching themes without
        // invalidating would serve stale-colored lines until each entry is
        // naturally evicted by the LRU. At boot the cache is empty so this is
        // a no-op, but we keep symmetry with the `/theme` handler so future
        // refactors don't introduce a regression.
        tracing::debug!(target: "jfc::render::cache", "theme switch — invalidating cache");
        app.render_cache.borrow_mut().clear();
        app.height_index.borrow_mut().clear();
        crate::markdown::clear_highlight_cache();
    }
    if let Some(name) = startup_config.output_style.as_deref() {
        let parsed = jfc_engine::output_style::OutputStyle::from_str_loose(name);
        jfc_engine::output_style::set_active_named(name);
        tracing::info!(
            target: "jfc::ui::output_style",
            style = %jfc_engine::output_style::active().name(),
            "applied persisted output style"
        );
        app.engine.output_style = parsed;
    }
    crate::markdown::set_syntax_highlighting_disabled(
        startup_config
            .claude
            .syntax_highlighting_disabled
            .unwrap_or(false),
    );

    // v132 Finch onboarding — first-run UI for users with no prior
    // session. Drops the help overlay automatically so they see the
    // keybindings + slash command catalog before typing. Suppressed
    // when the Finch feature gate is off (default for established
    // users). The gate flips itself off after the first successful
    // turn so the overlay doesn't repeat.
    if jfc_engine::feature_gates::is_enabled(jfc_engine::feature_gates::FeatureGate::Finch) {
        if !jfc_session::has_any_session().await {
            app.show_help = true;
            tracing::info!(
                target: "jfc::onboarding",
                "Finch onboarding active — showing help overlay"
            );
        }
    }

    // AutoDefaultNudge: show a one-time notice that auto is the default
    // permission mode. Only fires when the gate is enabled AND the
    // marker file `~/.config/jfc/auto_nudge_seen` does not exist AND
    // `show_startup_banner` is true (default).
    if startup_config.show_startup_banner
        && jfc_engine::feature_gates::is_enabled(
            jfc_engine::feature_gates::FeatureGate::AutoDefaultNudge,
        )
    {
        let marker = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("jfc")
            .join("auto_nudge_seen");
        if !marker.exists() {
            app.engine.messages.push(jfc_core::ChatMessage::assistant(
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
    // in `~/.config/jfc/config.toml` means `app.engine.slate = None` and every turn
    // uses the pinned `app.engine.model` (legacy behavior). When ON, each user
    // submission consults the router to pick a per-turn model based on the
    // classifier's `QueryClass`. See `crates/jfc/src/slate.rs`.
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
            app.engine.slate = Some(router);
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
            let cwd_str = app.engine.cwd.clone();
            // Prefer a session from *this* project (cwd-scoped, codex-rs /
            // v126 parity). Only when the current cwd has no sessions at all
            // do we fall back to the globally-most-recent — and we flag that
            // so the user isn't silently dropped into an unrelated project's
            // transcript (the "`--continue` resumed the wrong repo" footgun).
            let mut continued_foreign_cwd = false;
            let id = match jfc_session::most_recent_session_for_cwd(effective_cwd_filter(&cwd_str))
                .await
            {
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
                    cwd = %cwd_str,
                    "continuing most recent session"
                );
                app.engine.messages = messages;
                app.engine.current_session_id = Some(session_id.clone());
                // Task store: prefer the project store (`.jfc/tasks.json`)
                // that `App::new` already opened — it survives across every
                // session in the repo. ONLY fall back to the per-session store
                // (`~/.config/jfc/tasks/<session>.json`) when there's no git
                // root. Unconditionally reopening the per-session store here
                // was the `--continue` subj/desc-resurrection bug: it clobbered
                // the live project store with stale placeholder rows.
                if !matches!(app.engine.git_root, Some(Some(_))) {
                    app.engine.task_store = jfc_session::TaskStore::open(session_id.as_str());
                }
                // Rebuild any active stop-condition from the goal
                // sidecar — without this, /continue forgets the
                // user's goal and the next EndTurn settles silently.
                if let Some(goal) = jfc_engine::goal::load_sidecar(session_id.as_str()) {
                    tracing::info!(
                        target: "jfc::goal",
                        session_id = %session_id,
                        condition = %goal.condition,
                        iterations = goal.iterations,
                        "restored goal from sidecar"
                    );
                    app.engine.goal = Some(goal);
                }
                if let Some(model_id) = saved_model {
                    if let Some(resolved) =
                        crate::resolve_provider_model(&app.engine.providers, &model_id)
                    {
                        tracing::info!(
                            target: "jfc::session",
                            model = %model_id,
                            routed_provider = %resolved.provider.name(),
                            "rerouting active provider to match saved session model"
                        );
                        app.engine.provider = resolved.provider;
                        app.engine.model = resolved.model;
                    } else {
                        app.engine.model = model_id.into();
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
                        cwd = %cwd_str,
                        "no session for this cwd — continued the globally-most-recent session from another project"
                    );
                    jfc_engine::toast::push_with_cap(
                        &mut app.engine.toasts,
                        jfc_engine::toast::Toast::new(
                            jfc_engine::toast::ToastKind::Warning,
                            "No session for this directory — continued the most recent session from \
                             another project. Use `--resume <id>` or start fresh if that's not what you wanted."
                                .to_string(),
                        ),
                    );
                }
                if let Some(hl_cache_path) = highlight_cache_path(&cwd_str) {
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
        }
        crate::StartupSession::Resume(session_id) => {
            let session_id = jfc_engine::ids::SessionId::new(session_id);
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
                let current_cwd = app.engine.cwd.clone();
                if let Some(msg) =
                    jfc_session::cwd_mismatch_message(session_cwd.as_deref(), &current_cwd)
                {
                    tracing::warn!(
                        target: "jfc::session",
                        session_id = %session_id,
                        "{msg}"
                    );
                }
                app.engine.messages = messages;
                app.engine.current_session_id = Some(session_id.clone());
                // Same project-store-first rule as --continue (see above):
                // keep the project `.jfc/tasks.json` and only fall back to the
                // per-session store when no git root exists. Prevents the
                // resumed session's old per-session placeholders from
                // overwriting the live project task list.
                if !matches!(app.engine.git_root, Some(Some(_))) {
                    app.engine.task_store = jfc_session::TaskStore::open(session_id.as_str());
                }
                // Rebuild any active stop-condition from the goal sidecar.
                if let Some(goal) = jfc_engine::goal::load_sidecar(session_id.as_str()) {
                    tracing::info!(
                        target: "jfc::goal",
                        session_id = %session_id,
                        condition = %goal.condition,
                        iterations = goal.iterations,
                        "restored goal from sidecar"
                    );
                    app.engine.goal = Some(goal);
                }
                if let Some(model_id) = saved_model {
                    if let Some(resolved) =
                        crate::resolve_provider_model(&app.engine.providers, &model_id)
                    {
                        tracing::info!(
                            target: "jfc::session",
                            model = %model_id,
                            routed_provider = %resolved.provider.name(),
                            "rerouting active provider to match saved session model"
                        );
                        app.engine.provider = resolved.provider;
                        app.engine.model = resolved.model;
                    } else {
                        app.engine.model = model_id.into();
                    }
                }
                app.recompute_token_estimate();
                if let Some(hl_cache_path) = highlight_cache_path(&current_cwd) {
                    let hl_loaded = jfc_markdown::load_highlight_line_counts(&hl_cache_path);
                    if hl_loaded > 0 {
                        tracing::debug!(
                            target: "jfc::session",
                            hl_loaded,
                            "pre-seeded highlight line-count cache from disk (resume)"
                        );
                    }
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
            let source_session_id = jfc_engine::ids::SessionId::new(source_id.clone());
            if let Some((messages, _saved_model)) =
                session::load_session_with_model(&source_session_id).await
            {
                let new_id = jfc_engine::ids::SessionId::new(uuid::Uuid::new_v4().to_string());
                tracing::info!(
                    target: "jfc::session",
                    source = %source_session_id,
                    new_session = %new_id,
                    message_count = messages.len(),
                    "forking session"
                );
                app.engine.messages = messages;
                app.engine.current_session_id = Some(new_id);
                app.recompute_token_estimate();
            } else {
                // Try loading from teleport export
                let export_path =
                    std::path::Path::new(".jfc/teleport").join(format!("{source_id}.json"));
                if export_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&export_path)
                        && let Ok(export) = serde_json::from_str::<serde_json::Value>(&content)
                    {
                        let new_id =
                            jfc_engine::ids::SessionId::new(uuid::Uuid::new_v4().to_string());
                        // Load messages from the export
                        if let Some(msgs) = export.get("messages").and_then(|m| m.as_array()) {
                            for msg in msgs {
                                let role =
                                    msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
                                let content =
                                    msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                                let chat_msg = if role == "assistant" {
                                    jfc_core::ChatMessage::assistant(content.to_owned())
                                } else {
                                    jfc_core::ChatMessage::user(content.to_owned())
                                };
                                app.engine.messages.push(chat_msg);
                            }
                        }
                        app.engine.current_session_id = Some(new_id);
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
    restore_persistent_background_agents(&mut app.engine);

    // Feature: cross-session up-arrow history. Load user prompts from the N
    // most-recent sessions (sync/blocking, capped so startup latency stays
    // sub-ms on cold paths) and stash them in `app.prior_session_prompts` for
    // `user_prompts()` / `cmd_open_prompt_search` to include. Gated by the
    // `cross_session_history` config flag (default true).
    if startup_config.cross_session_history {
        load_prior_session_prompts(&mut app);
    }

    // Check for pending historian transcripts from previous sessions.
    jfc_engine::learn_lifecycle::on_session_start(&app.engine.cwd).await;

    // Apply persisted reasoning_effort from config.toml. MUST run AFTER
    // the --continue/--resume block above (which may switch `app.engine.model` to
    // the session's saved model) so the effort resolves for the ACTUAL
    // model in use, not the initial CLI-provided one.
    {
        let cfg = jfc_engine::config::load_arc();
        let effort_str = resolve_effort_for_model(&cfg, &app.engine.model);
        if let Some(level) = effort_str
            .as_deref()
            .and_then(jfc_engine::effort::ReasoningEffort::from_str_loose)
        {
            tracing::info!(
                target: "jfc::ui::effort",
                effort = %level,
                model = %app.engine.model,
                "applied persisted reasoning_effort (post-session-restore)"
            );
            app.engine.effort_state.set(level);
        }
        if let Some(temperature) = jfc_engine::exploration::temperature_from_env().or_else(|| {
            jfc_engine::exploration::resolve_temperature_for_model(&cfg, &app.engine.model)
        }) {
            tracing::info!(
                target: "jfc::exploration",
                temperature,
                model = %app.engine.model,
                "applied persisted/session temperature (post-session-restore)"
            );
            let _ = app.engine.temperature_state.set(temperature);
        }
        app.engine.exploration_state.configure(
            jfc_engine::exploration::ExplorationSettings::from_config(&cfg),
        );
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
                .send(EngineEvent::Provider(ProviderEvent::ModelsLoaded {
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
                    .send(EngineEvent::Provider(ProviderEvent::ProfileLoaded {
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
        let ui_tx = ui_tx.clone();
        let wants_anim = app.wants_animation_frame.clone();
        tokio::spawn(async move {
            loop {
                let ms = if wants_anim.load(std::sync::atomic::Ordering::Relaxed) {
                    ANIM_TICK_MS
                } else {
                    IDLE_TICK_MS
                };
                tokio::time::sleep(Duration::from_millis(ms)).await;
                _ = ui_tx.try_send(UiEvent::Tick);
            }
        });
    }

    // Forward teammate runner events into the main event channel.
    {
        let tx = tx.clone();
        let mut teammate_rx = app.engine.teammate_event_rx.take().unwrap();
        tokio::spawn(async move {
            while let Some(ev) = teammate_rx.recv().await {
                _ = tx.send(EngineEvent::Team(TeamEvent::Runner(ev))).await;
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

    let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
    jfc_engine::spawn_knowledge_maintenance_loop(cwd);

    // Optional pushed-diagnostics LSP client. It is opt-in because
    // rust-analyzer can eagerly prime caches for large workspaces on spawn;
    // the LSP tool still starts a one-shot server on demand.
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
        let registry = jfc_engine::mcp::McpRegistry::new();
        jfc_engine::tools::register_mcp_registry(registry.clone());
        let mcp_configs = jfc_engine::config::load_arc().mcp.clone();
        let tx_mcp = tx.clone();
        tokio::spawn(async move {
            jfc_engine::mcp::register_servers_from_config(&registry, &mcp_configs).await;
            // Notify UI so the sidebar shows server status.
            let servers = registry
                .list()
                .await
                .iter()
                .map(|s| McpServerInfo {
                    name: s.name.clone(),
                    status: match s.status {
                        jfc_engine::mcp::McpServerStatus::Connected => McpStatus::Connected,
                        jfc_engine::mcp::McpServerStatus::Failed => McpStatus::Error,
                        jfc_engine::mcp::McpServerStatus::Disabled => McpStatus::Disabled,
                    },
                })
                .collect();
            _ = tx_mcp
                .send(EngineEvent::Provider(ProviderEvent::McpUpdated { servers }))
                .await;
        });
    }

    app.engine.sync_task_completions();
    draw_synchronized(terminal, &mut app)?;
    // Initial terminal title — updates whenever the model or session
    // changes.
    set_terminal_title(&app);
    // OSC 0 window title: emit once at startup so the terminal's title bar
    // immediately shows the project name. `set_terminal_title` uses crossterm
    // SetTitle which most terminals also honour; this raw OSC sequence is the
    // universal fallback recognised by xterm, kitty, alacritty, iTerm2 et al.
    {
        use std::io::Write as _;
        let project = std::env::current_dir()
            .ok()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
            .unwrap_or_else(|| "jfc".into());
        print!("\x1b]0;jfc \u{2014} {project}\x07");
        let _ = std::io::stdout().flush();
    }

    // Submit initial prompt if provided via --prompt flag
    if let Some(prompt) = queued_initial_prompt {
        if jfc_engine::runtime::ops::refuse_budget_cap_if_reached(&mut app.engine) {
            return Ok(());
        }

        // Use the same logic as handle_submit but without waiting for user input
        let assistant_idx = app.engine.messages.len() + 1;
        app.engine.messages.push(ChatMessage::user(prompt.clone()));
        app.engine.tool_ctx.total_user_turns += 1;
        app.engine
            .messages
            .push(ChatMessage::assistant(String::new()));
        app.engine.streaming_assistant_idx = Some(assistant_idx);
        app.engine.is_streaming = true;
        let now = std::time::Instant::now();
        app.engine.streaming_started_at = Some(now);
        app.engine.last_stream_event_at = Some(now);
        app.engine.streaming_last_token_at = Some(now);
        app.engine.turn_started_at = Some(now);
        app.engine.turn_start_cost = jfc_engine::cost::total_cost(&app.engine.usage_by_model);
        app.engine.last_usage_output = 0;
        app.engine.usage_apply_baseline = (0, 0, 0, 0);

        // Create session if not resuming one
        let session_id = app
            .engine
            .current_session_id
            .clone()
            .unwrap_or_else(jfc_session::generate_session_id);
        {
            let sid = session_id.clone();
            let msgs = app.engine.messages.clone();
            let cwd = app.engine.cwd.clone();
            let model = app.engine.model.clone();
            tokio::spawn(async move {
                session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
            });
        }
        app.engine.current_session_id = Some(session_id);

        let provider = app.engine.provider.clone();
        let messages = stream::build_provider_messages(&app.engine.messages[..assistant_idx]);
        // Slate per-turn routing for the `--prompt` startup path.
        let model = if let Some(ref router) = app.engine.slate {
            router.route(&prompt, app.engine.model.clone())
        } else {
            app.engine.model.clone()
        };
        let cfg = jfc_engine::config::load_arc();
        app.engine.exploration_state.begin_turn(&prompt, &cfg);
        let interrupt = app.engine.interrupt_flag.clone();
        // wg-async: --prompt startup spawns a stream that holds critical
        // state (SSE conn + tx). Wire the cancel token in so an early
        // ESC can drop it cleanly.
        app.engine.cancel_token = tokio_util::sync::CancellationToken::new();
        let cancel = app.engine.cancel_token.clone();
        let prev_msg_id = app.engine.last_response_id.take();
        // Refresh CLAUDE.md frontmatter disallowed tools before each turn.
        if let Ok(cwd_path) = std::env::current_dir() {
            let hierarchy = jfc_engine::context::ClaudeMdHierarchy::load(&cwd_path);
            app.engine.claudemd_disallowed_tools = hierarchy.collect_disallowed_tools();
        }
        let overrides = StreamRequestOverrides {
            background_reminders: app.engine.take_background_reminders(),
            disallowed_tools: app.engine.effective_disallowed_tools(),
            allowed_tools: app.engine.allowed_tools.clone(),
            custom_betas: app.engine.custom_betas.clone(),
            fine_grained_tool_streaming: app.engine.fine_grained_tool_streaming,
            strict_tool_schemas: app.engine.strict_tool_schemas,
            task_budget: app.engine.cli_task_budget,
            max_thinking_tokens: app.engine.cli_max_thinking_tokens,
            thinking_display: app.engine.cli_thinking_display.clone(),
            brief_mode: app.engine.brief_mode,
            interaction_mode: app.engine.active_interaction_mode,
            ..Default::default()
        };
        jfc_engine::runtime::spawn_stream_response_scoped(
            &mut app.engine,
            &tx,
            provider,
            messages,
            model,
            interrupt,
            cancel,
            prev_msg_id,
            overrides,
        );
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
        let first_event = {
            let _linkscope_wait = linkscope::phase("ui.event_wait");
            loop {
                if !term_events_open {
                    tokio::select! {
                        biased;
                        ui = ui_rx.recv() => {
                            if let Some(u) = ui { break AppEvent::Ui(u); }
                        }
                        app_event = rx.recv() => {
                            break match app_event {
                                Some(e) => AppEvent::Engine(e),
                                None => break 'main_loop,
                            };
                        }
                    }
                    continue;
                }
                tokio::select! {
                    biased;
                    term = term_rx.recv() => {
                        match term {
                            Some(ev) => break AppEvent::Ui(UiEvent::Term(ev)),
                            None => term_events_open = false,
                        }
                    }
                    ui = ui_rx.recv() => {
                        if let Some(u) = ui { break AppEvent::Ui(u); }
                    }
                    app_event = rx.recv() => {
                        break match app_event {
                            Some(e) => AppEvent::Engine(e),
                            None => break 'main_loop,
                        };
                    }
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
            if let Ok(u) = ui_rx.try_recv() {
                events.push(AppEvent::Ui(u));
                continue;
            }
            match rx.try_recv() {
                Ok(extra) => events.push(AppEvent::Engine(extra)),
                Err(_) => break,
            }
        }
        prioritize_terminal_events(&mut events);
        let first_kind = app_event_kind(&events[0]);
        let (term_events, tick_events, engine_events) = event_burst_counts(&events);
        let event_count = usize_to_u64_saturating(events.len());
        linkscope::record_items("ui.event_burst", event_count);
        let _linkscope_burst = linkscope::phase("ui.event_burst");
        let _linkscope_burst_trace = linkscope::trace_fields(
            "ui.event_burst",
            [
                linkscope::TraceField::count("events", event_count),
                linkscope::TraceField::count("term_events", term_events),
                linkscope::TraceField::count("tick_events", tick_events),
                linkscope::TraceField::count("engine_events", engine_events),
                linkscope::TraceField::text("first_kind", first_kind),
                linkscope::TraceField::count("term_events_open", u64::from(term_events_open)),
            ],
        );

        // Track whether any event in this burst dirties the screen. Pure Tick
        // events with no streaming/animation skip the draw entirely — eliminates
        // ~12.5 idle redraws per second.
        let mut needs_draw = std::mem::take(&mut pending_draw);
        let mut should_quit = false;
        let mut force_draw = false;

        for ev in events {
            let event_kind = app_event_kind(&ev);
            linkscope::detail_event_fields(
                "ui.event.handle",
                [
                    linkscope::TraceField::text("kind", event_kind),
                    linkscope::TraceField::count("pending_draw", u64::from(pending_draw)),
                    linkscope::TraceField::count("needs_draw", u64::from(needs_draw)),
                ],
            );
            // Mirror engine events to remote-control clients. Non-blocking;
            // returns early when remote control is inactive. Frontend-local
            // events (keys, ticks) are never mirrored.
            if let AppEvent::Engine(ref engine_ev) = ev
                && !app.engine.is_stale_stream_event(engine_ev)
                && let Some(ref rc) = app.remote_host
                && let Some(envelope) = jfc_engine::remote_host::mirror_event(engine_ev)
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

                // ── Tick ────────────────────────────────────────────────
                AppEvent::Ui(UiEvent::Tick) => {
                    if handlers::tick::handle_tick(&mut app, &tx, oauth_for_snapshot.as_ref()).await
                    {
                        needs_draw = true;
                    }
                }

                AppEvent::Engine(crate::runtime::EngineEvent::Voice(ref voice_ev)) => {
                    handle_voice_event(&mut app, voice_ev, &tx).await;
                    needs_draw = true;
                }
                AppEvent::Engine(ev) => {
                    let elicit_before = app.engine.pending_elicitations.len();
                    match crate::runtime::handle_engine_event(&mut app.engine, &tx, ev).await? {
                        Some(crate::runtime::FrontendDirective::SubmitPrompt(submission)) => {
                            handlers::ui_actions::handle_submit(&mut app, submission, &tx).await?;
                        }
                        Some(crate::runtime::FrontendDirective::RunCommand(text)) => {
                            crate::input::run_slash_command_with_tx(&mut app, &text, &tx).await;
                        }
                        None => {}
                    }
                    // If a new elicitation arrived and there was none before,
                    // initialize the input state from its schema.
                    if app.engine.pending_elicitations.len() > elicit_before {
                        if let Some(e) = app.engine.pending_elicitations.front() {
                            app.elicitation_input = match &e.kind {
                                jfc_core::mcp_elicitation::ElicitationKind::Form {
                                    schema, ..
                                } => {
                                    crate::render::elicitation::ElicitationInputState::from_schema(
                                        schema,
                                    )
                                }
                                _ => crate::render::elicitation::ElicitationInputState::default(),
                            };
                        }
                    }
                }
            }
        }

        // Apply view-facing effects queued by engine handlers in this burst
        // (scroll pinning, render-cache invalidation).
        apply_engine_effects(&mut app, Some(&tx));

        // Publish one coherent token-audit snapshot per drained burst (no-op
        // unless the opt-in dashboard server is running).
        crate::runtime::dashboard::publish(&app);

        // After processing all events in this burst, mirror derived state
        // to remote-control clients.
        if let Some(ref rc) = app.remote_host {
            // Session status (transition-only).
            let status = if app.engine.is_streaming {
                jfc_remote::protocol::SessionState::Running
            } else if app.engine.pending_approval.is_some() {
                jfc_remote::protocol::SessionState::WaitingApproval
            } else {
                jfc_remote::protocol::SessionState::Idle
            };
            rc.mirror_status(status);

            // Pending approval → PermissionRequest with diff preview.
            if let Some(ref approval) = app.engine.pending_approval {
                let diff = jfc_engine::remote_host::tool_diff_preview(&approval.tool);
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
        let want_streaming_cursor = app.engine.is_streaming
            || app.engine.compacting_started_at.is_some()
            || app.engine.pipeline_busy_for_submit()
            || app.engine.pending_approval.is_some()
            || !app.engine.approval_queue.is_empty()
            || app
                .engine
                .background_tasks
                .values()
                .any(|bt| bt.status.is_alive())
            || app.engine.turn_started_at.is_some();
        if want_streaming_cursor {
            needs_draw = true;
        }

        let elapsed_since_draw = last_draw.elapsed();
        let should_draw_now = needs_draw && (force_draw || elapsed_since_draw >= FRAME_BUDGET);
        linkscope::detail_event_fields(
            "ui.draw.decision",
            [
                linkscope::TraceField::count("needs_draw", u64::from(needs_draw)),
                linkscope::TraceField::count("force_draw", u64::from(force_draw)),
                linkscope::TraceField::count("pending_draw", u64::from(pending_draw)),
                linkscope::TraceField::count(
                    "want_streaming_cursor",
                    u64::from(want_streaming_cursor),
                ),
                linkscope::TraceField::count(
                    "elapsed_since_draw_ms",
                    u64::try_from(elapsed_since_draw.as_millis()).unwrap_or(u64::MAX),
                ),
                linkscope::TraceField::count("should_draw_now", u64::from(should_draw_now)),
            ],
        );
        if should_draw_now {
            // `terminal.draw` flushes stdout synchronously; `block_in_place`
            // tells the multi-threaded runtime to migrate other tasks off this
            // worker so they keep running while we hold the I/O.
            let _linkscope_blocking_draw = linkscope::phase("ui.draw.block_in_place");
            tokio::task::block_in_place(|| -> io::Result<()> {
                app.engine.sync_task_completions();
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
    jfc_engine::learn_lifecycle::on_session_end(&app.engine.messages, &app.engine.cwd).await;

    Ok(())
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
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
fn resolve_effort_for_model(cfg: &jfc_engine::config::Config, model: &str) -> Option<String> {
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

/// Drain and apply the view-facing effects queued by engine handlers during
/// this burst. The engine never touches scroll/render state directly — it
/// queues `EngineEffect`s and this is the TUI's interpretation of them.
/// Headless frontends have their own (mostly no-op) interpretation.
pub(crate) fn apply_engine_effects(app: &mut App, tx: Option<&EventSender>) {
    let effects = std::mem::take(&mut app.engine.effects);
    for effect in effects {
        match effect {
            crate::app::EngineEffect::TranscriptAppended => {
                // Pre-warm the TTS socket on the FIRST delta of the turn — which
                // is often a reasoning/thinking chunk, before any spoken text —
                // so the first sentence isn't blocked on connect latency.
                if let Some(tx) = tx
                    && app.engine.is_streaming
                    && !app.voice_skip_next_stream_read_aloud
                {
                    crate::voice::read_aloud_prewarm(tx);
                }
                if app.engine.is_streaming
                    && let Some(idx) = app.engine.streaming_assistant_idx
                    && let Some(msg) = app.engine.messages.get(idx)
                    && let Some(jfc_core::MessagePart::Text(text)) = msg.parts.last()
                {
                    let total = crate::render::codex_stream::stream_pacer::display_line_count(text);
                    app.stream_pacer.reveal_all(total);
                    // Incremental read-aloud: speak the newly-streamed text as it
                    // arrives (cheap no-op unless read-aloud is enabled). Skipped
                    // when the voice-conversation path already produced audio.
                    if let Some(tx) = tx
                        && !app.voice_skip_next_stream_read_aloud
                    {
                        crate::voice::read_aloud_feed(text, tx);
                    }
                }
                // Follow content as it streams *only when the user is already
                // pinned to the bottom* — and freeze the viewport while a
                // mid-drag selection is active (autoscrolling would slide the
                // transcript out from under the highlight).
                let selecting = app.text_selection.is_some_and(|s| s.dragged);
                if app.follow_bottom && !selecting {
                    app.scroll_to_bottom();
                }
            }
            crate::app::EngineEffect::StreamingFinalized => {
                // One token-timeline sample per finalized request (edge-triggered:
                // this effect is drained once per round-trip).
                app.record_timeline_sample();
                app.render_cache.borrow_mut().clear_streaming();
                // End the incremental read-aloud turn (flush the trailing partial
                // sentence + drain playback). No-op if read-aloud is off or the
                // voice-conversation path handled audio.
                if app.voice_skip_next_stream_read_aloud {
                    app.voice_skip_next_stream_read_aloud = false;
                } else {
                    crate::voice::read_aloud_finish();
                }
            }
            crate::app::EngineEffect::ScrollToBottom => {
                app.scroll_to_bottom();
            }
            crate::app::EngineEffect::ToolOutputArrived => {
                app.path_yank_cursor = 0;
            }
            crate::app::EngineEffect::SessionSwitched => {
                app.task_panel.reset_selection();
                app.task_panel.reset_drilldown();
                app.recompute_token_estimate();
                // New session / cleared transcript: drop the timeline + baseline so
                // the next request doesn't emit a spurious giant first delta.
                app.reset_timeline();
            }
            crate::app::EngineEffect::ModelsRefreshed => {
                app.model_picker.query_cache.clear();
                if app.model_picker.visible {
                    let models = crate::input::collect_all_models(app);
                    app.model_picker.models = models;
                }
            }
            crate::app::EngineEffect::PromptRewriteProposed {
                original,
                rewrite,
                rationale,
                original_intent,
            } => {
                // Surface the proposal as a blocking modal — never apply it
                // silently. The user accepts (send rewrite), rejects (send
                // original), or edits (load rewrite into composer). See
                // `input::prompt_rewrite`.
                app.pending_rewrite_proposal = Some(crate::app::PromptRewriteProposal {
                    original,
                    rewrite,
                    rationale,
                    original_intent,
                });
            }
        }
    }
}

/// Handle a voice event from the jfc-voice STT pipeline.
async fn handle_voice_event(
    app: &mut App,
    ev: &crate::runtime::VoiceEvent,
    tx: &crate::runtime::EventSender,
) {
    use crate::runtime::VoiceEvent;
    use jfc_voice::VoiceState;
    match ev {
        VoiceEvent::StateChanged(raw) => {
            let state = match raw {
                0 => VoiceState::Idle,
                1 => VoiceState::Recording,
                _ => VoiceState::Processing,
            };
            app.voice_state = state;
            match state {
                VoiceState::Recording => {
                    // Barge-in: stop any in-progress read-aloud the instant the
                    // user starts speaking (VAD speech onset or PTT key-down).
                    crate::voice::read_aloud_barge_in();
                    // A fresh, wanted utterance has begun — stop suppressing
                    // voice-driven input edits. Any late events from a manually
                    // submitted (discarded) prior utterance have arrived before
                    // this onset, so it's safe to listen to the box again.
                    app.voice_suppress_input = false;
                    // Fresh recording — reset the level ring and hue time base.
                    app.voice_audio_levels.clear();
                    app.voice_record_started = Some(std::time::Instant::now());
                }
                VoiceState::Idle => {
                    app.voice_interim = None;
                    app.voice_audio_levels.clear();
                    app.voice_record_started = None;
                }
                VoiceState::Processing => {}
            }
        }
        VoiceEvent::Level(level) => {
            // Append to the rolling level ring (newest last), capped.
            let levels = &mut app.voice_audio_levels;
            if levels.len() >= crate::app::VOICE_AUDIO_LEVELS_CAP {
                levels.remove(0);
            }
            levels.push(*level);
        }
        VoiceEvent::Interim(text) => {
            // Drop late interims from an utterance the user already manually
            // submitted (Enter) — without this they re-hydrate the cleared box.
            if app.voice_suppress_input {
                tracing::debug!(
                    target: "jfc::voice",
                    "ignoring interim for manually-submitted (discarded) utterance"
                );
                return;
            }
            // Live transcription: type the partial transcript into the input
            // box, replacing the previous interim in place. CC types interims
            // live; we mirror that so the box feels alive while you speak.
            app.voice_interim = if text.is_empty() {
                None
            } else {
                Some(text.clone())
            };
            replace_interim_in_input(app, text);
        }
        VoiceEvent::Final(text) => {
            // Drop the late Final from an utterance the user already manually
            // submitted — otherwise it auto-submits a duplicate of what was just
            // sent (or re-hydrates the box in non-auto-submit modes).
            if app.voice_suppress_input {
                tracing::info!(
                    target: "jfc::voice",
                    chars = text.chars().count(),
                    "ignoring Final for manually-submitted (discarded) utterance"
                );
                app.voice_interim = None;
                return;
            }
            tracing::info!(
                target: "jfc::voice",
                chars = text.chars().count(),
                empty = text.is_empty(),
                "VoiceEvent::Final received → injecting transcript (→ auto-submit decision)"
            );
            app.voice_interim = None;
            // Clear the live interim text first so the final transcript
            // replaces it cleanly (no double-typing).
            clear_interim_from_input(app);
            if !text.is_empty() {
                inject_voice_transcript(app, text, tx).await;
            }
        }
        VoiceEvent::Error(msg) => {
            app.voice_interim = None;
            app.voice_state = VoiceState::Idle;
            // Drop any partial interim text we'd typed into the box.
            clear_interim_from_input(app);
            // Show as a toast notification
            let _ = tx
                .send(crate::runtime::EngineEvent::Control(
                    crate::runtime::ControlEvent::Notice {
                        kind: jfc_engine::toast::ToastKind::Error,
                        text: format!("Voice error: {msg}"),
                    },
                ))
                .await;
        }
        VoiceEvent::ReadAloudStarted { chars: _ } => {
            app.voice_read_aloud_active = true;
            app.voice_read_aloud_started = Some(std::time::Instant::now());
            app.voice_tts_word_timings.clear();
        }
        VoiceEvent::ReadAloudCompleted {
            audio_bytes,
            chunks_sent,
        } => {
            app.voice_read_aloud_active = false;
            app.voice_read_aloud_started = None;
            app.voice_read_aloud_last_stats = Some((*audio_bytes, *chunks_sent));
        }
        VoiceEvent::ReadAloudError(msg) => {
            app.voice_read_aloud_active = false;
            app.voice_read_aloud_started = None;
            let _ = tx
                .send(crate::runtime::EngineEvent::Control(
                    crate::runtime::ControlEvent::Notice {
                        kind: jfc_engine::toast::ToastKind::Warning,
                        text: format!("Read aloud failed: {msg}"),
                    },
                ))
                .await;
        }
        VoiceEvent::AssistantMessageStarted => {
            commit_voice_interim_as_user_message(app);
            start_voice_assistant_stream(app);
        }
        VoiceEvent::AssistantResponseCompleted => {
            finish_voice_assistant_stream(app);
        }
        VoiceEvent::TtsWord { text, pts_ms } => {
            if !text.is_empty() {
                if app.voice_tts_word_timings.len() >= crate::app::VOICE_TTS_TIMINGS_CAP {
                    app.voice_tts_word_timings.remove(0);
                }
                app.voice_tts_word_timings.push((text.clone(), *pts_ms));
            }
        }
    }
}

fn start_voice_assistant_stream(app: &mut App) {
    if app.engine.streaming_assistant_idx.is_some() {
        return;
    }
    let idx = app.engine.messages.len();
    app.engine
        .messages
        .push(jfc_core::ChatMessage::assistant(String::new()));
    app.engine.streaming_assistant_idx = Some(idx);
    app.engine.is_streaming = true;
    let now = std::time::Instant::now();
    app.engine.streaming_started_at = Some(now);
    app.engine.last_stream_event_at = Some(now);
    app.engine.streaming_last_token_at = Some(now);
    app.engine.turn_started_at = Some(now);
    app.engine.turn_start_cost = jfc_engine::cost::total_cost(&app.engine.usage_by_model);
    app.engine.streaming_text.clear();
    app.engine.streaming_reasoning.clear();
    app.engine.streaming_response_bytes = 0;
    app.engine.last_usage_output = 0;
    app.engine.usage_apply_baseline = (0, 0, 0, 0);
}

fn finish_voice_assistant_stream(app: &mut App) {
    app.engine.is_streaming = false;
    app.engine.active_stream_handle = None;
    app.engine.clear_active_stream_scope();
    app.engine.last_stream_event_at = None;
    app.engine.streaming_assistant_idx = None;
    app.engine.streaming_started_at = None;
    app.engine.streaming_last_token_at = None;
    app.engine.turn_started_at = None;
    app.engine.streaming_text.clear();
    app.engine.streaming_reasoning.clear();
    app.engine.streaming_response_bytes = 0;
    jfc_engine::runtime::session_save::force_save(&mut app.engine);
    app.voice_skip_next_stream_read_aloud = true;
    app.engine
        .push_effect(crate::app::EngineEffect::StreamingFinalized);
}

fn commit_voice_interim_as_user_message(app: &mut App) {
    let Some(text) = app
        .voice_interim
        .as_ref()
        .map(|text| text.trim())
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
    else {
        return;
    };
    clear_interim_from_input(app);
    app.voice_interim = None;
    let insert_at = app
        .engine
        .streaming_assistant_idx
        .filter(|idx| *idx <= app.engine.messages.len())
        .unwrap_or(app.engine.messages.len());
    app.engine
        .messages
        .insert(insert_at, jfc_core::ChatMessage::user(text));
    if let Some(idx) = app.engine.streaming_assistant_idx
        && idx >= insert_at
    {
        app.engine.streaming_assistant_idx = Some(idx.saturating_add(1));
    }
}

/// Replace the live interim transcript in the input box.
///
/// Deletes the chars from the previous interim, then types the new one.
/// Tracks `voice_interim_chars` so successive interims overwrite in place
/// rather than appending. Only safe to call while recording — the user
/// isn't typing concurrently in voice mode.
fn replace_interim_in_input(app: &mut App, text: &str) {
    use ratatui_textarea::CursorMove;
    // Delete the previous interim (cursor is at the end of it).
    if app.voice_interim_chars > 0 {
        app.textarea.move_cursor(CursorMove::End);
        for _ in 0..app.voice_interim_chars {
            app.textarea.delete_char();
        }
    }
    // Type the new interim.
    for ch in text.chars() {
        app.textarea.insert_char(ch);
    }
    app.voice_interim_chars = text.chars().count();
}

/// Remove any live interim text from the input box and reset the counter.
fn clear_interim_from_input(app: &mut App) {
    use ratatui_textarea::CursorMove;
    if app.voice_interim_chars > 0 {
        app.textarea.move_cursor(CursorMove::End);
        for _ in 0..app.voice_interim_chars {
            app.textarea.delete_char();
        }
        app.voice_interim_chars = 0;
    }
}

/// Inject the STT transcript into the textarea (and optionally submit).
async fn inject_voice_transcript(app: &mut App, text: &str, tx: &crate::runtime::EventSender) {
    let cfg = crate::voice::current_config();
    // The RUNNING VAD loop is the source of truth for hands-free mode: `/voice
    // vad` configures the recorder for VAD but doesn't persist `mode=vad` to the
    // config `current_config()` reads (it stays `Hold`), so cfg alone would say
    // "don't auto-submit" and the transcript would just sit in the box. A live
    // VAD loop ⇒ no key to press ⇒ MUST submit on its own, exactly like Enter.
    let vad_running = crate::voice::vad_loop_running().await;
    let auto_submit = vad_running || voice_auto_submit();
    tracing::info!(
        target: "jfc::voice",
        mode = ?cfg.mode,
        vad_loop_running = vad_running,
        configured_auto_submit = cfg.auto_submit,
        auto_submit,
        chars = text.chars().count(),
        "inject_voice_transcript: auto-submit decision (VAD loop running ⇒ submit like Enter)"
    );

    if auto_submit {
        tracing::info!(
            target: "jfc::voice",
            chars = text.chars().count(),
            "voice transcript auto-submitting as a turn"
        );
        // Clear any leftover input, then submit the transcript directly —
        // same path as pressing Enter. We don't type into the box first
        // because handle_submit doesn't drain it (the Enter path resets the
        // input before calling submit), which would leave the text behind.
        app.textarea.select_all();
        app.textarea.cut();
        handlers::ui_actions::handle_submit(app, text.to_owned().into(), tx)
            .await
            .unwrap_or_else(|err| {
                tracing::warn!(target: "jfc::voice", error = %err, "auto-submit failed");
            });
    } else {
        tracing::info!(
            target: "jfc::voice",
            chars = text.chars().count(),
            "voice transcript parked in input box (auto-submit off — press Enter to send)"
        );
        // No auto-submit: type the transcript into the box and leave it for
        // the user to edit and send manually.
        for ch in text.chars() {
            app.textarea.insert_char(ch);
        }
    }
}

fn voice_auto_submit() -> bool {
    #[cfg(test)]
    if let Some(value) = voice_auto_submit_override() {
        return value;
    }
    let cfg = crate::voice::current_config();
    voice_should_auto_submit(cfg.mode, cfg.auto_submit)
}

/// Whether a finalized voice transcript should be submitted as a turn
/// immediately, or merely typed into the input box for the user to review and
/// send by hand.
///
/// VAD (hands-free) mode has no key to press to send — the whole point is "just
/// talk" — so a completed utterance MUST become a turn on its own. Otherwise the
/// transcript silently piles up unsent in the input box, which is the "VAD
/// transcribes but never sends a turn" regression. The `autoSubmit` setting
/// only governs the keyed dictation modes (hold/tap), where the user may want
/// to review/edit before pressing Enter; it defaults to off there.
fn voice_should_auto_submit(mode: jfc_voice::VoiceMode, configured_auto_submit: bool) -> bool {
    matches!(mode, jfc_voice::VoiceMode::Vad) || configured_auto_submit
}

#[cfg(test)]
fn voice_auto_submit_override() -> Option<bool> {
    VOICE_AUTO_SUBMIT_OVERRIDE.with(|cell| cell.get())
}

// Per-thread (not process-global) so concurrently-scheduled `#[tokio::test]`s —
// each of which runs its current-thread runtime on its own worker thread — can
// independently set/reset this override without racing each other. A global
// `Mutex<Option<bool>>` here flaked `voice_final_replaces_live_interim` under
// full parallel load: one test's `AutoSubmitGuard::drop` reset the override to
// `None` while a sibling still needed `Some(false)`, so its Final event
// auto-submitted and blanked the input box.
#[cfg(test)]
thread_local! {
    static VOICE_AUTO_SUBMIT_OVERRIDE: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Populate `app.prior_session_prompts` from the most-recent past sessions.
///
/// Reads up to `MAX_SESSIONS` sessions from the DB transcript store.
/// De-duplication is caller-side (in `user_prompts` / `cmd_open_prompt_search`).
fn load_prior_session_prompts(app: &mut App) {
    const MAX_SESSIONS: usize = 10;
    const MAX_PROMPTS_PER_SESSION: usize = 50;

    let current_id = app
        .engine
        .current_session_id
        .as_ref()
        .map(|id| id.as_str().to_owned());
    let collected = jfc_session::prior_user_prompts(
        MAX_SESSIONS,
        MAX_PROMPTS_PER_SESSION,
        current_id.as_deref(),
    );
    let count = collected.len();
    app.prior_session_prompts = collected;
    tracing::debug!(
        target: "jfc::session::history",
        prompt_count = count,
        "loaded cross-session prompt history"
    );
}

#[cfg(test)]
mod event_priority_tests {
    use crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::runtime::StreamEvent;

    #[test]
    fn terminal_events_are_prioritized_within_burst_robust() {
        let mut events = vec![
            AppEvent::Engine(EngineEvent::Stream(StreamEvent::Chunk {
                text: Some("first".to_owned()),
                reasoning: None,
            })),
            AppEvent::Ui(UiEvent::Tick),
            AppEvent::Ui(UiEvent::Term(Event::Key(KeyEvent::new(
                KeyCode::Esc,
                KeyModifiers::NONE,
            )))),
            AppEvent::Engine(EngineEvent::Stream(StreamEvent::Chunk {
                text: Some("second".to_owned()),
                reasoning: None,
            })),
        ];

        prioritize_terminal_events(&mut events);

        assert!(matches!(&events[0], AppEvent::Ui(UiEvent::Term(_))));
        assert!(matches!(
            &events[1],
            AppEvent::Engine(EngineEvent::Stream(StreamEvent::Chunk { .. }))
        ));
        assert!(matches!(&events[2], AppEvent::Ui(UiEvent::Tick)));
        assert!(matches!(
            &events[3],
            AppEvent::Engine(EngineEvent::Stream(StreamEvent::Chunk { .. }))
        ));
    }
}

#[cfg(test)]
mod startup_cwd_tests {
    use super::*;

    #[test]
    fn effective_cwd_filter_uses_engine_cwd_after_override_regression() {
        assert_eq!(
            effective_cwd_filter("/home/cole/project"),
            Some("/home/cole/project")
        );
        assert_eq!(effective_cwd_filter(""), None);
    }

    #[test]
    fn highlight_cache_path_uses_effective_cwd_regression() {
        let path = highlight_cache_path("/tmp/jfc-project").unwrap();
        assert_eq!(
            path,
            std::path::PathBuf::from("/tmp/jfc-project/.jfc/highlight-heights.json")
        );
    }
}

#[cfg(test)]
mod voice_event_tests {
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;

    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for TestProvider {}

    fn app() -> App {
        App::new(Arc::new(TestProvider), "test-model")
    }

    fn input_text(app: &App) -> String {
        app.textarea.lines().join("\n")
    }

    struct AutoSubmitGuard;

    impl AutoSubmitGuard {
        fn set(value: bool) -> Self {
            VOICE_AUTO_SUBMIT_OVERRIDE.with(|cell| cell.set(Some(value)));
            Self
        }
    }

    impl Drop for AutoSubmitGuard {
        fn drop(&mut self) {
            VOICE_AUTO_SUBMIT_OVERRIDE.with(|cell| cell.set(None));
        }
    }

    #[tokio::test]
    async fn voice_final_writes_transcript_to_input_box_normal() {
        let mut app = app();
        let _guard = AutoSubmitGuard::set(false);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Final("hello from vad".to_owned()),
            &tx,
        )
        .await;

        assert_eq!(input_text(&app), "hello from vad");
        assert_eq!(app.voice_interim_chars, 0);
    }

    // --- VAD auto-submit regression -----------------------------------------
    // Bug: in VAD (hands-free) mode, speech was transcribed but never sent as a
    // turn — the transcript just piled up in the input box. The auto-submit gate
    // only consulted the `autoSubmit` setting (default false, documented "hold
    // mode only") and ignored the voice mode. VAD has no key to press to submit,
    // so it MUST force-submit regardless of the flag. These pin the decision so
    // the gate can't silently revert to a flag-only check.

    #[test]
    fn vad_mode_auto_submits_even_when_flag_is_off_regression() {
        use jfc_voice::VoiceMode;
        // The exact broken case: VAD + autoSubmit unset/false must still submit.
        assert!(voice_should_auto_submit(VoiceMode::Vad, false));
        assert!(voice_should_auto_submit(VoiceMode::Vad, true));
    }

    #[test]
    fn keyed_modes_honor_auto_submit_flag_normal() {
        use jfc_voice::VoiceMode;
        // Hold/tap dictation lets the user review before sending, so they honor
        // the configured flag and default to parking the transcript in the box.
        assert!(!voice_should_auto_submit(VoiceMode::Hold, false));
        assert!(voice_should_auto_submit(VoiceMode::Hold, true));
        assert!(!voice_should_auto_submit(VoiceMode::Tap, false));
        assert!(voice_should_auto_submit(VoiceMode::Tap, true));
    }

    #[tokio::test]
    async fn voice_final_auto_submits_and_drains_input_box_regression() {
        let mut app = app();
        // Simulate the VAD decision (force auto-submit) at the consumer seam.
        let _guard = AutoSubmitGuard::set(true);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Final("send me as a turn".to_owned()),
            &tx,
        )
        .await;

        // Auto-submit takes the submit branch, which drains the box (the text is
        // handed to handle_submit). The bug's symptom was the opposite: the
        // transcript left parked in the input, never becoming a turn.
        assert_eq!(input_text(&app), "");
        assert_eq!(app.voice_interim, None);
        assert_eq!(app.voice_interim_chars, 0);
    }

    #[tokio::test]
    async fn voice_final_replaces_live_interim_regression() {
        let mut app = app();
        let _guard = AutoSubmitGuard::set(false);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Interim("hello inter".to_owned()),
            &tx,
        )
        .await;
        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Final("hello final".to_owned()),
            &tx,
        )
        .await;

        assert_eq!(input_text(&app), "hello final");
        assert_eq!(app.voice_interim, None);
        assert_eq!(app.voice_interim_chars, 0);
    }

    // REGRESSION (rehydration after manual Enter): a manual submit while a VAD
    // utterance is still finalizing left `voice_suppress_input` set; late
    // Interim/Final events from that discarded utterance must NOT touch the
    // input box, and the next Recording onset must lift the suppression so a
    // fresh utterance types normally again.
    #[tokio::test]
    async fn suppressed_voice_events_do_not_rehydrate_input_regression() {
        let mut app = app();
        let _guard = AutoSubmitGuard::set(true); // VAD-style: Final would auto-submit
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        // User pressed Enter mid-utterance → input cleared, suppression armed.
        app.voice_suppress_input = true;

        // Late interim for the discarded utterance — must be ignored.
        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Interim("late partial".to_owned()),
            &tx,
        )
        .await;
        assert_eq!(input_text(&app), "", "late interim re-hydrated the box");
        assert_eq!(app.voice_interim_chars, 0);

        // Late Final for the discarded utterance — must not auto-submit/inject.
        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Final("late final".to_owned()),
            &tx,
        )
        .await;
        assert_eq!(
            input_text(&app),
            "",
            "late Final injected after manual submit"
        );
        assert_eq!(app.voice_interim, None);

        // Next utterance begins → suppression lifts, live typing resumes.
        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::StateChanged(1), // Recording
            &tx,
        )
        .await;
        assert!(
            !app.voice_suppress_input,
            "Recording onset must clear suppression"
        );

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Interim("new utterance".to_owned()),
            &tx,
        )
        .await;
        assert_eq!(input_text(&app), "new utterance");
    }

    #[tokio::test]
    async fn voice_error_clears_live_interim_regression() {
        let mut app = app();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Interim("partial".to_owned()),
            &tx,
        )
        .await;
        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Error("boom".to_owned()),
            &tx,
        )
        .await;

        assert_eq!(input_text(&app), "");
        assert_eq!(app.voice_interim, None);
        assert_eq!(app.voice_interim_chars, 0);
        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn read_aloud_events_update_playback_state_without_touching_input_normal() {
        let mut app = app();
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::ReadAloudStarted { chars: 12 },
            &tx,
        )
        .await;
        assert!(app.voice_read_aloud_active);
        assert!(app.voice_read_aloud_started.is_some());

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::ReadAloudCompleted {
                audio_bytes: 3200,
                chunks_sent: 2,
            },
            &tx,
        )
        .await;
        assert!(!app.voice_read_aloud_active);
        assert_eq!(app.voice_read_aloud_last_stats, Some((3200, 2)));
        assert_eq!(input_text(&app), "");

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::ReadAloudError("speaker offline".to_owned()),
            &tx,
        )
        .await;
        assert!(!app.voice_read_aloud_active);
        assert!(rx.recv().await.is_some());
    }

    #[tokio::test]
    async fn assistant_message_start_commits_voice_interim_before_assistant_regression() {
        let mut app = app();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::Interim("voice question".to_owned()),
            &tx,
        )
        .await;
        app.engine
            .messages
            .push(jfc_core::ChatMessage::assistant(String::new()));
        app.engine.streaming_assistant_idx = Some(0);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::AssistantMessageStarted,
            &tx,
        )
        .await;

        assert_eq!(input_text(&app), "");
        assert_eq!(app.voice_interim, None);
        assert_eq!(app.voice_interim_chars, 0);
        assert_eq!(app.engine.messages.len(), 2);
        assert!(matches!(app.engine.messages[0].role, jfc_core::Role::User));
        assert_eq!(
            app.engine.messages[0].parts[0].text_only(),
            "voice question"
        );
        assert!(matches!(
            app.engine.messages[1].role,
            jfc_core::Role::Assistant
        ));
        assert_eq!(app.engine.streaming_assistant_idx, Some(1));
    }

    #[tokio::test]
    async fn assistant_message_start_creates_voice_owned_stream_normal() {
        let mut app = app();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::AssistantMessageStarted,
            &tx,
        )
        .await;

        assert!(app.engine.is_streaming);
        assert_eq!(app.engine.streaming_assistant_idx, Some(0));
        assert_eq!(app.engine.messages.len(), 1);
        assert!(matches!(
            app.engine.messages[0].role,
            jfc_core::Role::Assistant
        ));

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::AssistantResponseCompleted,
            &tx,
        )
        .await;

        assert!(!app.engine.is_streaming);
        assert_eq!(app.engine.streaming_assistant_idx, None);
        assert!(app.voice_skip_next_stream_read_aloud);
    }

    #[tokio::test]
    async fn tts_word_events_record_karaoke_timing_normal() {
        let mut app = app();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::ReadAloudStarted { chars: 12 },
            &tx,
        )
        .await;
        handle_voice_event(
            &mut app,
            &crate::runtime::VoiceEvent::TtsWord {
                text: "hello".to_owned(),
                pts_ms: 42,
            },
            &tx,
        )
        .await;

        assert_eq!(app.voice_tts_word_timings, vec![("hello".to_owned(), 42)]);
    }
}

#[cfg(test)]
mod effort_resolve_tests {
    use super::*;
    use jfc_engine::config::{AgentConfig, Config};

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
