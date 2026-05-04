use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::style::Style;
use std::sync::Arc;
use tokio::sync::mpsc;
use tui_textarea::{CursorMove, TextArea};

use crate::app::{App, AppEvent, ApprovalChoice};
use crate::stream;
use crate::types::*;

/// No-op: approvable tools are already inserted into the assistant message at
/// `StreamTool` time (see `main.rs` handler) so the user can see what's queued.
/// Kept as a stub for the call sites in the approval handlers; the real
/// status update happens via `ToolResult` when the dispatched tool finishes.
fn insert_tool_into_message(_app: &mut App, _tool: &ToolCall) {
    // intentionally empty — the tool is already in `messages` from StreamTool.
}

fn reset_input(app: &mut App) {
    app.textarea = TextArea::default();
    app.textarea.set_cursor_line_style(Style::default());
    app.textarea
        .set_placeholder_text("Type a message… (Enter to send, Shift+Enter for newline)");
}

fn input_has_text(app: &App) -> bool {
    app.textarea.lines().iter().any(|line| !line.is_empty())
}

fn dispatch_approved_tool(app: &App, tool: ToolCall, tx: &mpsc::UnboundedSender<AppEvent>) {
    tracing::info!(
        target: "jfc::ui::approval",
        tool_kind = tool.kind.label(),
        tool_id = %tool.id,
        queue_remaining = app.approval_queue.len(),
        "approved → dispatch"
    );
    stream::dispatch_tools_batched(
        vec![tool],
        tx,
        Arc::clone(&app.dedup_cache),
        Some(Arc::clone(&app.task_store)),
    );
}

/// Promote the next queued tool into `pending_approval` so the modal cycles
/// through every tool the model emitted in this turn. Auto-applies prior
/// `always_approved` / `session_approved` decisions so the user doesn't get
/// re-prompted for tool kinds they already greenlit, and **dispatches
/// auto-approved tools immediately** via `dispatch_tools_batched`.
///
/// The earlier version pushed auto-approved tools onto `pending_tool_calls`
/// thinking the StreamDone handler would flush them — but `StreamDone(ToolUse)`
/// has already fired by the time the user is approving, so anything dropped
/// into `pending_tool_calls` here would sit there forever. The user's
/// "Yes for session" / "Always" picks were the trigger: choosing those would
/// auto-pass the remaining 7 tools, none would execute, and the conversation
/// would stall with no error log.
fn advance_approval_queue(app: &mut App, tx: &mpsc::UnboundedSender<AppEvent>) {
    let mut auto_approved: Vec<ToolCall> = Vec::new();
    while let Some(next) = app.approval_queue.pop_front() {
        if !app.tool_needs_approval(&next) {
            // Already covered by an earlier "always" / "session" decision.
            // The tool is already in `messages` from the StreamTool handler;
            // dispatch it now alongside any other auto-approvable siblings.
            tracing::info!(
                target: "jfc::ui::approval",
                tool_kind = next.kind.label(),
                tool_id = %next.id,
                queue_remaining = app.approval_queue.len(),
                "auto-approved → dispatch"
            );
            auto_approved.push(next);
            continue;
        }
        app.pending_approval = Some(crate::app::PendingApproval {
            tool: next,
            selected: 0,
        });
        break;
    }
    if !auto_approved.is_empty() {
        stream::dispatch_tools_batched(
            auto_approved,
            tx,
            Arc::clone(&app.dedup_cache),
            Some(Arc::clone(&app.task_store)),
        );
    }
}

/// Mark a previously-displayed (already in `messages`) tool as denied. We
/// look up the existing entry by `id` and mutate its status/output in place,
/// rather than appending a duplicate. The agentic loop's
/// `should_continue_loop` then sees a Failed entry and continues normally.
fn deny_tool(app: &mut App, tool: ToolCall) {
    if let Some(idx) = app.streaming_assistant_idx {
        if let Some(msg) = app.messages.get_mut(idx) {
            for part in &mut msg.parts {
                if let MessagePart::Tool(tc) = part {
                    if tc.id == tool.id {
                        tc.status = ToolStatus::Failed;
                        tc.output = ToolOutput::Text("Denied by user".into());
                        return;
                    }
                }
            }
        }
    }
}

