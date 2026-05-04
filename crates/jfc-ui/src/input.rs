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

fn input_line_char_len(app: &App, line: usize) -> usize {
    app.textarea
        .lines()
        .get(line)
        .map(|line| line.chars().count())
        .unwrap_or_default()
}

fn cursor_index(value: usize) -> u16 {
    value.min(u16::MAX as usize) as u16
}

fn move_input_cursor_visual_up(app: &mut App) {
    let width = app.input_wrap_width.max(1);
    let (line, col) = app.textarea.cursor();

    if col >= width {
        app.textarea.move_cursor(CursorMove::Jump(
            cursor_index(line),
            cursor_index(col - width),
        ));
        return;
    }

    if line == 0 {
        app.textarea.move_cursor(CursorMove::Head);
        return;
    }

    let prev_len = input_line_char_len(app, line - 1);
    let prev_visual_start = (prev_len / width) * width;
    let target_col = prev_len.min(prev_visual_start + col);
    app.textarea.move_cursor(CursorMove::Jump(
        cursor_index(line - 1),
        cursor_index(target_col),
    ));
}

fn move_input_cursor_visual_down(app: &mut App) {
    let width = app.input_wrap_width.max(1);
    let (line, col) = app.textarea.cursor();
    let line_len = input_line_char_len(app, line);

    if col + width <= line_len {
        app.textarea.move_cursor(CursorMove::Jump(
            cursor_index(line),
            cursor_index(col + width),
        ));
        return;
    }

    let next_line = line + 1;
    if next_line >= app.textarea.lines().len() {
        app.textarea.move_cursor(CursorMove::End);
        return;
    }

    let target_col = input_line_char_len(app, next_line).min(col % width);
    app.textarea.move_cursor(CursorMove::Jump(
        cursor_index(next_line),
        cursor_index(target_col),
    ));
}

fn input_has_text(app: &App) -> bool {
    app.textarea.lines().iter().any(|line| !line.is_empty())
}

