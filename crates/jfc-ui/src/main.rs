mod agents;
mod app;
mod auto_mode;
mod compact;
mod context;
mod inline_tools;
mod input;
mod markdown;
mod provider;
mod providers;
mod render;
mod scheduler;
mod session;
mod stream;
mod tasks;
mod theme;
mod tools;
mod types;

use std::{io, sync::Arc, time::Duration};

use crossterm::{
    event::{
        self, DisableMouseCapture, EnableMouseCapture, Event, KeyEventKind,
        KeyboardEnhancementFlags, PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use futures::StreamExt;
use ratatui::{Terminal, backend::CrosstermBackend};
use tokio::sync::mpsc;

use app::{App, AppEvent, PendingApproval, SPINNER, TICK_MS};
use provider::Provider;
use providers::{AnthropicOAuthProvider, AnthropicProvider, OpenWebUIProvider};
use types::*;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
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
    let model = init.model;
    let oauth_handle = init.oauth;
    let provider = providers[active_idx].clone();

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let kbd_enhanced = enable_keyboard_enhancement(&mut stdout);
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.clear()?;

    let result = run(&mut terminal, providers, provider, model, oauth_handle).await;

    if kbd_enhanced {
        let _ = execute!(terminal.backend_mut(), PopKeyboardEnhancementFlags);
    }
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
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
        .unwrap_or_else(|_| EnvFilter::new("info,reqwest=warn,hyper=warn,h2=warn"));

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
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
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
    model: String,
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
        .unwrap_or_else(|_| "claude-opus-4-5".to_string());

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
    model: String,
    oauth_handle: Option<Arc<AnthropicOAuthProvider>>,
) -> anyhow::Result<()> {
    let (tx, mut rx) = mpsc::unbounded_channel::<AppEvent>();
    let mut app = App::new(provider, model);
    app.providers = providers.clone();

    // Kick off background model-list fetches so the picker reflects what each provider
    // actually serves (e.g., the user's OpenWebUI instance) instead of stale hardcoded
    // ids that produce "Model not found" at stream time.
    for p in &providers {
        let tx = tx.clone();
        let p = Arc::clone(p);
        let name = p.name().to_owned();
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

    app.sync_task_completions();
    terminal.draw(|f| render::frame(f, &mut app))?;

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
                        let in_input = mouse.row as usize
                            >= app
                                .viewport_height
                                .saturating_add(app.scroll_offset)
                                .saturating_sub(2);
                        if !in_input {
                            yank_last_assistant(&app);
                        }
                    }
                    _ => {}
                }
            }
            AppEvent::Term(_) => {}
            AppEvent::Tick => {
                app.spinner_frame = (app.spinner_frame + 1) % SPINNER.len();
            }
            AppEvent::StreamChunk { text, reasoning } => {
                if let Some(chunk) = text {
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
                            blocked: decision.should_block,
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
                app.is_streaming = false;
                app.streaming_text.clear();
                app.streaming_reasoning.clear();
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
                app.is_streaming = false;
                app.streaming_text.clear();
                app.streaming_reasoning.clear();
                app.streaming_assistant_idx = None;
                app.messages
                    .push(ChatMessage::assistant(format!("**Error:** {e}")));
                app.scroll_to_bottom();
            }
            AppEvent::StreamUsage {
                input_tokens,
                output_tokens,
            } => {
                app.last_usage_input = input_tokens;
                app.last_usage_output = output_tokens;
                app.tool_ctx.approx_tokens = input_tokens as usize + output_tokens as usize;
            }
            AppEvent::ToolResult { tool_id, result } => {
                tracing::info!(
                    target: "jfc::stream",
                    tool_id = %tool_id,
                    is_error = result.is_error,
                    output_len = result.output.len(),
                    "tool_result received"
                );
                let mut found = false;
                for msg in &mut app.messages {
                    for part in &mut msg.parts {
                        if let MessagePart::Tool(tc) = part {
                            if tc.id == tool_id {
                                tc.output = ToolOutput::Text(result.output.clone());
                                tc.status = if result.is_error {
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
            }
            AppEvent::AllToolsComplete => {
                if compact::should_compact(&app.messages, app.max_context_tokens) {
                    let _ = tx.send(AppEvent::CompactionStarted);
                    let messages = app.messages.clone();
                    let provider = Arc::clone(&app.provider);
                    let model = app.model.clone();
                    let mut tool_ctx = app.tool_ctx.clone();
                    let tx_compact = tx.clone();
                    tokio::spawn(async move {
                        let options = provider::StreamOptions::new(model);
                        let result =
                            compact::compact(&messages, provider.as_ref(), &options, &mut tool_ctx)
                                .await;
                        match result {
                            compact::CompactResult::Success {
                                messages,
                                pre_tokens,
                                post_tokens,
                            } => {
                                let _ = tx_compact.send(AppEvent::CompactionDone {
                                    messages,
                                    tool_ctx,
                                    pre_tokens,
                                    post_tokens,
                                });
                            }
                            compact::CompactResult::Unsupported
                            | compact::CompactResult::TooFewGroups => {}
                            compact::CompactResult::CircuitBreakerTripped => {
                                let _ = tx_compact.send(AppEvent::CompactionFailed(
                                    "Circuit breaker tripped — compaction keeps refilling".into(),
                                ));
                            }
                            compact::CompactResult::Exhausted { attempts } => {
                                let _ = tx_compact.send(AppEvent::CompactionFailed(format!(
                                    "Exhausted {attempts} compaction attempts"
                                )));
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
                    stream::continue_agentic_loop(&mut app, &tx).await;
                } else if !app.is_streaming
                    && app.pending_approval.is_none()
                    && app.approval_queue.is_empty()
                    && app.pending_tool_calls.is_empty()
                {
                    // Turn fully ended (model stopped, no more agentic loop
                    // iterations, no pending tools). v126 input queue: drain
                    // any prompts the user typed during streaming.
                    drain_queued_prompts(&mut app, &tx).await;
                }
            }
            AppEvent::CompactionStarted => {}
            AppEvent::CompactionDone {
                messages,
                tool_ctx,
                pre_tokens: _,
                post_tokens,
            } => {
                app.messages = messages;
                app.tool_ctx = tool_ctx;
                app.tool_ctx.approx_tokens = post_tokens;
            }
            AppEvent::CompactionFailed(_reason) => {}
            AppEvent::ModelsLoaded { provider, models } => {
                app.provider_models.insert(provider, models);
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
        }

        app.sync_task_completions();
        terminal.draw(|f| render::frame(f, &mut app))?;
    }

    Ok(())
}

fn update_task_activities(app: &mut app::App, calls: &[types::ToolCall]) {
    let in_progress: Vec<String> = app
        .task_store
        .list(false)
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