pub async fn handle_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::UnboundedSender<crate::app::AppEvent>,
) -> anyhow::Result<bool> {
    if let Some(ref mut approval) = app.pending_approval {
        match key.code {
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let tool = app.pending_approval.take().unwrap().tool;
                insert_tool_into_message(app, &tool);
                dispatch_approved_tool(app, tool, tx);
                advance_approval_queue(app, tx);
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                let tool = app.pending_approval.take().unwrap().tool;
                deny_tool(app, tool);
                advance_approval_queue(app, tx);
            }
            KeyCode::Char('a') | KeyCode::Char('A') => {
                let name = approval.tool.kind.label().to_owned();
                app.always_approved.push(name);
                let tool = app.pending_approval.take().unwrap().tool;
                insert_tool_into_message(app, &tool);
                dispatch_approved_tool(app, tool, tx);
                advance_approval_queue(app, tx);
            }
            KeyCode::Char('s') | KeyCode::Char('S') => {
                let name = approval.tool.kind.label().to_owned();
                app.session_approved.push(name);
                let tool = app.pending_approval.take().unwrap().tool;
                insert_tool_into_message(app, &tool);
                dispatch_approved_tool(app, tool, tx);
                advance_approval_queue(app, tx);
            }
            KeyCode::Up if approval.selected > 0 => {
                approval.selected -= 1;
            }
            KeyCode::Down => {
                approval.selected = (approval.selected + 1).min(ApprovalChoice::ALL.len() - 1);
            }
            KeyCode::Enter => {
                let choice = ApprovalChoice::ALL[approval.selected];
                let tool = app.pending_approval.take().unwrap().tool;
                match choice {
                    ApprovalChoice::Yes | ApprovalChoice::YesSession => {
                        if choice == ApprovalChoice::YesSession {
                            let name = tool.kind.label().to_owned();
                            app.session_approved.push(name);
                        }
                        insert_tool_into_message(app, &tool);
                        dispatch_approved_tool(app, tool, tx);
                    }
                    ApprovalChoice::Always => {
                        let name = tool.kind.label().to_owned();
                        app.always_approved.push(name);
                        insert_tool_into_message(app, &tool);
                        dispatch_approved_tool(app, tool, tx);
                    }
                    ApprovalChoice::No => {
                        deny_tool(app, tool);
                    }
                }
                advance_approval_queue(app, tx);
            }
            KeyCode::Esc => {
                // Esc cancels the entire batch — drop the queue too. Otherwise
                // a queued tool would surface immediately and the user would
                // have to dismiss them one-by-one.
                app.pending_approval = None;
                app.approval_queue.clear();
            }
            _ => {}
        }
        return Ok(false);
    }

    if app.show_task_panel {
        let total = app.task_store.list(false).len();
        match key.code {
            KeyCode::Esc => {
                app.show_task_panel = false;
            }
            KeyCode::Up if app.task_panel_selected > 0 => {
                app.task_panel_selected -= 1;
                app.task_panel_state.select(Some(app.task_panel_selected));
            }
            KeyCode::Down => {
                let max = total.saturating_sub(1);
                if app.task_panel_selected < max {
                    app.task_panel_selected += 1;
                    app.task_panel_state.select(Some(app.task_panel_selected));
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    if app.show_sidebar
        && matches!(
            (key.modifiers, key.code),
            (KeyModifiers::NONE, KeyCode::Up)
                | (KeyModifiers::NONE, KeyCode::Down)
                | (KeyModifiers::NONE, KeyCode::Enter)
        )
    {
        let total = app.session_ids.len();
        match key.code {
            KeyCode::Up if app.session_selected > 0 => {
                app.session_selected -= 1;
                app.session_list_state.select(Some(app.session_selected));
            }
            KeyCode::Down => {
                let max = total.saturating_sub(1);
                if app.session_selected < max {
                    app.session_selected += 1;
                    app.session_list_state.select(Some(app.session_selected));
                }
            }
            KeyCode::Enter => {
                if let Some(id) = app.session_ids.get(app.session_selected).cloned() {
                    if let Some(messages) = crate::session::load_session(&id) {
                        app.messages = messages;
                        app.current_session_id = Some(id);
                        app.streaming_text.clear();
                        app.streaming_reasoning.clear();
                        app.streaming_assistant_idx = None;
                        app.scroll_to_bottom();
                    }
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    if app.show_palette {
        match key.code {
            KeyCode::Esc => {
                app.show_palette = false;
                app.palette_input.clear();
                app.palette_selected = 0;
            }
            KeyCode::Enter => {
                let items = palette_items(app);
                if let Some(label) = items.get(app.palette_selected) {
                    let label = label.to_string();
                    app.show_palette = false;
                    app.palette_input.clear();
                    app.palette_selected = 0;
                    execute_palette_action(app, &label);
                }
            }
            KeyCode::Up if app.palette_selected > 0 => {
                app.palette_selected -= 1;
            }
            KeyCode::Down => {
                let max = palette_items(app).len().saturating_sub(1);
                if app.palette_selected < max {
                    app.palette_selected += 1;
                }
            }
            KeyCode::Char(c) => {
                app.palette_input.push(c);
                app.palette_selected = 0;
            }
            KeyCode::Backspace => {
                app.palette_input.pop();
                app.palette_selected = 0;
            }
            _ => {}
        }
        return Ok(false);
    }

    if app.show_model_picker {
        let total = filtered_models(app).len();
        match key.code {
            KeyCode::Esc => {
                app.show_model_picker = false;
                app.model_picker_filter.clear();
                app.model_picker_selected = 0;
                app.model_picker_state.select(Some(0));
            }
            KeyCode::Enter => {
                let filtered = filtered_models(app);
                if let Some(model) = filtered.get(app.model_picker_selected) {
                    let chosen_id = model.id.clone();
                    let chosen_provider_name = model.provider.clone();
                    if let Some(p) = app
                        .providers
                        .iter()
                        .find(|p| p.name() == chosen_provider_name)
                    {
                        app.provider = Arc::clone(p);
                    }
                    app.model = chosen_id;
                    app.show_model_picker = false;
                    app.model_picker_filter.clear();
                    app.model_picker_selected = 0;
                    app.model_picker_state.select(Some(0));
                }
            }
            KeyCode::Up if app.model_picker_selected > 0 => {
                app.model_picker_selected -= 1;
                app.model_picker_state
                    .select(Some(app.model_picker_selected));
            }
            KeyCode::Down => {
                let max = total.saturating_sub(1);
                if app.model_picker_selected < max {
                    app.model_picker_selected += 1;
                    app.model_picker_state
                        .select(Some(app.model_picker_selected));
                }
            }
            KeyCode::Home => {
                app.model_picker_selected = 0;
                app.model_picker_state.select(Some(0));
            }
            KeyCode::End => {
                let max = total.saturating_sub(1);
                app.model_picker_selected = max;
                app.model_picker_state.select(Some(max));
            }
            KeyCode::PageUp => {
                app.model_picker_selected = app.model_picker_selected.saturating_sub(10);
                app.model_picker_state
                    .select(Some(app.model_picker_selected));
            }
            KeyCode::PageDown => {
                let max = total.saturating_sub(1);
                app.model_picker_selected = (app.model_picker_selected + 10).min(max);
                app.model_picker_state
                    .select(Some(app.model_picker_selected));
            }
            KeyCode::Char(c) => {
                app.model_picker_filter.push(c);
                app.model_picker_selected = 0;
                app.model_picker_state.select(Some(0));
            }
            KeyCode::Backspace => {
                app.model_picker_filter.pop();
                app.model_picker_selected = 0;
                app.model_picker_state.select(Some(0));
            }
            _ => {}
        }
        return Ok(false);
    }

    // Up-arrow recall: when the textarea is empty and prompts are queued,
    // pressing Up pops the most recent queued prompt back into the textarea
    // for editing. Mirrors v126's "Press up to edit queued messages". Also
    // removes the corresponding ⏳/⚙ placeholder from the transcript so the
    // user sees the action took effect — they can re-edit and re-submit.
    if key.code == KeyCode::Up
        && key.modifiers == KeyModifiers::NONE
        && !app.queued_prompts.is_empty()
        && app.textarea.lines().iter().all(|l| l.is_empty())
    {
        if let Some(qp) = app.queued_prompts.pop_back() {
            let glyph = if qp.is_meta { "⚙" } else { "⏳" };
            let placeholder = format!("{glyph} {}", qp.text);
            // Remove the matching placeholder user message (last occurrence).
            for i in (0..app.messages.len()).rev() {
                if app.messages[i].role == Role::User
                    && app.messages[i]
                        .parts
                        .iter()
                        .any(|p| matches!(p, MessagePart::Text(t) if t == &placeholder))
                {
                    app.messages.remove(i);
                    break;
                }
            }
            // Recall into the textarea.
            for line in qp.text.split('\n') {
                app.textarea.insert_str(line);
                app.textarea.insert_newline();
            }
            // Drop the trailing newline added by the loop's last iteration.
            // tui-textarea's `delete_line_by_end` after a final newline
            // removes the empty trailing line cleanly.
            app.textarea.delete_line_by_end();
            tracing::info!(
                target: "jfc::ui::queue",
                remaining = app.queued_prompts.len(),
                "recall_queued_prompt"
            );
            return Ok(false);
        }
    }

    // Ctrl+Y yanks the last assistant message text to the system clipboard
    // (vim/Emacs convention: y for "yank"). We use `arboard` so the copy
    // works on Linux/macOS/Windows + Wayland. If the clipboard backend
    // isn't available (e.g. headless container), the copy silently no-ops
    // and a tracing warn fires so the user can see why nothing happened.
    if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('y') {
        let last_text: Option<String> = app
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
            .filter(|s| !s.is_empty());

        if let Some(text) = last_text {
            match arboard::Clipboard::new() {
                Ok(mut cb) => {
                    if let Err(e) = cb.set_text(text.clone()) {
                        tracing::warn!(
                            target: "jfc::ui::yank",
                            error = %e,
                            "clipboard set_text failed"
                        );
                    } else {
                        tracing::info!(
                            target: "jfc::ui::yank",
                            len = text.len(),
                            "yanked last assistant message"
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
        return Ok(false);
    }

    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => return Ok(true),
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.show_palette = true;
            app.palette_input.clear();
            app.palette_selected = 0;
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('m')) => {
            app.show_model_picker = true;
            app.model_picker_filter.clear();
            app.model_picker_selected = 0;
            app.model_picker_state.select(Some(0));
            app.model_picker_models = collect_all_models(app);
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
            app.show_sidebar = !app.show_sidebar;
            if app.show_sidebar {
                app.session_ids = crate::session::list_sessions();
                app.session_selected = 0;
                app.session_list_state.select(Some(0));
            }
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            if let Some(idx) = app.streaming_assistant_idx {
                let entry = app.reasoning_expanded.entry(idx).or_insert(false);
                *entry = !*entry;
            } else if !app.messages.is_empty() {
                let last_idx = app.messages.len() - 1;
                let entry = app.reasoning_expanded.entry(last_idx).or_insert(false);
                *entry = !*entry;
            }
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Esc) => {
            reset_input(app);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::PageUp) => {
            if input_has_text(app) {
                app.textarea.move_cursor(CursorMove::Top);
                app.textarea.move_cursor(CursorMove::Head);
            } else {
                app.scroll_page_up();
            }
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::PageDown) => {
            if input_has_text(app) {
                app.textarea.move_cursor(CursorMove::Bottom);
                app.textarea.move_cursor(CursorMove::End);
            } else {
                app.scroll_page_down();
            }
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Home) => {
            app.scroll_to_top();
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::End) => {
            app.scroll_to_bottom();
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Home) => {
            app.textarea.move_cursor(CursorMove::Head);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::End) => {
            app.textarea.move_cursor(CursorMove::End);
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            app.scroll_page_up();
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            app.scroll_page_down();
            return Ok(false);
        }
        // Ctrl+B is sidebar toggle (defined above). Ctrl+F is full-page-down.
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            let full = app.viewport_height.max(1);
            app.scroll_down(full);
            return Ok(false);
        }
        _ => {}
    }

    if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
        let text = app.textarea.lines().join("\n");
        let text = text.trim().to_string();
        if !text.is_empty() {
            reset_input(app);
            // v126 input queueing: when the model is mid-stream OR the
            // approval pipeline is non-empty, queue the prompt instead of
            // blocking on it. The approval gate matters: from the v126 log
            // we hit a 400 ("tool_use ids without tool_result") when the
            // user submitted a new turn while tools from the previous turn
            // were still Pending — the agentic loop's next request
            // serialized the orphan tool_use blocks. Queueing keeps the
            // conversation contract intact.
            //
            // The prompt renders in the transcript right away so the user
            // knows it landed, and drains via `drain_queued_prompts` in
            // main.rs once the approval pipeline empties.
            let pipeline_busy = app.pending_approval.is_some()
                || !app.approval_queue.is_empty()
                || !app.pending_tool_calls.is_empty();
            if app.is_streaming || pipeline_busy {
                let is_meta = text.starts_with('/');
                let glyph = if is_meta { "⚙" } else { "⏳" };
                tracing::info!(
                    target: "jfc::ui::queue",
                    depth = app.queued_prompts.len() + 1,
                    is_meta,
                    "queued_prompt"
                );
                app.queued_prompts.push_back(crate::app::QueuedPrompt {
                    text: text.clone(),
                    is_meta,
                });
                app.messages
                    .push(ChatMessage::user(format!("{glyph} {text}")));
                app.scroll_to_bottom();
            } else {
                handle_submit(app, text, tx).await?;
            }
        }
        return Ok(false);
    }

    app.textarea.input(key);
    Ok(false)
}

async fn handle_submit(
    app: &mut App,
    text: String,
    tx: &mpsc::UnboundedSender<crate::app::AppEvent>,
) -> anyhow::Result<()> {
    if text.starts_with('/') {
        handle_slash_command(app, &text);
        return Ok(());
    }

    let assistant_idx = app.messages.len() + 1;
    app.messages.push(ChatMessage::user(text.clone()));
    app.tool_ctx.total_user_turns += 1;
    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    app.scroll_to_bottom();

    // Auto-persist the session so the sidebar shows it. Reuses the existing
    // session id if one was loaded; otherwise mints a fresh one keyed on the
    // current timestamp.
    let session_id = app
        .current_session_id
        .clone()
        .unwrap_or_else(crate::session::generate_session_id);
    crate::session::save_session(&session_id, &app.messages);
    app.current_session_id = Some(session_id);

    let provider = app.provider.clone();
    let messages = crate::stream::build_provider_messages(&app.messages[..assistant_idx]);
    let model = app.model.clone();
    let tx = tx.clone();

    tokio::spawn(async move {
        crate::stream::stream_response(provider, messages, model, tx).await;
    });

    Ok(())
}

/// Public entry point used by `main::drain_queued_prompts` when an isMeta
/// queued prompt fires. Same body as the private slash dispatcher used in
/// `handle_submit`.
pub fn run_slash_command(app: &mut App, text: &str) {
    handle_slash_command(app, text)
}

fn handle_slash_command(app: &mut App, text: &str) {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    match parts[0] {
        "/clear" => {
            app.messages.clear();
            app.streaming_text.clear();
            app.streaming_reasoning.clear();
            app.streaming_assistant_idx = None;
        }
        "/help" => {
            app.messages.push(ChatMessage::user("/help".into()));
            app.messages.push(ChatMessage::assistant(
                "**Available commands:**\n\
                 - `/clear` — Clear conversation\n\
                 - `/auto-mode on` — Enable v126-style LLM tool classifier (no user prompts)\n\
                 - `/auto-mode off` — Disable auto-mode, restore manual approval\n\
                 - `/auto-mode status` — Show current state + rule sources\n\
                 - `/skills` — List available skills (.claude/skills/*.md)\n\
                 - `/agents` — List available agent definitions (.claude/agents/*.md)\n\
                 - `/claude-md` — Show which CLAUDE.md layers are loaded\n\
                 - `/tasks` — List todo/task items\n\
                 - `/task-add <subject>` — Create a new task\n\
                 - `/task-done <id>` — Mark task completed\n\
                 - `/task-rm <id>` — Delete task\n\
                 - `/help` — Show this message\n\
                 \n\
                 **Keys:**\n\
                 - Ctrl+B — Toggle sessions sidebar\n\
                 - Ctrl+M — Model picker\n\
                 - Ctrl+P — Command palette\n\
                 - Ctrl+O — Toggle reasoning expand\n\
                 - Ctrl+Y — Yank last assistant message to clipboard\n\
                 - Up — Recall most recent queued prompt (when textarea empty)"
                    .into(),
            ));
        }
        "/skills" => {
            let skills =
                crate::agents::load_skills(&std::env::current_dir().unwrap_or_else(|_| ".".into()));
            let body = if skills.is_empty() {
                "No skills found. Create `.claude/skills/<name>.md` files with \
                 optional YAML frontmatter (`name:`, `description:`) and a markdown \
                 body that becomes the system-prompt fragment."
                    .to_owned()
            } else {
                let mut s = format!("**{} skill(s) loaded:**\n\n", skills.len());
                for sk in &skills {
                    s.push_str(&format!(
                        "- **{}** — {}\n  source: `{}`\n",
                        sk.name,
                        sk.description.as_deref().unwrap_or("(no description)"),
                        sk.source.display()
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/skills".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/agents" => {
            let agents =
                crate::agents::load_agents(&std::env::current_dir().unwrap_or_else(|_| ".".into()));
            let body = if agents.is_empty() {
                "No agent definitions found. Create `.claude/agents/<name>.md` files \
                 with YAML frontmatter (`name:` required, plus optional `model`, \
                 `permissionMode`, `allowedTools`, `disallowedTools`, `skills`, \
                 `isolation`, `forksParentContext`) and a markdown body that becomes \
                 the system prompt for spawned subagents/teammates."
                    .to_owned()
            } else {
                let mut s = format!("**{} agent(s) loaded:**\n\n", agents.len());
                for a in &agents {
                    s.push_str(&format!(
                        "- **{}** — model: {}, permission: {:?}, isolation: {}\n  \
                         tools: allowed={:?}, denied={:?}\n  source: `{}`\n",
                        a.name,
                        a.model.as_deref().unwrap_or("inherit"),
                        a.permission_mode.unwrap_or_default(),
                        a.isolation.as_deref().unwrap_or("none"),
                        a.allowed_tools,
                        a.disallowed_tools,
                        a.source.display(),
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/agents".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/task-list" | "/tasks" => {
            let tasks = app.task_store.list(false);
            let body = if tasks.is_empty() {
                "No tasks. Use `/task-add <subject>` to create one.".to_owned()
            } else {
                let mut s = format!("**{} task(s):**\n\n", tasks.len());
                for t in &tasks {
                    let icon = match t.status {
                        crate::tasks::TaskStatus::Pending => "□",
                        crate::tasks::TaskStatus::InProgress => "▣",
                        crate::tasks::TaskStatus::Completed => "✓",
                        crate::tasks::TaskStatus::Deleted => "✗",
                    };
                    let owner = t
                        .owner
                        .as_deref()
                        .map(|o| format!(" (@{o})"))
                        .unwrap_or_default();
                    let blocks = if t.blocked_by.is_empty() {
                        String::new()
                    } else {
                        format!(" · blocked by {}", t.blocked_by.join(","))
                    };
                    s.push_str(&format!(
                        "{} `{}` {}{}{}\n",
                        icon, t.id, t.subject, owner, blocks
                    ));
                }
                let c = app.task_store.counts();
                s.push_str(&format!(
                    "\n*{} pending, {} in progress, {} completed*",
                    c.pending, c.in_progress, c.completed
                ));
                s
            };
            app.messages.push(ChatMessage::user("/tasks".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/task-add" => {
            let subject = parts.get(1).copied().unwrap_or("").trim();
            if subject.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/task-add <subject>`".into(),
                ));
            } else {
                match app
                    .task_store
                    .create(subject.to_owned(), String::new(), None, vec![])
                {
                    Ok(t) => {
                        app.messages
                            .push(ChatMessage::user(format!("/task-add {subject}")));
                        app.messages.push(ChatMessage::assistant(format!(
                            "Created task `{}`: {}",
                            t.id, t.subject
                        )));
                    }
                    Err(e) => {
                        app.messages
                            .push(ChatMessage::assistant(format!("**Error:** {e}")));
                    }
                }
            }
        }
        "/task-done" => {
            let id = parts.get(1).copied().unwrap_or("").trim();
            if id.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/task-done <id>` (e.g. `/task-done t3`)".into(),
                ));
            } else {
                match app.task_store.update(
                    id,
                    crate::tasks::TaskPatch {
                        status: Some(crate::tasks::TaskStatus::Completed),
                        ..Default::default()
                    },
                ) {
                    Ok(t) => {
                        app.messages
                            .push(ChatMessage::user(format!("/task-done {id}")));
                        app.messages.push(ChatMessage::assistant(format!(
                            "✓ Completed `{}`: {}",
                            t.id, t.subject
                        )));
                    }
                    Err(e) => {
                        app.messages
                            .push(ChatMessage::assistant(format!("**Error:** {e}")));
                    }
                }
            }
        }
        "/task-rm" | "/task-delete" => {
            let id = parts.get(1).copied().unwrap_or("").trim();
            if id.is_empty() {
                app.messages
                    .push(ChatMessage::assistant("Usage: `/task-rm <id>`".into()));
            } else {
                match app.task_store.delete(id) {
                    Ok(()) => {
                        app.messages
                            .push(ChatMessage::user(format!("/task-rm {id}")));
                        app.messages
                            .push(ChatMessage::assistant(format!("Deleted task `{id}`.")));
                    }
                    Err(e) => {
                        app.messages
                            .push(ChatMessage::assistant(format!("**Error:** {e}")));
                    }
                }
            }
        }
        "/claude-md" => {
            let h = crate::context::ClaudeMdHierarchy::load(
                &std::env::current_dir().unwrap_or_else(|_| ".".into()),
            );
            let body = if !h.any() {
                "No CLAUDE.md files found in any of the v126 hierarchy locations \
                 (`~/.config/claude/CLAUDE.md`, `~/.claude/CLAUDE.md`, \
                 `<project>/CLAUDE.md`, `<project>/.claude/CLAUDE.md`, \
                 `<project>/CLAUDE.local.md`)."
                    .to_owned()
            } else {
                let mut s = String::from("**CLAUDE.md layers loaded** (in precedence order):\n\n");
                for (label, layer) in [
                    ("Managed policy", &h.managed),
                    ("User preferences", &h.user),
                    ("Project instructions", &h.project),
                    ("Project (.claude)", &h.project_dot),
                    ("Local overrides", &h.local),
                ] {
                    if let Some((path, content)) = layer {
                        s.push_str(&format!(
                            "- **{}** ({}) — {} bytes\n",
                            label,
                            path.display(),
                            content.len()
                        ));
                    }
                }
                s
            };
            app.messages.push(ChatMessage::user("/claude-md".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/auto-mode" => {
            let arg = parts.get(1).copied().unwrap_or("status").trim();
            match arg {
                "on" | "enable" | "true" => {
                    app.auto_mode.enabled = true;
                    app.messages.push(ChatMessage::assistant(
                        "**Auto-mode enabled.** Every tool call will be sent to the v126 \
                         classifier LLM. The classifier may block dangerous operations \
                         without prompting you. Edit `~/.config/jfc/settings.json` under \
                         `autoMode.{allow,soft_deny,environment}` (with `$defaults` \
                         inheritance) to extend the rules."
                            .into(),
                    ));
                }
                "off" | "disable" | "false" => {
                    app.auto_mode.enabled = false;
                    app.messages.push(ChatMessage::assistant(
                        "**Auto-mode disabled.** Tool calls will use the manual approval \
                         flow again."
                            .into(),
                    ));
                }
                _ => {
                    let n_allow = app.auto_mode.allow.len();
                    let n_block = app.auto_mode.soft_deny.len();
                    let n_env = app.auto_mode.environment.len();
                    let state = if app.auto_mode.enabled { "ON" } else { "OFF" };
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Auto-mode: {state}**\n\
                         \n\
                         Custom rule counts (settings.json):\n\
                         - allow: {n_allow}\n\
                         - soft_deny: {n_block}\n\
                         - environment: {n_env}\n\
                         \n\
                         Use `/auto-mode on` or `/auto-mode off` to toggle."
                    )));
                }
            }
        }
        _ => {
            app.messages.push(ChatMessage::assistant(format!(
                "Unknown command: `{}`. Type `/help` for available commands.",
                parts[0]
            )));
        }
    }
    app.scroll_to_bottom();
}

fn execute_palette_action(app: &mut App, label: &str) {
    match label {
        "Clear Messages" => {
            app.messages.clear();
            app.streaming_text.clear();
            app.streaming_reasoning.clear();
            app.streaming_assistant_idx = None;
        }
        _ => {}
    }
}

pub fn palette_items(app: &App) -> Vec<&'static str> {
    let all: &[&str] = &["Clear Messages"];
    if app.palette_input.is_empty() {
        all.to_vec()
    } else {
        all.iter()
            .filter(|s| s.to_lowercase().contains(&app.palette_input.to_lowercase()))
            .copied()
            .collect()
    }
}

/// Union of every configured provider's models, in provider-registration order.
/// For each provider, prefer the cached `fetch_models()` result (live data — for
/// OpenWebUI this is the configured instance's actual model list); fall back to
/// the static `available_models()` only when the cache is missing. After the
/// union, apply the OAuth seat-tier filter (v126's `XwH()` equivalent) so the
/// picker hides Opus variants the account can't use.
pub fn collect_all_models(app: &App) -> Vec<crate::provider::ModelInfo> {
    let merged: Vec<_> = app
        .providers
        .iter()
        .flat_map(|p| {
            app.provider_models
                .get(p.name())
                .cloned()
                .unwrap_or_else(|| p.available_models())
        })
        .collect();
    crate::providers::anthropic_models::apply_seat_tier_filter(merged, app.seat_tier.as_deref())
}

pub fn filtered_models(app: &App) -> Vec<crate::provider::ModelInfo> {
    if app.model_picker_filter.is_empty() {
        app.model_picker_models.clone()
    } else {
        let q = app.model_picker_filter.to_lowercase();
        app.model_picker_models
            .iter()
            .filter(|m| {
                m.display_name.to_lowercase().contains(&q) || m.id.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }
}