/// Collect every user-message prompt text in chronological order.
/// Used by the up-arrow history recall to walk backwards through what
/// the user has typed this session. Excludes empty messages and tool
/// outputs — only the actual prompts the user submitted.
fn user_prompts(app: &App) -> Vec<String> {
    app.messages
        .iter()
        .filter(|m| m.role == Role::User)
        .filter_map(|m| {
            let text: String = m
                .parts
                .iter()
                .filter_map(|p| match p {
                    MessagePart::Text(s) if !s.is_empty() => Some(s.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        })
        .collect()
}

/// Bump `history_cursor` to the next-older prompt and return its text.
/// Returns `None` when there's no history left to recall (the cursor
/// has reached the oldest prompt or there are no user messages).
pub fn recall_previous_prompt(app: &mut App) -> Option<String> {
    let prompts = user_prompts(app);
    if prompts.is_empty() {
        return None;
    }
    let next = match app.history_cursor {
        None => prompts.len() - 1,
        Some(0) => return None, // already at the oldest
        Some(n) => n - 1,
    };
    app.history_cursor = Some(next);
    prompts.get(next).cloned()
}

/// Bump `history_cursor` toward the most-recent prompt and return its
/// text. Returns `None` when the cursor would advance past the most
/// recent — caller is expected to clear the input in that case.
pub fn recall_next_prompt(app: &mut App) -> Option<String> {
    let prompts = user_prompts(app);
    let cur = app.history_cursor?;
    if cur + 1 >= prompts.len() {
        app.history_cursor = None;
        return None;
    }
    let next = cur + 1;
    app.history_cursor = Some(next);
    prompts.get(next).cloned()
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
        Arc::clone(&app.provider),
        app.model.clone(),
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
            Arc::clone(&app.provider),
            app.model.clone(),
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
        let total = app
            .task_store
            .list(crate::tasks::DeletedFilter::Exclude)
            .len();
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
                        app.switch_session(Some(id));
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
                        .find(|p| chosen_provider_name == p.name())
                    {
                        app.provider = Arc::clone(p);
                    }
                    app.model = chosen_id;
                    app.sync_selected_context_window();
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

    if app.leader_key_active {
        if let Some(t) = app.leader_key_timeout {
            if t.elapsed() >= std::time::Duration::from_secs(2) {
                app.leader_key_active = false;
                app.leader_key_timeout = None;
            }
        }
    }

    if app.leader_key_active {
        app.leader_key_active = false;
        app.leader_key_timeout = None;

        let task_ids: Vec<String> = app.background_tasks.keys().cloned().collect();
        let task_count = task_ids.len();

        match key.code {
            KeyCode::Esc => {}
            KeyCode::Down | KeyCode::Char('j') => {
                if task_count > 0 {
                    let current_pos = app
                        .viewing_task_id
                        .as_ref()
                        .and_then(|id| task_ids.iter().position(|t| t == id));
                    let next = match current_pos {
                        None => 0,
                        Some(i) => (i + 1).min(task_count - 1),
                    };
                    app.viewing_task_id = task_ids.into_iter().nth(next);
                    app.scroll_to_bottom();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.viewing_task_id = None;
                app.scroll_to_bottom();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(ref id) = app.viewing_task_id.clone() {
                    let pos = task_ids.iter().position(|t| t == id).unwrap_or(0);
                    if pos > 0 {
                        app.viewing_task_id = task_ids.into_iter().nth(pos - 1);
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some(ref id) = app.viewing_task_id.clone() {
                    let pos = task_ids.iter().position(|t| t == id).unwrap_or(0);
                    if pos + 1 < task_count {
                        app.viewing_task_id = task_ids.into_iter().nth(pos + 1);
                    }
                }
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
        (KeyModifiers::NONE, KeyCode::Up) => {
            // Up at empty input → recall previous user prompt. Multiple
            // presses cycle backwards through history. Mirrors v126's
            // `useArrowKeyHistory` (cli.js) — quality-of-life win for
            // resending or editing recent submissions.
            if !input_has_text(app) {
                if let Some(prompt) = recall_previous_prompt(app) {
                    app.textarea = TextArea::from(
                        prompt.lines().map(str::to_string).collect::<Vec<_>>(),
                    );
                    app.textarea.set_cursor_line_style(Style::default());
                    app.textarea.set_placeholder_text(
                        "Type a message… (Enter to send, Shift+Enter for newline)",
                    );
                    app.textarea.move_cursor(CursorMove::End);
                    return Ok(false);
                }
            }
            move_input_cursor_visual_up(app);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            // Symmetric to Up — cycle forward through history when the
            // user has recalled a past prompt. When `history_cursor` is
            // None or at the live edit, falls through to cursor move.
            if app.history_cursor.is_some() {
                if let Some(prompt) = recall_next_prompt(app) {
                    app.textarea = TextArea::from(
                        prompt.lines().map(str::to_string).collect::<Vec<_>>(),
                    );
                    app.textarea.set_cursor_line_style(Style::default());
                    app.textarea.set_placeholder_text(
                        "Type a message… (Enter to send, Shift+Enter for newline)",
                    );
                    app.textarea.move_cursor(CursorMove::End);
                    return Ok(false);
                } else {
                    // Cycled past the most recent — return to empty input.
                    app.history_cursor = None;
                    reset_input(app);
                    return Ok(false);
                }
            }
            move_input_cursor_visual_down(app);
            return Ok(false);
        }
        _ => {}
    }

    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            if input_has_text(app) {
                reset_input(app);
                return Ok(false);
            }
            return Ok(true);
        }
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
        (KeyModifiers::CONTROL, KeyCode::Char('x')) => {
            app.leader_key_active = true;
            app.leader_key_timeout = Some(std::time::Instant::now());
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('i')) => {
            app.show_info_sidebar = !app.show_info_sidebar;
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
            app.show_info_sidebar = !app.show_info_sidebar;
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('y')) => {
            // Yank the most recent assistant message's text to the clipboard.
            // v126 has the same shortcut (cli.js advertises `Ctrl+Y` for
            // assistant copy). Walks backwards through `messages` looking
            // for the first Assistant message with a non-empty Text part;
            // joins all Text parts with newlines so multi-paragraph
            // replies copy intact. Pushes a Success toast on success,
            // Error on clipboard failure.
            let text = app
                .messages
                .iter()
                .rev()
                .find(|m| m.role == Role::Assistant)
                .map(|m| {
                    m.parts
                        .iter()
                        .filter_map(|p| match p {
                            MessagePart::Text(s) if !s.is_empty() => Some(s.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n\n")
                });
            match text {
                Some(t) if !t.is_empty() => {
                    use arboard::Clipboard;
                    match Clipboard::new().and_then(|mut c| c.set_text(t.clone())) {
                        Ok(()) => {
                            let preview: String = t.chars().take(40).collect();
                            let suffix = if t.chars().count() > 40 { "…" } else { "" };
                            let _ = tx.send(crate::app::AppEvent::Toast {
                                kind: crate::toast::ToastKind::Success,
                                text: format!("Copied: {preview}{suffix}"),
                            });
                        }
                        Err(e) => {
                            let _ = tx.send(crate::app::AppEvent::Toast {
                                kind: crate::toast::ToastKind::Error,
                                text: format!("Clipboard error: {e}"),
                            });
                        }
                    }
                }
                _ => {
                    let _ = tx.send(crate::app::AppEvent::Toast {
                        kind: crate::toast::ToastKind::Warning,
                        text: "No assistant message to yank".into(),
                    });
                }
            }
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            // Ctrl+O is v126's universal "expand" key (cli.js:338038
            // advertises `(ctrl+o to expand)` on the diagnostic row).
            // Priority: when the diagnostic panel is closeable from
            // here OR diagnostics exist, that wins — toggling the
            // diagnostic-expansion panel is the primary affordance.
            // Falls back to thinking-toggle otherwise.
            if app.show_diagnostic_panel {
                app.show_diagnostic_panel = false;
            } else if !app.diagnostics.is_empty() {
                app.show_diagnostic_panel = true;
                // Opening the panel = acknowledgment. Mark every current
                // entry as "delivered" so the summary row stops surfacing
                // them. v126 cli.js:231025-231036 does the same — once a
                // diagnostic has been shown to the user, subsequent
                // refreshes don't re-pop the row for the same entry.
                for entry in &app.diagnostics {
                    app.delivered_diagnostics
                        .insert(crate::diagnostics::entry_key(entry));
                }
            } else if let Some(idx) = app.streaming_assistant_idx {
                let entry = app.reasoning_expanded.entry(idx).or_insert(false);
                *entry = !*entry;
            } else if !app.messages.is_empty() {
                let last_idx = app.messages.len() - 1;
                let entry = app.reasoning_expanded.entry(last_idx).or_insert(false);
                *entry = !*entry;
            }
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Esc) if app.show_diagnostic_panel => {
            app.show_diagnostic_panel = false;
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Char('o')) if !input_has_text(app) => {
            'toggle: {
                let messages = &mut app.messages;
                for msg in messages.iter_mut().rev() {
                    for part in msg.parts.iter_mut().rev() {
                        if let MessagePart::Tool(tc) = part {
                            if matches!(tc.output, ToolOutput::LargeText(_)) {
                                tc.is_collapsed = !tc.is_collapsed;
                                break 'toggle;
                            }
                        }
                    }
                }
            }
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Esc) => {
            if app.viewing_task_id.is_some() {
                app.viewing_task_id = None;
                return Ok(false);
            }
            reset_input(app);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::PageUp) => {
            app.scroll_page_up();
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::PageDown) => {
            app.scroll_page_down();
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
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            app.textarea.move_cursor(CursorMove::Head);
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
            app.textarea.move_cursor(CursorMove::End);
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            app.textarea.delete_line_by_head();
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            app.textarea.delete_line_by_end();
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            app.textarea.delete_word();
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            if input_has_text(app) {
                app.textarea.delete_next_char();
                return Ok(false);
            }
            return Ok(true);
        }
        (KeyModifiers::ALT, KeyCode::Char('d')) => {
            app.textarea.delete_next_word();
            return Ok(false);
        }
        (KeyModifiers::ALT, KeyCode::Char('b')) => {
            app.textarea.move_cursor(CursorMove::WordBack);
            return Ok(false);
        }
        (KeyModifiers::ALT, KeyCode::Char('f')) => {
            app.textarea.move_cursor(CursorMove::WordForward);
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

    // `@filename` autocomplete. When the popup is active, intercept the
    // keys that drive it (Esc / Enter / Tab / arrows) BEFORE letting the
    // textarea consume them. Anything else falls through; we re-derive
    // the query from the buffer after each keystroke. Mirrors v126's
    // `autocomplete:accept` / `autocomplete:dismiss` keybindings
    // (cli.js:161602).
    if app.mention.active {
        match key.code {
            KeyCode::Esc => {
                app.mention.dismiss();
                return Ok(false);
            }
            KeyCode::Enter | KeyCode::Tab => {
                if let Some(pick) = app.mention.accepted().map(str::to_owned) {
                    apply_mention_pick(app, &pick);
                }
                app.mention.dismiss();
                return Ok(false);
            }
            KeyCode::Up => {
                app.mention.move_selection(-1);
                return Ok(false);
            }
            KeyCode::Down => {
                app.mention.move_selection(1);
                return Ok(false);
            }
            _ => {}
        }
    }

    app.textarea.input(key);
    update_mention_state_after_input(app);
    Ok(false)
}

/// Replace the active `@<query>` token in the textarea with the picked
/// path + trailing space. Reconstructs the textarea from the resulting
/// string so cursor positioning is correct (the `tui_textarea` API
/// doesn't expose a "replace range" operation).
fn apply_mention_pick(app: &mut App, pick: &str) {
    let buffer = app.textarea.lines().join("\n");
    let anchor = app.mention.anchor_byte;
    let q_len = app.mention.query.chars().count();
    // `apply_acceptance` expects byte offsets but treats the query as a
    // suffix following the `@`. Build the new buffer.
    let (new_buf, _new_cursor) = crate::mentions::apply_acceptance(&buffer, anchor, q_len, pick);
    app.textarea = TextArea::from(new_buf.lines().map(str::to_string).collect::<Vec<_>>());
    app.textarea.set_cursor_line_style(Style::default());
    app.textarea
        .set_placeholder_text("Type a message… (Enter to send, Shift+Enter for newline)");
    app.textarea.move_cursor(CursorMove::End);
}

/// Decide whether the popup should activate (newly-typed `@` after
/// whitespace) or update its query (already-active, more chars typed
/// or backspace shrunk the buffer).
fn update_mention_state_after_input(app: &mut App) {
    let (line_idx, col) = app.textarea.cursor();
    let line = match app.textarea.lines().get(line_idx) {
        Some(s) => s.clone(),
        None => return,
    };
    let prefix: String = line.chars().take(col).collect();
    if app.mention.active {
        // Recompute query from anchor → cursor on the same line. If the
        // user backspaced past the `@` or moved off-line, dismiss.
        let buffer = app.textarea.lines().join("\n");
        if app.mention.anchor_byte >= buffer.len()
            || !buffer[app.mention.anchor_byte..].starts_with('@')
        {
            app.mention.dismiss();
            return;
        }
        // Query = chars after `@` up to first whitespace (so typing a
        // space terminates the popup naturally).
        let after_at = &buffer[app.mention.anchor_byte + 1..];
        let q: String = after_at
            .chars()
            .take_while(|c| !c.is_whitespace())
            .collect();
        let all = app.mention_all_files.clone();
        app.mention.update_query(q, &all);
        // Whitespace after `@token` → user typed past the trigger; close.
        let after_q_len = app.mention.anchor_byte + 1 + app.mention.query.len();
        if after_q_len < buffer.len() && buffer[after_q_len..].starts_with(char::is_whitespace) {
            app.mention.dismiss();
        }
        return;
    }
    if let Some(anchor) = crate::mentions::should_activate(&prefix) {
        // Lazy-load file list so we don't walk `cwd` on every keystroke.
        if app.mention_all_files.is_empty() {
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            app.mention_all_files = crate::mentions::scan_files(&cwd, 5000);
        }
        let all = app.mention_all_files.clone();
        let initial = crate::mentions::filter_candidates(&all, "");
        app.mention.activate(anchor, initial);
    }
}

/// Public re-entry used by `AppEvent::Submit`. Same body as the private
/// `handle_submit` used from the typing path.
pub async fn handle_submit_text(
    app: &mut App,
    text: String,
    tx: &mpsc::UnboundedSender<crate::app::AppEvent>,
) -> anyhow::Result<()> {
    handle_submit(app, text, tx).await
}

async fn handle_submit(
    app: &mut App,
    text: String,
    tx: &mpsc::UnboundedSender<crate::app::AppEvent>,
) -> anyhow::Result<()> {
    if text.starts_with('/') {
        // `/check` re-runs the cargo-check producer. Handled here (not in
        // `handle_slash_command`) because it needs the tx channel to emit
        // `DiagnosticsUpdated` from a spawned task.
        if text.trim() == "/check" {
            let tx_diag = tx.clone();
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            tokio::spawn(async move {
                crate::diagnostics_producer::run_once(cwd, tx_diag).await;
            });
        }
        handle_slash_command(app, &text);
        return Ok(());
    }

    // Pre-submit compaction gate (mirrors v126 `Du7` running before the API
    // call rather than only after tool batches). Without this, a long
    // text-only assistant reply pushes the context past 200K — by the time
    // the next user message arrives, the conversation already exceeds the
    // hard limit and the provider returns 400 prompt_too_long. v126 cli.js
    // line 382476 shows the same pre-submit check returning a "blocking_limit"
    // result before queryDirect ever fires.
    let est = crate::compact::estimate_tokens(&app.messages);
    let level = crate::compact::compact_level(est, app.max_context_tokens);
    if matches!(
        level,
        crate::compact::CompactLevel::Compact | crate::compact::CompactLevel::Blocked
    ) || app.force_compact_pending
    {
        let manual = std::mem::take(&mut app.force_compact_pending);
        tracing::info!(
            target: "jfc::compact",
            est, level = ?level, manual,
            "pre-submit compact triggered"
        );
        let messages = app.messages.clone();
        let provider = Arc::clone(&app.provider);
        let model = app.model.clone();
        let mut tool_ctx = app.tool_ctx.clone();
        let tx_pre = tx.clone();
        let user_text = text.clone();
        let _ = tx_pre.send(crate::app::AppEvent::CompactionStarted);
        tokio::spawn(async move {
            let options = crate::provider::StreamOptions::new(model);
            let result =
                crate::compact::compact(&messages, provider.as_ref(), &options, &mut tool_ctx)
                    .await;
            match result {
                crate::compact::CompactResult::Success {
                    messages,
                    pre_tokens,
                    post_tokens,
                } => {
                    let _ = tx_pre.send(crate::app::AppEvent::CompactionDone {
                        messages,
                        tool_ctx,
                        pre_tokens,
                        post_tokens,
                    });
                    // Re-queue the user's message — it didn't make it into
                    // the conversation before compaction ran.
                    let _ = tx_pre.send(crate::app::AppEvent::Submit(user_text));
                }
                crate::compact::CompactResult::CircuitBreakerTripped => {
                    let _ = tx_pre.send(crate::app::AppEvent::CompactionFailed(
                        "Circuit breaker tripped — submit again with `/compact` if needed".into(),
                    ));
                }
                crate::compact::CompactResult::Exhausted { attempts } => {
                    let _ = tx_pre.send(crate::app::AppEvent::CompactionFailed(format!(
                        "Exhausted {attempts} compaction attempts — request is too large"
                    )));
                }
                _ => {
                    // Unsupported / TooFewGroups: provider can't compact, just
                    // submit anyway and let the API return its own error.
                    let _ = tx_pre.send(crate::app::AppEvent::Submit(user_text));
                }
            }
        });
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
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.turn_started_at = Some(now);
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);
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
        "/rename" => {
            // Set a custom title on the current session. v126 cli.js:39786
            // calls this `customTitle` and it sits at the top of the title
            // precedence chain (custom → ai → firstPrompt → id-slice).
            // Persisted to the session JSON so it survives restarts.
            let new_title = parts.get(1).copied().unwrap_or("").trim().to_owned();
            app.messages.push(ChatMessage::user(format!("/rename {new_title}")));
            match (&app.current_session_id, new_title.is_empty()) {
                (None, _) => {
                    app.messages.push(ChatMessage::assistant(
                        "No active session to rename. Send a message first.".into(),
                    ));
                }
                (_, true) => {
                    app.messages.push(ChatMessage::assistant(
                        "Usage: `/rename <title>`. Pass any text to set the session title; the picker / sidebar will show it.".into(),
                    ));
                }
                (Some(id), false) => {
                    crate::session::set_session_title(id, &new_title);
                    app.messages.push(ChatMessage::assistant(format!(
                        "Session `{id}` renamed to **{new_title}**.",
                    )));
                }
            }
        }
        "/clear" => {
            app.messages.clear();
            app.streaming_text.clear();
            app.streaming_reasoning.clear();
            app.streaming_assistant_idx = None;
            // Mint a fresh session id and wipe per-session state (tasks,
            // completion timers). v126 cli.js:271511 keys todos by sessionId
            // so a new session inherently has an empty list — match that.
            app.switch_session(None);
        }
        "/check" => {
            // Re-run `cargo check --message-format=json` and refresh the
            // diagnostic row + transition toast. v126 has an analogous
            // `/diagnostics` flow; keep ours short. Best-effort — silently
            // no-ops outside a cargo project.
            app.messages.push(ChatMessage::user("/check".into()));
            app.messages.push(ChatMessage::assistant(
                "Running `cargo check`… (results will land in the diagnostic row)".into(),
            ));
            // The handler emits `AppEvent::DiagnosticsUpdated` whose
            // handler shows a transition toast — no need to render
            // results inline.
            // We don't have direct `tx` here; emit via a no-op
            // background spawn that returns through the channel exposed
            // to other slash-command paths. Instead, we set a flag the
            // main loop can pick up; for now the simpler thing is to
            // tell the user to wait for the auto-update.
            //
            // (The startup-time spawn already does this on launch; this
            // command just reminds the user how to retrigger.)
        }
        "/compact" => {
            let est = crate::compact::estimate_tokens(&app.messages);
            let level = crate::compact::compact_level(est, app.max_context_tokens);
            let pct = if app.max_context_tokens > 0 {
                (est * 100 / app.max_context_tokens).min(999)
            } else {
                0
            };
            app.messages.push(ChatMessage::user("/compact".into()));
            app.messages.push(ChatMessage::assistant(format!(
                "Manual compaction queued — current estimate **{est} / {} tokens ({pct}%)**, level: **{level:?}**.\n\n\
                 The next assistant turn will summarize the conversation up to here, replacing the prior turns with a 9-section summary.\n\n\
                 *(Tip: set `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` (1-100) to test thresholds, or `JFC_DISABLE_AUTO_COMPACT=1` to disable auto-compact entirely.)*",
                app.max_context_tokens
            )));
            app.force_compact_pending = true;
        }
        "/config" => {
            // `/config` (no args) → dump the parsed config as TOML in a code block.
            // `/config path` → print the canonical file path so the user knows
            // where to put their overrides. We re-parse on every invocation
            // (instead of caching at startup) so edits to ~/.config/jfc/config.toml
            // surface without restart — this command is the user's read-only
            // window into "what does jfc currently see?". Wiring the resolved
            // model into the actual stream call site is a separate task; for now
            // this command exists so users can verify their file parses and
            // know where to edit.
            let arg = parts.get(1).copied().unwrap_or("").trim();
            app.messages.push(ChatMessage::user(text.to_owned()));
            if arg == "path" {
                let p = crate::config::config_path();
                app.messages.push(ChatMessage::assistant(format!(
                    "**Config path:** `{}`",
                    p.display()
                )));
            } else {
                let cfg = crate::config::load();
                let body = match toml::to_string_pretty(&cfg) {
                    Ok(s) if s.trim().is_empty() => "(empty config — no overrides)".to_owned(),
                    Ok(s) => format!("```toml\n{s}```"),
                    Err(e) => format!("**Error serializing config:** {e}"),
                };
                app.messages.push(ChatMessage::assistant(body));
            }
        }
        "/continue" | "/c" => {
            // v126 / codex-rs parity: `/continue` is cwd-scoped by default.
            // `/continue all` (or `/c all`) shows the globally most recent
            // — useful when the user moved a project or wants any session.
            // The original behavior (global most-recent) caused the
            // "continue from project A accidentally resumed project B"
            // confusion the user reported.
            let want_global = parts.get(1).copied().map(str::trim) == Some("all");
            let session_id = if want_global {
                crate::session::most_recent_session()
            } else {
                let cwd_str = std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string());
                crate::session::most_recent_session_for_cwd(cwd_str.as_deref())
            };
            if let Some(session_id) = session_id {
                if let Some(messages) = crate::session::load_session(&session_id) {
                    app.messages = messages;
                    app.switch_session(Some(session_id.clone()));
                    app.streaming_text.clear();
                    app.streaming_reasoning.clear();
                    app.streaming_assistant_idx = None;
                    app.scroll_to_bottom();
                    let scope = if want_global { "any cwd" } else { "this cwd" };
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Resumed session `{session_id}`** ({scope}) — {} message(s) loaded.",
                        app.messages.len() - 1
                    )));
                } else {
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Error:** Failed to load session `{session_id}`."
                    )));
                }
            } else {
                let hint = if want_global {
                    "No previous sessions found anywhere."
                } else {
                    "No previous sessions found in this cwd. Try `/continue all` for any session."
                };
                app.messages.push(ChatMessage::assistant(hint.into()));
            }
        }
        "/resume" => {
            // Resume a specific session by id
            let session_id = parts.get(1).copied().unwrap_or("").trim();
            if session_id.is_empty() {
                // List available sessions
                let sessions = crate::session::list_sessions();
                if sessions.is_empty() {
                    app.messages.push(ChatMessage::assistant(
                        "No sessions found. Usage: `/resume <session_id>`".into(),
                    ));
                } else {
                    let list = sessions
                        .iter()
                        .take(10)
                        .map(|s| format!("  - `{s}`"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    let more = if sessions.len() > 10 {
                        format!("\n  ... and {} more", sessions.len() - 10)
                    } else {
                        String::new()
                    };
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Usage:** `/resume <session_id>`\n\n**Available sessions:**\n{list}{more}"
                    )));
                }
            } else if let Some(messages) = crate::session::load_session(session_id) {
                let msg_count = messages.len();
                app.messages = messages;
                app.switch_session(Some(session_id.to_owned()));
                app.streaming_text.clear();
                app.streaming_reasoning.clear();
                app.streaming_assistant_idx = None;
                app.scroll_to_bottom();
                app.messages.push(ChatMessage::assistant(format!(
                    "**Resumed session `{session_id}`** — {msg_count} message(s) loaded."
                )));
            } else {
                app.messages.push(ChatMessage::assistant(format!(
                    "**Error:** Session `{session_id}` not found."
                )));
            }
        }
        "/sessions" => {
            // List all sessions with metadata
            let sessions = crate::session::list_sessions_with_metadata();
            if sessions.is_empty() {
                app.messages
                    .push(ChatMessage::assistant("No sessions found.".into()));
            } else {
                let mut body = format!("**{} session(s):**\n\n", sessions.len());
                for (i, s) in sessions.iter().take(20).enumerate() {
                    let prompt = s.first_prompt.as_deref().unwrap_or("(no prompt)");
                    let prompt_display = if prompt.len() > 50 {
                        format!("{}…", &prompt[..50])
                    } else {
                        prompt.to_string()
                    };
                    let current = app.current_session_id.as_deref() == Some(&s.id);
                    let marker = if current { " ← current" } else { "" };
                    body.push_str(&format!(
                        "{}. `{}`{} — {} msg(s)\n   {}\n",
                        i + 1,
                        s.id,
                        marker,
                        s.message_count,
                        prompt_display
                    ));
                }
                if sessions.len() > 20 {
                    body.push_str(&format!(
                        "\n... and {} more (use Ctrl+B sidebar)",
                        sessions.len() - 20
                    ));
                }
                app.messages.push(ChatMessage::user("/sessions".into()));
                app.messages.push(ChatMessage::assistant(body));
            }
        }
        "/help" => {
            app.messages.push(ChatMessage::user("/help".into()));
            app.messages.push(ChatMessage::assistant(
                "**Available commands:**\n\
                 - `/clear` — Clear conversation and start fresh\n\
                 - `/compact` — Manually compact the conversation\n\
                 - `/check` — Re-run cargo-check diagnostics\n\
                 - `/config` — Show parsed `~/.config/jfc/config.toml` (use `/config path` for the file location)\n\
                 - `/continue` (or `/c`) — Resume most recent session\n\
                 - `/resume <id>` — Resume a specific session by id\n\
                 - `/sessions` — List all saved sessions\n\
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
                 - `/worktree [list|create <name>|remove <name>|switch <name>]` — Manage `.jfc-worktrees/<name>` checkouts on `jfc/<name>` branches\n\
                 - `/help` — Show this message\n\
                 \n\
                 **Keys:**\n\
                 - Ctrl+B — Toggle sessions sidebar\n\
                 - Ctrl+M — Model picker\n\
                 - Ctrl+P — Command palette\n\
                 - Ctrl+O — Expand reasoning / open diagnostic panel\n\
                 - Ctrl+T — Open task panel\n\
                 - Ctrl+Y — Yank last assistant message to clipboard\n\
                 - Ctrl+S — Toggle info sidebar\n\
                 - `@` — Autocomplete file paths from cwd\n\
                 - Up — Recall most recent queued prompt / cycle history (when input empty)\n\
                 - Esc — Dismiss popup / close diagnostic panel\n\
                 \n\
                 **Env knobs:**\n\
                 - `JFC_DISABLE_BELL=1` — silence terminal bell on tool completion\n\
                 - `JFC_DISABLE_AUTO_COMPACT=1` — disable auto-compaction\n\
                 - `JFC_DISABLE_CARGO_CHECK=1` — skip startup `cargo check`\n\
                 - `JFC_AUTOCOMPACT_PCT_OVERRIDE=N` — force compact threshold\n\
                 - `JFC_TOOL_TITLE_WIDTH=N` — cap tool title length (default 100)"
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
            let tasks = app.task_store.list(crate::tasks::DeletedFilter::Exclude);
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
                match app.task_store.create(
                    subject.to_owned(),
                    String::new(),
                    None,
                    Vec::<crate::tasks::TaskId>::new(),
                ) {
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
        "/worktree" => {
            handle_worktree_command(app, parts.get(1).copied().unwrap_or("").trim());
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

/// Dispatch the `/worktree …` subcommands. Argument string is the slice after
/// `/worktree ` — empty / `"list"` lists, `"create <name>"` creates,
/// `"remove <name>"` removes, `"switch <name>"` prints the manual cd hint.
///
/// The runtime cwd of `App` is fixed at startup (see `App::new` in app.rs), so
/// `switch` cannot teleport the running session into a different checkout —
/// it tells the user how to do it manually. Once App.cwd becomes mutable we
/// can revisit.
fn handle_worktree_command(app: &mut App, args: &str) {
    let mut it = args.split_whitespace();
    let sub = it.next().unwrap_or("");
    let arg = it.next().unwrap_or("");
    let repo_root = std::path::PathBuf::from(&app.cwd);

    let echo = |app: &mut App, raw: String, body: String| {
        app.messages.push(ChatMessage::user(raw));
        app.messages.push(ChatMessage::assistant(body));
    };

    let list_body = |app: &App| -> String {
        match crate::worktrees::list_worktrees(&std::path::PathBuf::from(&app.cwd)) {
            Ok(rows) if rows.is_empty() => "No worktrees registered.".to_owned(),
            Ok(rows) => {
                let mut s = format!("**{} worktree(s):**\n\n", rows.len());
                for w in &rows {
                    let branch = if w.branch.is_empty() {
                        "(none)"
                    } else {
                        w.branch.as_str()
                    };
                    s.push_str(&format!("- `{}` — branch `{}`\n", w.path, branch));
                }
                s
            }
            Err(e) => format!("**Error:** {e}"),
        }
    };

    match sub {
        "" | "list" => {
            let body = list_body(app);
            echo(app, "/worktree list".to_owned(), body);
        }
        "create" => {
            if arg.is_empty() {
                echo(
                    app,
                    "/worktree create".to_owned(),
                    "Usage: `/worktree create <name>` (alphanumeric, dash, underscore)".to_owned(),
                );
                return;
            }
            if let Err(e) = crate::worktrees::validate_name(arg) {
                echo(
                    app,
                    format!("/worktree create {arg}"),
                    format!("**Error:** {e}"),
                );
                return;
            }
            let body = match crate::worktrees::create_worktree(&repo_root, arg) {
                Ok(w) => format!(
                    "Created worktree `{}` on branch `{}`.\n\n\
                     Switch into it with:\n```\ncd {}\n```\nthen re-run `jfc`.",
                    w.path, w.branch, w.path
                ),
                Err(e) => format!("**Error:** {e}"),
            };
            echo(app, format!("/worktree create {arg}"), body);
        }
        "remove" => {
            if arg.is_empty() {
                echo(
                    app,
                    "/worktree remove".to_owned(),
                    "Usage: `/worktree remove <name>` (the `jfc/<name>` branch is preserved)"
                        .to_owned(),
                );
                return;
            }
            if let Err(e) = crate::worktrees::validate_name(arg) {
                echo(
                    app,
                    format!("/worktree remove {arg}"),
                    format!("**Error:** {e}"),
                );
                return;
            }
            let body = match crate::worktrees::remove_worktree(&repo_root, arg) {
                Ok(()) => format!(
                    "Removed worktree `.jfc-worktrees/{arg}`. The branch `jfc/{arg}` is preserved \
                     — recover with `git switch jfc/{arg}` from any checkout."
                ),
                Err(e) => format!("**Error:** {e}"),
            };
            echo(app, format!("/worktree remove {arg}"), body);
        }
        "switch" => {
            if arg.is_empty() {
                echo(
                    app,
                    "/worktree switch".to_owned(),
                    "Usage: `/worktree switch <name>`".to_owned(),
                );
                return;
            }
            if let Err(e) = crate::worktrees::validate_name(arg) {
                echo(
                    app,
                    format!("/worktree switch {arg}"),
                    format!("**Error:** {e}"),
                );
                return;
            }
            let target = std::path::PathBuf::from(&app.cwd)
                .join(".jfc-worktrees")
                .join(arg);
            // jfc's cwd is captured at startup, so we can't transparently
            // teleport mid-session — print the manual recipe.
            let body = format!(
                "To switch into `{name}`, run:\n```\ncd {path}\n```\nthen re-launch `jfc`. \
                 (jfc captures its cwd at startup; live cwd-switch is not yet wired.)",
                name = arg,
                path = target.display()
            );
            echo(app, format!("/worktree switch {arg}"), body);
        }
        other => {
            echo(
                app,
                format!("/worktree {args}"),
                format!(
                    "Unknown subcommand `{other}`. Try `/worktree list|create <name>|remove <name>|switch <name>`."
                ),
            );
        }
    }
}

fn execute_palette_action(app: &mut App, label: &str) {
    // Each palette entry is paired with the keybinding it replaces — the
    // status row used to advertise these explicitly, but they're now lifted
    // into the palette to free vertical space for the context gauge. The
    // bindings still work (handled at their original sites in `handle_key`);
    // the palette is just a discoverable index.
    match label {
        "Clear Messages (/clear)" => {
            app.messages.clear();
            app.streaming_text.clear();
            app.streaming_reasoning.clear();
            app.streaming_assistant_idx = None;
            app.switch_session(None);
        }
        "Compact Conversation (/compact)" => {
            app.force_compact_pending = true;
            app.messages.push(ChatMessage::user("/compact".into()));
            app.messages.push(ChatMessage::assistant(
                "Compaction queued — runs on the next turn.".into(),
            ));
        }
        "Toggle Sessions Sidebar (Ctrl+B)" => {
            app.show_sidebar = !app.show_sidebar;
            if app.show_sidebar {
                app.session_ids = crate::session::list_sessions();
            }
        }
        "Toggle Info Sidebar (Ctrl+S)" => {
            app.show_info_sidebar = !app.show_info_sidebar;
        }
        "Open Model Picker (Ctrl+M)" => {
            app.show_model_picker = true;
            app.model_picker_filter.clear();
            app.model_picker_selected = 0;
            app.model_picker_models = collect_all_models(app);
        }
        "Open Task Panel (Ctrl+T)" => {
            app.show_task_panel = true;
            app.task_panel_selected = 0;
        }
        "Toggle Thinking (Ctrl+O)" => {
            // Thinking toggle is a per-message expand/collapse — flip the
            // most recent reasoning row if there is one, otherwise no-op.
            if let Some(idx) = app.messages.len().checked_sub(1) {
                let entry = app.reasoning_expanded.entry(idx).or_insert(false);
                *entry = !*entry;
            }
        }
        "Continue Most Recent Session (/continue)" => {
            run_slash_command(app, "/continue");
        }
        "Show Tasks (/tasks)" => {
            run_slash_command(app, "/tasks");
        }
        "Show Help (/help)" => {
            run_slash_command(app, "/help");
        }
        _ => {}
    }
}

pub fn palette_items(app: &App) -> Vec<&'static str> {
    // Discoverability index for keybindings + slash commands. Order matches
    // expected frequency: clear/compact at the top because they're used on
    // every long session; less-frequent toggles further down.
    let all: &[&str] = &[
        "Clear Messages (/clear)",
        "Compact Conversation (/compact)",
        "Continue Most Recent Session (/continue)",
        "Toggle Sessions Sidebar (Ctrl+B)",
        "Toggle Info Sidebar (Ctrl+S)",
        "Open Model Picker (Ctrl+M)",
        "Open Task Panel (Ctrl+T)",
        "Toggle Thinking (Ctrl+O)",
        "Show Tasks (/tasks)",
        "Show Help (/help)",
    ];
    if app.palette_input.is_empty() {
        all.to_vec()
    } else {
        let needle = app.palette_input.to_lowercase();
        all.iter()
            .filter(|s| s.to_lowercase().contains(&needle))
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
    let fingerprint_input: Vec<_> = app
        .providers
        .iter()
        .map(|p| {
            let models = app
                .provider_models
                .get(p.name())
                .cloned()
                .unwrap_or_else(|| p.available_models());
            (
                p.name().to_string(),
                models
                    .iter()
                    .map(|m| {
                        (
                            m.provider.to_string(),
                            m.id.to_string(),
                            m.display_name.clone(),
                            m.context_window_tokens,
                        )
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect::<Vec<_>>();
    let key = crate::query::QueryKey::ModelPickerModels(crate::query::Fingerprint::new((
        &fingerprint_input,
        app.seat_tier.as_deref(),
    )));

    app.model_picker_query_cache.get_or_insert_with(key, || {
        let merged = fingerprint_input
            .iter()
            .flat_map(|(provider_name, _)| {
                app.provider_models
                    .get(provider_name.as_str())
                    .cloned()
                    .unwrap_or_else(|| {
                        app.providers
                            .iter()
                            .find(|p| p.name() == provider_name)
                            .map(|p| p.available_models())
                            .unwrap_or_default()
                    })
            })
            .collect();
        crate::providers::anthropic_models::apply_seat_tier_filter(merged, app.seat_tier.as_deref())
    })
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::app::App;
    use crate::provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

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

    fn test_app_with_input(input: &str, wrap_width: usize) -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.input_wrap_width = wrap_width;
        app.textarea = TextArea::from(input.lines().map(str::to_string).collect::<Vec<_>>());
        app
    }

    #[tokio::test]
    async fn up_and_down_move_across_soft_wrapped_input_rows() {
        let mut app = test_app_with_input("abcdefghij", 5);
        app.textarea.move_cursor(CursorMove::Jump(0, 8));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.textarea.cursor(), (0, 3));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.textarea.cursor(), (0, 8));
    }

    #[tokio::test]
    async fn up_and_down_still_cross_logical_input_lines() {
        let mut app = test_app_with_input("abc\ndefghijkl", 5);
        app.textarea.move_cursor(CursorMove::Jump(0, 2));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.textarea.cursor(), (1, 2));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.textarea.cursor(), (1, 7));

        handle_key(
            &mut app,
            KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.textarea.cursor(), (1, 2));
    }
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
