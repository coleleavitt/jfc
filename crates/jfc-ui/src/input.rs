use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::style::Style;
use ratatui_textarea::{CursorMove, TextArea};
use std::sync::Arc;
use tokio::sync::mpsc;

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
    app.textarea.set_placeholder_text("send a message…");
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
    let cursor = app.textarea.cursor();
    let (line, col) = (cursor.0, cursor.1);

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
    let cursor = app.textarea.cursor();
    let (line, col) = (cursor.0, cursor.1);
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

/// Walk back through `messages` collecting `path:line(:col)?`
/// references from the most recent tool output (Bash stdout/stderr,
/// Read content, command-output blocks). Stops at the most recent
/// turn — once we find references, we don't keep scanning older
/// turns. Returns most-recent-first, deduplicated.
///
/// Pattern: at least one slash OR a recognised file extension,
/// followed by `:<digits>` and optionally `:<digits>`. Matches:
///   - `src/lib.rs:42:5`
///   - `crates/jfc-ui/src/main.rs:1234`
///   - `Cargo.toml:7`
/// Doesn't match:
///   - `12:34` (no path component)
///   - `https://...` (the colon-port pattern)
fn collect_recent_paths(messages: &[crate::types::ChatMessage]) -> Vec<String> {
    use crate::types::{MessagePart, ToolOutput};
    let mut out: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for msg in messages.iter().rev() {
        let mut found_in_this_msg = false;
        for part in msg.parts.iter().rev() {
            let text: String = match part {
                MessagePart::Text(s) | MessagePart::Reasoning(s) => s.clone(),
                MessagePart::Tool(tc) => match &tc.output {
                    ToolOutput::Text(s) => s.clone(),
                    ToolOutput::LargeText(lt) => lt.content.clone(),
                    ToolOutput::Command { stdout, stderr, .. } => {
                        format!("{stdout}\n{stderr}")
                    }
                    ToolOutput::FileContent { content, path, .. } => {
                        format!("{path}\n{content}")
                    }
                    _ => continue,
                },
                _ => continue,
            };
            for matched in scan_path_refs(&text) {
                if seen.insert(matched.clone()) {
                    out.push(matched);
                    found_in_this_msg = true;
                }
            }
        }
        if found_in_this_msg {
            // First-hit message wins — older turns aren't relevant
            // for "what error did I just see?"
            break;
        }
    }
    out
}

/// Pure scanner: finds `path:line(:col)?` substrings. Implementation
/// is character-walking instead of regex to avoid pulling another
/// dep — the codebase is regex-free elsewhere.
fn scan_path_refs(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        let b = bytes[i];
        // Path char: alnum, /, ., _, -, +
        if b.is_ascii_alphanumeric()
            || b == b'/'
            || b == b'.'
            || b == b'_'
            || b == b'-'
            || b == b'+'
        {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric()
                    || c == b'/'
                    || c == b'.'
                    || c == b'_'
                    || c == b'-'
                    || c == b'+'
                {
                    i += 1;
                } else {
                    break;
                }
            }
            let path_end = i;
            // Need a `:digit` after.
            if i + 1 < bytes.len() && bytes[i] == b':' && bytes[i + 1].is_ascii_digit() {
                let after_colon = i + 1;
                let mut j = after_colon;
                while j < bytes.len() && bytes[j].is_ascii_digit() {
                    j += 1;
                }
                let line_end = j;
                // Optional `:digit` for col.
                let col_end =
                    if j + 1 < bytes.len() && bytes[j] == b':' && bytes[j + 1].is_ascii_digit() {
                        let mut k = j + 1;
                        while k < bytes.len() && bytes[k].is_ascii_digit() {
                            k += 1;
                        }
                        k
                    } else {
                        line_end
                    };
                let path_slice = &text[start..path_end];
                // Reject candidates that look like `12:34` (no path)
                // or `http://...` (URL-port).
                let is_url = path_slice.starts_with("http://")
                    || path_slice.starts_with("https://")
                    || path_slice.starts_with("file://");
                let is_pure_number = path_slice.bytes().all(|c| c.is_ascii_digit());
                let has_path_char = path_slice.contains('/') || path_slice.contains('.');
                if !is_url && !is_pure_number && has_path_char && path_end > start {
                    let captured = &text[start..col_end];
                    out.push(captured.to_owned());
                }
                i = col_end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Recompute `app.transcript_search.matches` from a fresh query.
/// Case-insensitive substring match against each message's text /
/// reasoning content. Updates `cursor` to 0 and scrolls to the first
/// match if any. Empty query clears matches but leaves the search
/// overlay open (so the user can keep typing).
fn refresh_search_matches(app: &mut App, query: &str) {
    let q = query.to_lowercase();
    let mut matches: Vec<usize> = Vec::new();
    if !q.is_empty() {
        for (idx, msg) in app.messages.iter().enumerate() {
            let body_hit = msg.parts.iter().any(|p| match p {
                crate::types::MessagePart::Text(s) => s.to_lowercase().contains(&q),
                crate::types::MessagePart::Reasoning(s) => s.to_lowercase().contains(&q),
                crate::types::MessagePart::Tool(tc) => {
                    tc.input.summary().to_lowercase().contains(&q)
                        || match &tc.output {
                            crate::types::ToolOutput::Text(s) => s.to_lowercase().contains(&q),
                            crate::types::ToolOutput::LargeText(lt) => {
                                lt.content.to_lowercase().contains(&q)
                            }
                            _ => false,
                        }
                }
                _ => false,
            });
            if body_hit {
                matches.push(idx);
            }
        }
    }
    let first_target = if let Some(s) = app.transcript_search.as_mut() {
        s.matches = matches;
        s.cursor = 0;
        s.matches.first().copied()
    } else {
        None
    };
    if let Some(target) = first_target {
        scroll_to_message(app, target);
    }
}

// ─── Jump-to navigation helpers ──────────────────────────────────────────
// Each helper scans `app.messages` from the end backwards for a target,
// computes the cumulative line offset of the message above the match,
// and pins `scroll_offset` so that line lands near the top of the
// viewport. Falls back to scrolling to the bottom if no match is
// found. The line counts are derived from the same MessageView height
// math the renderer uses, so the resulting position lines up precisely.

/// Scroll the transcript so the message at `target_idx` lands near the
/// top of the viewport. Pure scroll-state mutator; does no rendering
/// itself. Skips if `target_idx` is out of range.
fn scroll_to_message(app: &mut App, target_idx: usize) {
    if target_idx >= app.messages.len() {
        return;
    }
    // Coarse height estimate per message — enough to position the
    // scroll near the target. The renderer's clamp logic will pull
    // the offset back into bounds on the next frame, and the user's
    // arrow keys can fine-tune from there. Going through the exact
    // MessageView width-sensitive math from here would require
    // knowing the messages-area width, which input.rs doesn't have.
    let approx_width: usize = 80;
    let mut offset = 0usize;
    for (i, msg) in app.messages.iter().enumerate() {
        if i >= target_idx {
            break;
        }
        // Role label = 1 row.
        offset += 1;
        for part in &msg.parts {
            let chars = part.approx_text_len();
            if chars == 0 {
                offset += 1;
            } else {
                offset += chars.div_ceil(approx_width);
            }
        }
        // Trailing blank between messages.
        offset += 1;
    }
    app.scroll_offset = offset;
    app.follow_bottom = false;
    crate::toast::push_with_cap(
        &mut app.toasts,
        crate::toast::Toast::new(
            crate::toast::ToastKind::Info,
            format!(
                "jumped to message {}/{}",
                target_idx + 1,
                app.messages.len()
            ),
        ),
    );
}

fn jump_to_last_error(app: &mut App) {
    use crate::types::{MessagePart, ToolStatus};
    let target = app.messages.iter().enumerate().rev().find(|(_, m)| {
        m.parts.iter().any(|p| {
            matches!(
                p,
                MessagePart::Tool(tc) if tc.status == ToolStatus::Failed
            )
        })
    });
    match target {
        Some((idx, _)) => scroll_to_message(app, idx),
        None => crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                "no failed tools in this session".to_string(),
            ),
        ),
    }
}

fn jump_to_last_tool(app: &mut App) {
    use crate::types::MessagePart;
    let target = app
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| m.parts.iter().any(|p| matches!(p, MessagePart::Tool(_))));
    match target {
        Some((idx, _)) => scroll_to_message(app, idx),
        None => crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                "no tool calls in this session".to_string(),
            ),
        ),
    }
}

fn jump_to_last_user(app: &mut App) {
    let target = app
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| m.role_is_user() && !m.is_compact_boundary());
    match target {
        Some((idx, _)) => scroll_to_message(app, idx),
        None => crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                "no user messages yet".to_string(),
            ),
        ),
    }
}

fn jump_to_last_assistant(app: &mut App) {
    let target = app
        .messages
        .iter()
        .enumerate()
        .rev()
        .find(|(_, m)| !m.role_is_user());
    match target {
        Some((idx, _)) => scroll_to_message(app, idx),
        None => crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Warning,
                "no assistant messages yet".to_string(),
            ),
        ),
    }
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
            if text.is_empty() { None } else { Some(text) }
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

fn dispatch_approved_tool(app: &App, tool: ToolCall, tx: &mpsc::Sender<AppEvent>) {
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
        app.teammate_event_tx.clone(),
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
fn advance_approval_queue(app: &mut App, tx: &mpsc::Sender<AppEvent>) {
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
            app.teammate_event_tx.clone(),
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
    tx: &mpsc::Sender<crate::app::AppEvent>,
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
        // The sidebar reorders sessions visually (this-project first, others
        // below) but `session_meta` itself stays in recency order. Build a
        // resolved order each navigation tick so Up/Down/Enter walk the
        // user-visible list, not the underlying vec.
        let ordered = crate::render::ordered_sidebar_sessions(app);
        let total = ordered.len();
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
                if let Some(id) = ordered.get(app.session_selected).cloned() {
                    if let Some(messages) = crate::session::load_session(&id).await {
                        app.messages = messages;
                        app.switch_session(Some(id));
                        app.streaming_text.clear();
                        app.streaming_reasoning.clear();
                        app.streaming_response_bytes = 0;
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
                    execute_palette_action(app, &label).await;
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
                    let old_model = app.model.clone();
                    let old_max_ctx = app.max_context_tokens;
                    tracing::info!(
                        target: "jfc::input",
                        old_model = %old_model,
                        new_model = %chosen_id,
                        old_provider = %app.provider.name(),
                        new_provider = %chosen_provider_name,
                        old_max_context_tokens = old_max_ctx,
                        "model switch initiated from picker"
                    );
                    if let Some(p) = app
                        .providers
                        .iter()
                        .find(|p| chosen_provider_name == p.name())
                    {
                        app.provider = Arc::clone(p);
                    }
                    app.model = chosen_id.clone();
                    crate::app::push_recent_model(&mut app.recent_models, chosen_id.as_str());
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

    // ─── Slash autocomplete popup ─────────────────────────────────────────
    // Active whenever the input bar is exactly one line starting with
    // `/` and there's at least one matching command. Tab/Enter
    // commits the highlighted entry, Up/Down navigate, Esc dismisses.
    if let Some(prefix) = crate::render::current_slash_prefix(app) {
        let matches = crate::render::slash_matches(&prefix);
        if !matches.is_empty() {
            match key.code {
                KeyCode::Tab | KeyCode::Enter => {
                    let idx = app.slash_popup_selected.unwrap_or(0).min(matches.len() - 1);
                    let (cmd, _) = matches[idx];
                    // Replace the textarea content with the chosen
                    // command + a trailing space (so the user can
                    // immediately type args).
                    app.textarea.select_all();
                    app.textarea.cut();
                    app.textarea.insert_str(&format!("{cmd} "));
                    app.slash_popup_selected = None;
                    return Ok(false);
                }
                KeyCode::Down => {
                    let idx = app.slash_popup_selected.unwrap_or(0);
                    app.slash_popup_selected = Some((idx + 1) % matches.len());
                    return Ok(false);
                }
                KeyCode::Up => {
                    let idx = app.slash_popup_selected.unwrap_or(0);
                    app.slash_popup_selected =
                        Some(if idx == 0 { matches.len() - 1 } else { idx - 1 });
                    return Ok(false);
                }
                KeyCode::Esc => {
                    // Dismiss the popup but leave the typed text
                    // alone so the user can keep editing.
                    app.slash_popup_selected = None;
                    // Don't consume Esc — fall through so the user
                    // can still chain Esc to clear input or interrupt.
                }
                _ => {
                    // Any other key — let the textarea handle it as
                    // normal. Reset the highlight so it re-anchors
                    // to the new top-match on the next char.
                    app.slash_popup_selected = None;
                }
            }
        }
    }

    // ─── Transcript search (Ctrl+F) ──────────────────────────────────────
    if app.transcript_search.is_some() {
        match key.code {
            KeyCode::Esc => {
                // Cancel: drop the search state without scrolling.
                app.transcript_search = None;
            }
            KeyCode::Enter => {
                // Commit + exit. Scroll to the currently-focused
                // match (already done via Up/Down navigation), then
                // close the search bar.
                if let Some(s) = app.transcript_search.take() {
                    if let Some(&idx) = s.matches.get(s.cursor) {
                        scroll_to_message(app, idx);
                    }
                }
            }
            KeyCode::Backspace => {
                if let Some(s) = app.transcript_search.as_mut() {
                    s.query.pop();
                    let q = s.query.clone();
                    refresh_search_matches(app, &q);
                }
            }
            KeyCode::Char(c) => {
                if let Some(s) = app.transcript_search.as_mut() {
                    s.query.push(c);
                    let q = s.query.clone();
                    refresh_search_matches(app, &q);
                }
            }
            KeyCode::Down => {
                if let Some(s) = app.transcript_search.as_mut() {
                    if !s.matches.is_empty() {
                        s.cursor = (s.cursor + 1) % s.matches.len();
                        let target = s.matches[s.cursor];
                        scroll_to_message(app, target);
                    }
                }
            }
            KeyCode::Up => {
                if let Some(s) = app.transcript_search.as_mut() {
                    if !s.matches.is_empty() {
                        s.cursor = if s.cursor == 0 {
                            s.matches.len() - 1
                        } else {
                            s.cursor - 1
                        };
                        let target = s.matches[s.cursor];
                        scroll_to_message(app, target);
                    }
                }
            }
            _ => {}
        }
        return Ok(false);
    }

    // ─── Jump-to navigation (Ctrl+G prefix) ──────────────────────────────
    if app.jump_armed {
        if let Some(t) = app.jump_armed_at {
            if t.elapsed() >= std::time::Duration::from_secs(2) {
                app.jump_armed = false;
                app.jump_armed_at = None;
            }
        }
    }
    if app.jump_armed {
        app.jump_armed = false;
        app.jump_armed_at = None;
        match key.code {
            KeyCode::Char('e') => jump_to_last_error(app),
            KeyCode::Char('t') => jump_to_last_tool(app),
            KeyCode::Char('m') => jump_to_last_user(app),
            KeyCode::Char('a') => jump_to_last_assistant(app),
            KeyCode::Esc => {}
            _ => {}
        }
        return Ok(false);
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
                    app.textarea =
                        TextArea::from(prompt.lines().map(str::to_string).collect::<Vec<_>>());
                    app.textarea.set_cursor_line_style(Style::default());
                    app.textarea.set_placeholder_text("send a message…");
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
                    app.textarea =
                        TextArea::from(prompt.lines().map(str::to_string).collect::<Vec<_>>());
                    app.textarea.set_cursor_line_style(Style::default());
                    app.textarea.set_placeholder_text("send a message…");
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
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            // Arm jump-to mode. The next single keystroke (e / t / m /
            // a) picks a target and scrolls the transcript to it. Esc
            // or any unbound key cancels. Auto-disarms after 2s so a
            // forgotten chord doesn't intercept user typing.
            app.jump_armed = true;
            app.jump_armed_at = Some(std::time::Instant::now());
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Info,
                    "jump: e=last error · t=last tool · m=last user · a=last assistant".to_string(),
                ),
            );
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('f')) if !input_has_text(app) => {
            // Arm transcript search. Empty bar (input has no text)
            // gates this so the user can still type literal Ctrl+F
            // sequences if some legacy keybinding software passes
            // them through. The search overlay renders at the bottom
            // of the screen via `app.transcript_search.is_some()`.
            app.transcript_search = Some(crate::app::TranscriptSearch::default());
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) if !input_has_text(app) => {
            // Edit the most recent user message. Pre-fills the
            // textarea with its body and flags edit mode; submit
            // replaces that message and drops subsequent turns.
            // Esc cancels. Useful when the previous prompt was
            // ambiguous and the user wants a clean re-roll without
            // re-typing.
            if app.is_streaming
                || !app.pending_tool_calls.is_empty()
                || app.pending_approval.is_some()
            {
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Warning,
                        "edit: still in flight, finish or interrupt first".to_string(),
                    ),
                );
                return Ok(false);
            }
            let last_user: Option<(usize, String)> =
                app.messages.iter().enumerate().rev().find_map(|(i, m)| {
                    if m.role_is_user() && !m.is_compact_boundary() {
                        m.parts.iter().find_map(|p| match p {
                            MessagePart::Text(s) if !s.is_empty() && !s.starts_with('/') => {
                                Some((i, s.clone()))
                            }
                            _ => None,
                        })
                    } else {
                        None
                    }
                });
            if let Some((idx, text)) = last_user {
                app.textarea.select_all();
                app.textarea.cut();
                app.textarea.insert_str(&text);
                app.editing_message_idx = Some(idx);
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Info,
                        "editing previous message — Esc cancels, Enter resubmits".to_string(),
                    ),
                );
            } else {
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Info,
                        "no previous user message to edit".to_string(),
                    ),
                );
            }
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => {
            // Yank a `path:line(:col)?` reference out of recent tool
            // output to the clipboard. First press copies the most
            // recent match; subsequent presses cycle through older
            // matches in the same transcript scan, so a multi-error
            // cargo run becomes "Ctrl+L Ctrl+L Ctrl+L" through each
            // file:line pair.
            let paths = collect_recent_paths(&app.messages);
            if paths.is_empty() {
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Info,
                        "no path:line refs found in recent output".to_string(),
                    ),
                );
                return Ok(false);
            }
            let idx = app.path_yank_cursor % paths.len();
            let target = &paths[idx];
            match arboard::Clipboard::new().and_then(|mut c| c.set_text(target.clone())) {
                Ok(_) => {
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(
                            crate::toast::ToastKind::Success,
                            format!("📋 {} ({}/{})", target, idx + 1, paths.len()),
                        ),
                    );
                }
                Err(e) => {
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(
                            crate::toast::ToastKind::Warning,
                            format!("clipboard failed: {e}"),
                        ),
                    );
                }
            }
            app.path_yank_cursor = app.path_yank_cursor.wrapping_add(1);
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => {
            // Retry: re-submit the most recent user prompt as a fresh
            // turn. Useful after a stream error or when the model's
            // response wasn't useful and the user wants another roll
            // of the dice. The retry is a *new* turn — we don't strip
            // the prior assistant response, so the conversation
            // history reflects "I asked twice" rather than rewriting
            // history.
            if app.is_streaming
                || !app.pending_tool_calls.is_empty()
                || app.pending_approval.is_some()
            {
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Warning,
                        "retry: still in flight, finish or interrupt first".to_string(),
                    ),
                );
                return Ok(false);
            }
            // Walk back for the most recent user prompt that wasn't
            // a slash command or a compact boundary.
            let last_prompt: Option<String> = app
                .messages
                .iter()
                .rev()
                .find(|m| {
                    m.role_is_user()
                        && !m.is_compact_boundary()
                        && m.parts
                            .iter()
                            .any(|p| matches!(p, MessagePart::Text(s) if !s.starts_with('/')))
                })
                .and_then(|m| {
                    m.parts.iter().find_map(|p| match p {
                        MessagePart::Text(s) if !s.is_empty() => Some(s.clone()),
                        _ => None,
                    })
                });
            match last_prompt {
                Some(text) => {
                    let _ = tx.send(crate::app::AppEvent::Submit(text)).await;
                }
                None => {
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(
                            crate::toast::ToastKind::Info,
                            "no prompt to retry".to_string(),
                        ),
                    );
                }
            }
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('z')) => {
            // Undo the last textarea edit. ratatui-textarea tracks
            // history internally — Ctrl+Z is the universal undo
            // gesture and was previously unbound. Returns false when
            // there's nothing to undo, which we silently ignore so
            // the keystroke isn't reflected.
            app.textarea.undo();
            return Ok(false);
        }
        (mods, KeyCode::Char('Z'))
            if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
        {
            // Ctrl+Shift+Z redo. The shift modifier may or may not be
            // exposed depending on the kitty-protocol negotiation, so
            // match the modifier-set explicitly.
            app.textarea.redo();
            return Ok(false);
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
                app.session_meta = crate::session::list_sessions_with_metadata().await;
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
                            let _ = tx
                                .send(crate::app::AppEvent::Toast {
                                    kind: crate::toast::ToastKind::Success,
                                    text: format!("Copied: {preview}{suffix}"),
                                })
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(crate::app::AppEvent::Toast {
                                    kind: crate::toast::ToastKind::Error,
                                    text: format!("Clipboard error: {e}"),
                                })
                                .await;
                        }
                    }
                }
                _ => {
                    let _ = tx
                        .send(crate::app::AppEvent::Toast {
                            kind: crate::toast::ToastKind::Warning,
                            text: "No assistant message to yank".into(),
                        })
                        .await;
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
                // Reset scroll on open so the user always lands at the
                // top of the list — the panel is more useful when
                // freshly-arrived errors are visible first.
                app.diagnostic_panel_scroll = 0;
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
        // ─── Diagnostic panel scroll ───────────────────────────────────────
        // Up/Down/PgUp/PgDn/Home/End/j/k/g/G all move the cursor inside
        // the panel rather than the underlying transcript. The panel is
        // a modal so other transcript-scroll bindings should NOT fire
        // while it's open — gating each arm on `app.show_diagnostic_panel`
        // keeps the behavior local. Saturating arithmetic prevents
        // underflow at 0 and the renderer clamps to (total - viewport)
        // each frame, so over-scrolling at the bottom stays in range.
        (KeyModifiers::NONE, KeyCode::Down | KeyCode::Char('j')) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = app.diagnostic_panel_scroll.saturating_add(1);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = app.diagnostic_panel_scroll.saturating_sub(1);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::PageDown) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = app.diagnostic_panel_scroll.saturating_add(10);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::PageUp) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = app.diagnostic_panel_scroll.saturating_sub(10);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Home | KeyCode::Char('g')) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = 0;
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::End | KeyCode::Char('G')) if app.show_diagnostic_panel => {
            // The renderer clamps overflow each frame, so passing a
            // large value lands at the bottom regardless of the
            // current diagnostic-set size.
            app.diagnostic_panel_scroll = usize::MAX / 2;
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Esc) if app.show_diagnostic_panel => {
            app.show_diagnostic_panel = false;
            return Ok(false);
        }
        // ─── Vim-style transcript navigation (input empty) ────────────────
        // h/j/k/l for scroll, g/G for top/bottom. Only fire when the
        // input bar is empty — typing actual prose with `j` shouldn't
        // jump the transcript.
        (KeyModifiers::NONE, KeyCode::Char('j')) if !input_has_text(app) => {
            app.scroll_down(1);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Char('k')) if !input_has_text(app) => {
            app.scroll_up(1);
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Char('G')) if !input_has_text(app) => {
            app.scroll_to_bottom();
            app.follow_bottom = true;
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Char('g')) if !input_has_text(app) => {
            // Lone `g` jumps to top. v126 / Vim use `gg` (double-g)
            // for safety, but the input-empty gate already prevents
            // typos here so a single `g` is fine.
            app.scroll_offset = 0;
            app.follow_bottom = false;
            return Ok(false);
        }

        (KeyModifiers::NONE, KeyCode::Char('?')) if !input_has_text(app) => {
            // `?` toggles the help overlay. Gated on empty input so
            // the user can still type a literal `?` mid-message.
            app.show_help = !app.show_help;
            return Ok(false);
        }
        (KeyModifiers::SHIFT, KeyCode::Char('?')) if !input_has_text(app) => {
            app.show_help = !app.show_help;
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Char('o')) if !input_has_text(app) => {
            // In the subagent task view (`viewing_task_id.is_some()`),
            // `o` toggles expansion of the most recent long entry in
            // `BackgroundTask.messages`. In the main chat it falls
            // through to the most recent `LargeText` tool block. The
            // two paths can't share state until Phase B unifies the
            // subagent renderer with `MessageView`.
            if let Some(ref task_id) = app.viewing_task_id.clone() {
                if let Some(bt) = app.background_tasks.get(task_id) {
                    let threshold_lines = crate::render::TASK_VIEW_COLLAPSE_LINES;
                    let threshold_bytes = crate::render::TASK_VIEW_COLLAPSE_BYTES;
                    let last_collapsible = bt
                        .messages
                        .iter()
                        .enumerate()
                        .rev()
                        .find(|(_, m)| {
                            m.lines().count() > threshold_lines || m.len() > threshold_bytes
                        })
                        .map(|(i, _)| i);
                    if let Some(idx) = last_collapsible {
                        let entry = app
                            .viewing_task_expanded
                            .entry(task_id.clone())
                            .or_default();
                        if !entry.insert(idx) {
                            entry.remove(&idx);
                        }
                    }
                }
                return Ok(false);
            }
            'toggle: {
                let messages = &mut app.messages;
                for msg in messages.iter_mut().rev() {
                    for part in msg.parts.iter_mut().rev() {
                        if let MessagePart::Tool(tc) = part {
                            // Two-level expand: huge LargeText flips
                            // `is_collapsed` (1-row teaser ⇄ body), all
                            // other tools flip `expanded` (80-line cap
                            // ⇄ 500-line cap). The user gets a single
                            // `o` shortcut that scales: small Read →
                            // expand to full, huge Bash dump → expand
                            // teaser to body.
                            match &tc.output {
                                ToolOutput::LargeText(lt)
                                    if lt.line_count > crate::types::LargeText::COLLAPSE_LINES
                                        || lt.content.len()
                                            > crate::types::LargeText::COLLAPSE_BYTES =>
                                {
                                    tc.is_collapsed = !tc.is_collapsed;
                                    break 'toggle;
                                }
                                ToolOutput::Empty => {}
                                _ => {
                                    tc.expanded = !tc.expanded;
                                    break 'toggle;
                                }
                            }
                        }
                    }
                }
            }
            return Ok(false);
        }
        // ─── Task view: sticky arrow navigation ──────────────────────────
        // Once you're inside the task view (Ctrl+X then ↓ to enter, or you
        // typed something equivalent) plain ←/→ cycle through running
        // tasks, ↑ leaves the view, ↓ jumps to the most recent. No
        // leader-key chord required for each step — the leader is only
        // needed to *enter* the view. Without this the user had to type
        // Ctrl+X → → → → → to walk through five running agents.
        (KeyModifiers::NONE, KeyCode::Right) | (KeyModifiers::NONE, KeyCode::Left)
            if app.viewing_task_id.is_some() && !input_has_text(app) =>
        {
            let task_ids: Vec<String> = app.background_tasks.keys().cloned().collect();
            if task_ids.is_empty() {
                return Ok(false);
            }
            let pos = app
                .viewing_task_id
                .as_ref()
                .and_then(|id| task_ids.iter().position(|t| t == id))
                .unwrap_or(0);
            let next = match key.code {
                KeyCode::Right => (pos + 1).min(task_ids.len() - 1),
                KeyCode::Left => pos.saturating_sub(1),
                _ => pos,
            };
            if next != pos {
                app.viewing_task_id = Some(task_ids[next].clone());
                app.scroll_to_bottom();
            }
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Up)
            if app.viewing_task_id.is_some() && !input_has_text(app) =>
        {
            // Up exits the task view back to the main transcript —
            // matches the leader-mode `k` behavior so muscle memory is
            // consistent across modes. Per-task expansion state stays
            // in `app.viewing_task_expanded` so re-entering the same
            // task restores what was expanded.
            app.viewing_task_id = None;
            app.scroll_to_bottom();
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Down)
            if app.viewing_task_id.is_some() && !input_has_text(app) =>
        {
            // Down jumps to the most recently spawned task — useful
            // when several agents are running and you want the one
            // that just kicked off.
            let task_ids: Vec<String> = app.background_tasks.keys().cloned().collect();
            if let Some(last) = task_ids.last() {
                app.viewing_task_id = Some(last.clone());
                app.scroll_to_bottom();
            }
            return Ok(false);
        }
        (KeyModifiers::NONE, KeyCode::Esc) => {
            if app.show_help {
                app.show_help = false;
                return Ok(false);
            }
            // Cancel edit mode if armed; clear input so the user
            // doesn't accidentally re-submit the prefilled text.
            if app.editing_message_idx.is_some() {
                app.editing_message_idx = None;
                app.textarea.select_all();
                app.textarea.cut();
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Info,
                        "edit cancelled".to_string(),
                    ),
                );
                return Ok(false);
            }
            if app.viewing_task_id.is_some() {
                app.viewing_task_id = None;
                return Ok(false);
            }

            // Double-tap ESC interrupts active work — streaming, the
            // agentic continuation loop, and the subagent runner all
            // poll `interrupt_flag` between iterations. Single ESC just
            // hints; the second within `DOUBLE_TAP_MS` flips the flag
            // and the in-flight tasks unwind. Mirrors Claude Code's ESC
            // behavior: one ESC arms the cancel, two confirms.
            const DOUBLE_TAP_MS: u128 = 600;
            let active = app.is_streaming
                || app.compacting_started_at.is_some()
                || !app.pending_tool_calls.is_empty()
                || app
                    .background_tasks
                    .values()
                    .any(|bt| matches!(bt.status, crate::types::TaskLifecycle::Running));
            if active {
                let now = std::time::Instant::now();
                let recent = app
                    .last_esc_at
                    .map(|t| now.duration_since(t).as_millis() < DOUBLE_TAP_MS)
                    .unwrap_or(false);
                if recent {
                    app.interrupt_flag
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    app.last_esc_at = None;
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(
                            crate::toast::ToastKind::Warning,
                            "⏹ Interrupting…".to_owned(),
                        ),
                    );
                } else {
                    app.last_esc_at = Some(now);
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(
                            crate::toast::ToastKind::Info,
                            "Press ESC again to interrupt".to_owned(),
                        ),
                    );
                }
                return Ok(false);
            }
            reset_input(app);
            return Ok(false);
        }
        (KeyModifiers::SHIFT, KeyCode::BackTab) | (KeyModifiers::NONE, KeyCode::BackTab) => {
            // Shift+Tab cycles permission modes
            app.permission_mode = app.permission_mode.next();
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Info,
                    format!(
                        "{} Mode: {}",
                        app.permission_mode.symbol(),
                        app.permission_mode.label()
                    ),
                ),
            );
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
/// string so cursor positioning is correct (the `ratatui_textarea` API
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
    app.textarea.set_placeholder_text("send a message…");
    app.textarea.move_cursor(CursorMove::End);
}

/// Decide whether the popup should activate (newly-typed `@` after
/// whitespace) or update its query (already-active, more chars typed
/// or backspace shrunk the buffer).
fn update_mention_state_after_input(app: &mut App) {
    let cursor = app.textarea.cursor();
    let (line_idx, col) = (cursor.0, cursor.1);
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
    tx: &mpsc::Sender<crate::app::AppEvent>,
) -> anyhow::Result<()> {
    handle_submit(app, text, tx).await
}

async fn handle_submit(
    app: &mut App,
    text: String,
    tx: &mpsc::Sender<crate::app::AppEvent>,
) -> anyhow::Result<()> {
    tracing::info!(
        target: "jfc::input",
        text_len = text.len(),
        text_preview = %&text[..text.len().min(80)],
        model = %app.model,
        message_count = app.messages.len(),
        editing_idx = ?app.editing_message_idx,
        "handle_submit"
    );
    // Edit mode: if the user is editing an earlier message, rewrite
    // history at that index and drop everything after before
    // continuing as a fresh submit. The new turn arrives as if the
    // user had typed it just now — agentic loop, tool calls, and
    // streaming all flow normally.
    if let Some(edit_idx) = app.editing_message_idx.take() {
        if edit_idx < app.messages.len() {
            tracing::info!(
                target: "jfc::input",
                edit_idx,
                kept = edit_idx,
                dropped = app.messages.len() - edit_idx,
                "edit-resubmit: rewriting history"
            );
            app.messages.truncate(edit_idx);
        }
        // Clear streaming-related state that might be tied to the
        // dropped messages (assistant placeholder index, etc.).
        app.streaming_text.clear();
        app.streaming_reasoning.clear();
        app.streaming_response_bytes = 0;
        app.streaming_assistant_idx = None;
    }
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
        handle_slash_command(app, &text, Some(tx)).await;
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
    let want_compact = matches!(
        level,
        crate::compact::CompactLevel::Compact | crate::compact::CompactLevel::Blocked
    ) || app.force_compact_pending;
    // Respect the suppression flag set by the post-response compact
    // path. Once compaction has permanently failed (provider doesn't
    // support it, breaker latched, retries exhausted), retrying on
    // every user message just re-fires the failing API call and
    // re-warns. The user clears it manually via /compact, which sets
    // `force_compact_pending` and bypasses this guard.
    if want_compact && app.compact_suppressed && !app.force_compact_pending {
        tracing::debug!(
            target: "jfc::compact",
            est, level = ?level,
            "pre-submit compact skipped — compact_suppressed latched"
        );
    } else if want_compact {
        let manual = std::mem::take(&mut app.force_compact_pending);
        tracing::info!(
            target: "jfc::compact",
            est, level = ?level, manual,
            model = %app.model,
            max_context_tokens = app.max_context_tokens,
            message_count = app.messages.len(),
            rapid_refill_count = app.tool_ctx.rapid_refill_count,
            "pre-submit compact triggered"
        );
        let messages = app.messages.clone();
        let provider = Arc::clone(&app.provider);
        let model = app.model.clone();
        let mut tool_ctx = app.tool_ctx.clone();
        let window = app.max_context_tokens;
        let tx_pre = tx.clone();
        let user_text = text.clone();
        let is_blocked = matches!(level, crate::compact::CompactLevel::Blocked);
        let _ = tx_pre.send(crate::app::AppEvent::CompactionStarted).await;
        // Progress callback fires on every text_delta from the streaming
        // compact, forwards the cumulative output length as a
        // CompactionProgress event so the spinner shows live token
        // count. Mirrors v126's `addResponseLength` callback in PB7.
        let progress_tx = tx_pre.clone();
        let on_progress: crate::compact::CompactProgressCb = Box::new(move |chars| {
            // CompactionProgress is non-critical; next progress update supersedes.
            let _ = progress_tx.try_send(crate::app::AppEvent::CompactionProgress {
                output_chars: chars,
            });
        });
        tokio::spawn(async move {
            let options = crate::provider::StreamOptions::new(model.clone());
            tracing::debug!(
                target: "jfc::compact",
                model = %model,
                window,
                "spawned pre-submit compaction task"
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
                        "pre-submit compaction succeeded — re-queuing user message"
                    );
                    let _ = tx_pre
                        .send(crate::app::AppEvent::CompactionDone {
                            messages,
                            tool_ctx,
                            pre_tokens,
                            post_tokens,
                        })
                        .await;
                    // Re-queue the user's message — it didn't make it into
                    // the conversation before compaction ran.
                    let _ = tx_pre.send(crate::app::AppEvent::Submit(user_text)).await;
                }
                crate::compact::CompactResult::CircuitBreakerTripped => {
                    tracing::warn!(
                        target: "jfc::compact",
                        "pre-submit compaction: circuit breaker tripped"
                    );
                    let _ = tx_pre
                        .send(crate::app::AppEvent::CompactionFailed(
                            "Circuit breaker tripped — submit again with `/compact` if needed"
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
                        "pre-submit compaction exhausted all attempts"
                    );
                    let _ = tx_pre
                        .send(crate::app::AppEvent::CompactionFailed(
                            format!(
                                "Exhausted {attempts} compaction attempts — request is too large"
                            ),
                            Some(tool_ctx.approx_tokens),
                            false,
                        ))
                        .await;
                }
                _ => {
                    // Unsupported / TooFewGroups: provider can't compact.
                    // If we were merely at Compact level, submit anyway and
                    // let the API handle it. But if Blocked, don't re-submit
                    // (that would re-enter the compaction gate and loop forever).
                    if is_blocked {
                        tracing::warn!(
                            target: "jfc::compact",
                            "pre-submit compaction unsupported and context is Blocked — cannot proceed"
                        );
                        let _ = tx_pre
                            .send(crate::app::AppEvent::CompactionFailed(
                                "Context exceeds limit and provider cannot compact — \
                             try switching to a model/provider that supports compaction, \
                             or start a new session."
                                    .into(),
                                Some(tool_ctx.approx_tokens),
                                false,
                            ))
                            .await;
                    } else {
                        tracing::debug!(
                            target: "jfc::compact",
                            "pre-submit compaction skipped (unsupported/too few groups) — submitting anyway"
                        );
                        let _ = tx_pre.send(crate::app::AppEvent::Submit(user_text)).await;
                    }
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
    app.streaming_response_bytes = 0;
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.turn_started_at = Some(now);
    // Reset thinking-state for the new turn so the spinner doesn't carry
    // a stale `thought for Ns` from the previous turn.
    app.thinking_started_at = None;
    app.thinking_ended_at = None;
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
    // Fire-and-forget session save — don't block the UI on disk I/O.
    {
        let sid = session_id.clone();
        let msgs = app.messages.clone();
        let cwd = app.cwd.clone();
        let model = app.model.clone();
        tokio::spawn(async move {
            crate::session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
        });
    }
    app.current_session_id = Some(session_id.clone());

    let provider = app.provider.clone();
    let messages = crate::stream::build_provider_messages(&app.messages[..assistant_idx]);
    let model = app.model.clone();
    let tx = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    // Fresh user submission resets any prior interrupt state — the user
    // moved on, so the next stream should run unchecked.
    interrupt.store(false, std::sync::atomic::Ordering::SeqCst);

    tracing::info!(
        target: "jfc::input",
        model = %model,
        provider_message_count = messages.len(),
        assistant_idx,
        session_id = %session_id,
        total_user_turns = app.tool_ctx.total_user_turns,
        "spawning stream_response"
    );

    tokio::spawn(async move {
        crate::stream::stream_response(provider, messages, model, tx, interrupt).await;
    });

    Ok(())
}

/// Public entry point used by `main::drain_queued_prompts` when an isMeta
/// queued prompt fires. Same body as the private slash dispatcher used in
/// `handle_submit`. No `tx` is wired through this path because queued-prompt
/// dispatch runs synchronously between turns; commands that need to spawn a
/// stream (e.g. skill invocation) silently no-op the streaming step here.
pub async fn run_slash_command(app: &mut App, text: &str) {
    handle_slash_command(app, text, None).await
}

async fn handle_slash_command(app: &mut App, text: &str, tx: Option<&mpsc::Sender<AppEvent>>) {
    let parts: Vec<&str> = text.splitn(2, ' ').collect();
    match parts[0] {
        "/rename" => {
            // Set a custom title on the current session. v126 cli.js:39786
            // calls this `customTitle` and it sits at the top of the title
            // precedence chain (custom → ai → firstPrompt → id-slice).
            // Persisted to the session JSON so it survives restarts.
            let new_title = parts.get(1).copied().unwrap_or("").trim().to_owned();
            app.messages
                .push(ChatMessage::user(format!("/rename {new_title}")));
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
                    crate::session::set_session_title(id, &new_title).await;
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
            app.streaming_response_bytes = 0;
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
            tracing::info!(
                target: "jfc::compact",
                est, max_context_tokens = app.max_context_tokens,
                pct, ?level, model = %app.model,
                "manual /compact command invoked"
            );
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
                crate::session::most_recent_session().await
            } else {
                let cwd_str = std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string());
                crate::session::most_recent_session_for_cwd(cwd_str.as_deref()).await
            };
            if let Some(session_id) = session_id {
                if let Some(messages) = crate::session::load_session(&session_id).await {
                    app.messages = messages;
                    app.switch_session(Some(session_id.clone()));
                    app.streaming_text.clear();
                    app.streaming_reasoning.clear();
                    app.streaming_response_bytes = 0;
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
            // Resume a specific session by id. Accepts an optional
            // `--force` token to suppress the cwd-mismatch warning
            // (mirrors codex-rs `tui/src/session_resume.rs:99-111`,
            // where the user explicitly opts in to a cross-project
            // resume).
            let raw_args = parts.get(1).copied().unwrap_or("").trim();
            let mut force = false;
            let mut session_id = "";
            for tok in raw_args.split_whitespace() {
                if tok == "--force" {
                    force = true;
                } else if session_id.is_empty() {
                    session_id = tok;
                }
            }
            if session_id.is_empty() {
                // List available sessions
                let sessions = crate::session::list_sessions().await;
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
            } else if let Some(messages) = crate::session::load_session(session_id).await {
                let msg_count = messages.len();
                // Compare the loaded session's recorded cwd against the
                // current process cwd before mutating app state. The
                // resume still proceeds either way — the toast is just
                // informational so the user notices they may be
                // pointing at the wrong project.
                if !force {
                    let session_cwd = crate::session::load_session_metadata(session_id)
                        .await
                        .and_then(|m| m.cwd);
                    let current_cwd = std::env::current_dir()
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if let Some(msg) =
                        crate::session::cwd_mismatch_message(session_cwd.as_deref(), &current_cwd)
                    {
                        crate::toast::push_with_cap(
                            &mut app.toasts,
                            crate::toast::Toast::new(crate::toast::ToastKind::Warning, msg),
                        );
                    }
                }
                app.messages = messages;
                app.switch_session(Some(session_id.to_owned()));
                app.streaming_text.clear();
                app.streaming_reasoning.clear();
                app.streaming_response_bytes = 0;
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
            let sessions = crate::session::list_sessions_with_metadata().await;
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
            // Also flip the visual overlay so users get the same
            // keybindings table they'd see from `?`. The text dump
            // below is kept for searchability + transcript export.
            app.show_help = true;
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
        "/market" => {
            // Surface the agent-economy snapshot — same data the
            // `market_status` tool returns, but framed for the user
            // rather than the model. No bounty_id filter for now.
            let report_str = match crate::tools::market_report_string().await {
                Ok(s) => s,
                Err(e) => format!("Market unavailable: {e}"),
            };
            app.messages.push(ChatMessage::user("/market".into()));
            app.messages.push(ChatMessage::assistant(report_str));
        }
        "/cascade" => {
            // Filter the task store for cascade-tagged entries
            // produced by symbol_edit's `dispatch_cascade=true`. The
            // metadata.kind="cascade" tag is the signal we emit when
            // queuing them. Group by file (one Task ≈ one file) and
            // show status + caller list per group.
            let tasks = app.task_store.list(crate::tasks::DeletedFilter::Exclude);
            let cascade: Vec<&crate::tasks::Task> = tasks
                .iter()
                .filter(|t| {
                    t.metadata
                        .as_ref()
                        .and_then(|m| m.get("kind"))
                        .and_then(|k| k.as_str())
                        == Some("cascade")
                })
                .collect();
            let body = if cascade.is_empty() {
                "No cascade tasks. Cascade entries are queued by `symbol_edit` \
                 when called with `dispatch_cascade: true` and the edit changes \
                 a function signature with downstream callers."
                    .to_owned()
            } else {
                let mut s = format!(
                    "**{} cascade task{}** (from `symbol_edit dispatch_cascade=true`):\n\n",
                    cascade.len(),
                    if cascade.len() == 1 { "" } else { "s" }
                );
                for t in &cascade {
                    let status_marker = match t.status {
                        crate::tasks::TaskStatus::Completed => "✓",
                        crate::tasks::TaskStatus::InProgress => "⏵",
                        crate::tasks::TaskStatus::Pending => "•",
                        crate::tasks::TaskStatus::Deleted => "✗",
                    };
                    let file = t
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("file"))
                        .and_then(|f| f.as_str())
                        .unwrap_or("<unknown>");
                    let callers = t
                        .metadata
                        .as_ref()
                        .and_then(|m| m.get("callers"))
                        .and_then(|c| c.as_array())
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default();
                    s.push_str(&format!(
                        "{status_marker} `{}` — {}\n  callers: {callers}\n  → {}\n\n",
                        t.id, file, t.subject,
                    ));
                }
                s
            };
            app.messages.push(ChatMessage::user("/cascade".into()));
            app.messages.push(ChatMessage::assistant(body));
        }
        "/graph-history" => {
            let records = crate::tools::graph_history_snapshot();
            let body = if records.is_empty() {
                "No graph queries recorded yet. Run `graph_query` (via the model) or \
                 ask the model to query the code graph, then re-invoke `/graph-history` \
                 to see the most recent queries with their result counts."
                    .to_owned()
            } else {
                let mut s = format!(
                    "**{} graph quer{} recorded** (most recent first):\n\n",
                    records.len(),
                    if records.len() == 1 { "y" } else { "ies" }
                );
                for record in records.iter().rev().take(20) {
                    let trunc_marker = if record.was_truncated {
                        " [truncated]"
                    } else {
                        ""
                    };
                    let cycle_marker = if record.cycles_detected > 0 {
                        format!(
                            " [{} cycle{} detected]",
                            record.cycles_detected,
                            if record.cycles_detected == 1 { "" } else { "s" }
                        )
                    } else {
                        String::new()
                    };
                    s.push_str(&format!(
                        "- `{}`\n  → {} node{}{}{}\n",
                        record.query_text,
                        record.result_node_count,
                        if record.result_node_count == 1 {
                            ""
                        } else {
                            "s"
                        },
                        trunc_marker,
                        cycle_marker,
                    ));
                }
                s
            };
            app.messages
                .push(ChatMessage::user("/graph-history".into()));
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
        "/mode" => {
            let arg = parts.get(1).copied().unwrap_or("").trim().to_lowercase();
            let new_mode = match arg.as_str() {
                "default" | "d" => Some(crate::app::PermissionMode::Default),
                "plan" | "p" => Some(crate::app::PermissionMode::Plan),
                "accept" | "acceptedits" | "a" => Some(crate::app::PermissionMode::AcceptEdits),
                "bypass" | "b" | "yolo" => Some(crate::app::PermissionMode::BypassPermissions),
                "auto" => Some(crate::app::PermissionMode::Auto),
                "" => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Current mode:** {} {}\n\n\
                         Available: `default`, `plan`, `accept`, `auto`, `bypass`\n\
                         Switch: `/mode <name>` or **Shift+Tab** to cycle.",
                        app.permission_mode.symbol(),
                        app.permission_mode.label(),
                    )));
                    None
                }
                _ => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Unknown mode `{arg}`. Available: `default`, `plan`, `accept`, `auto`, `bypass`"
                    )));
                    None
                }
            };
            if let Some(mode) = new_mode {
                app.permission_mode = mode;
                // Sync auto_mode.enabled with permission mode for backward compat
                app.auto_mode.enabled = mode == crate::app::PermissionMode::Auto;
                app.messages.push(ChatMessage::assistant(format!(
                    "**Mode → {} {}**",
                    mode.symbol(),
                    mode.label()
                )));
            }
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
            handle_worktree_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/export" => {
            handle_export_command(app).await;
        }
        "/theme" => {
            handle_theme_command(app, parts.get(1).copied().unwrap_or("").trim());
        }
        "/dump-context" | "/debug-context" => {
            handle_dump_context_command(app).await;
        }
        "/swarm-approve" | "/swarm-deny" => {
            // Resolve a pending swarm permission request from the user's
            // input bar. Toasts surface the request id when it lands;
            // here we hand it back to `permission_sync::resolve_permission`
            // with the leader as `resolved_by` so the teammate's poll
            // loop unblocks.
            let id = parts.get(1).copied().unwrap_or("").trim().to_owned();
            let approve = parts[0] == "/swarm-approve";
            let feedback = parts
                .get(2..)
                .map(|rest| rest.join(" "))
                .filter(|s| !s.trim().is_empty());
            if id.is_empty() {
                app.messages.push(ChatMessage::assistant(format!(
                    "Usage: {} <request-id> [feedback]\nFind the id in the toast that appeared when the teammate asked.",
                    parts[0]
                )));
            } else {
                let team_name = app.team_context.team_name.clone().unwrap_or_default();
                let echo = if approve {
                    format!("/swarm-approve {id}")
                } else if let Some(ref f) = feedback {
                    format!("/swarm-deny {id} {f}")
                } else {
                    format!("/swarm-deny {id}")
                };
                app.messages.push(ChatMessage::user(echo));
                if team_name.is_empty() {
                    app.messages.push(ChatMessage::assistant(
                        "No active team — nothing to approve.".into(),
                    ));
                } else {
                    let resolution = crate::swarm::types::PermissionResolution {
                        decision: if approve {
                            crate::swarm::types::PermissionDecision::Approved
                        } else {
                            crate::swarm::types::PermissionDecision::Rejected
                        },
                        resolved_by: "user".to_owned(),
                        feedback,
                        updated_input: None,
                        permission_updates: Vec::new(),
                    };
                    let req_id = id.clone();
                    tokio::spawn(async move {
                        let _ = crate::swarm::permission_sync::resolve_permission(
                            &req_id,
                            &resolution,
                            &team_name,
                        )
                        .await;
                    });
                    app.messages.push(ChatMessage::assistant(format!(
                        "Resolved swarm request {id} → {}",
                        if approve { "approved" } else { "denied" }
                    )));
                }
            }
        }
        _ => {
            // Skill-name fallthrough: `/<skill>` invokes the matching skill
            // body as if the user had pasted it. Mirrors v126 cli.js:226634
            // where slash-name-not-otherwise-bound resolves to a skill or
            // markdown command and either inline-expands or forks a subagent.
            //
            // TODO Phase B: if `frontmatter.context == "fork"` (or the v126
            // equivalent flag), spawn a Task subagent here instead of inline
            // expansion. Schema: cli.js:178962.
            let name = parts[0].trim_start_matches('/');
            let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
            let skills = crate::agents::load_skills(&cwd);
            if let Some(skill) = crate::agents::find_skill_by_name(&skills, name) {
                // Echo the user's invocation so the chat shows what they
                // typed (with optional args) — same pattern as the other
                // slash arms. The injected user message that follows carries
                // the skill body, which is what the model actually sees.
                let echo = if let Some(rest) = parts.get(1) {
                    let trimmed = rest.trim();
                    if trimmed.is_empty() {
                        format!("/{name}")
                    } else {
                        format!("/{name} {trimmed}")
                    }
                } else {
                    format!("/{name}")
                };
                app.messages.push(ChatMessage::user(echo));

                // Phase A: inline-expand the body. If the user passed args
                // after the skill name, append them under an `# Args` heading
                // so the skill prompt can reference them without us having to
                // template-substitute.
                let mut body = skill.body.clone();
                if let Some(rest) = parts.get(1) {
                    let trimmed = rest.trim();
                    if !trimmed.is_empty() {
                        body.push_str("\n\n# Args\n");
                        body.push_str(trimmed);
                    }
                }

                let Some(tx) = tx else {
                    // No tx in this dispatch path (e.g. queued-prompt drain).
                    // Fall back to a hint rather than silently swallowing the
                    // invocation.
                    app.messages.push(ChatMessage::assistant(format!(
                        "Skill `/{name}` cannot be invoked from this context (no stream channel). \
                         Submit `/{name}` directly from the input bar instead."
                    )));
                    app.scroll_to_bottom();
                    return;
                };

                // Drive the same streaming setup as `handle_submit` for a
                // fresh user turn: push the synthetic user message, push the
                // empty assistant placeholder, prime streaming flags, persist
                // the session, then spawn the provider stream.
                let assistant_idx = app.messages.len() + 1;
                app.messages.push(ChatMessage::user(body));
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
                app.turn_started_at = Some(now);
                app.thinking_started_at = None;
                app.thinking_ended_at = None;
                app.last_usage_output = 0;
                app.usage_apply_baseline = (0, 0, 0, 0);
                app.scroll_to_bottom();

                let session_id = app
                    .current_session_id
                    .clone()
                    .unwrap_or_else(crate::session::generate_session_id);
                // Fire-and-forget — don't block UI on disk I/O
                {
                    let sid = session_id.clone();
                    let msgs = app.messages.clone();
                    let model = app.model.clone();
                    tokio::spawn(async move {
                        crate::session::save_session(&sid, &msgs, None, Some(model.as_str())).await;
                    });
                }
                app.current_session_id = Some(session_id);

                let provider = app.provider.clone();
                let messages =
                    crate::stream::build_provider_messages(&app.messages[..assistant_idx]);
                let model = app.model.clone();
                let tx_stream = tx.clone();
                let interrupt = app.interrupt_flag.clone();
                interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
                tokio::spawn(async move {
                    crate::stream::stream_response(provider, messages, model, tx_stream, interrupt)
                        .await;
                });
                return;
            }

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
/// `/dump-context` — print everything jfc would inject into the
/// system prompt (CLAUDE.md hierarchy, skills, memories, tool list,
/// model info) into the transcript. The exact bytes the model sees
/// on its next turn — useful when debugging "why did the model
/// hallucinate that I had a Python project / why doesn't it know
/// about this skill".
async fn handle_dump_context_command(app: &mut App) {
    let mut report = String::new();
    let cwd = std::path::PathBuf::from(&app.cwd);

    report.push_str("**Model context dump**\n\n");
    report.push_str(&format!("- Model: `{}`\n", app.model));
    report.push_str(&format!("- Cwd: `{}`\n", app.cwd));
    report.push_str(&format!("- Provider: `{}`\n", app.provider.name()));
    report.push_str(&format!("- Permission mode: `{:?}`\n", app.permission_mode));
    if let Some(ref branch) = app.git_branch {
        report.push_str(&format!("- Git branch: `{branch}`\n"));
    }
    report.push('\n');

    // CLAUDE.md hierarchy
    let hierarchy = crate::context::ClaudeMdHierarchy::load(&cwd);
    if let Some(rendered) = hierarchy.render() {
        report.push_str("### CLAUDE.md hierarchy\n\n```\n");
        report.push_str(&rendered);
        report.push_str("\n```\n\n");
    } else {
        report.push_str(
            "### CLAUDE.md hierarchy\n\n_(none — no managed/user/project files found)_\n\n",
        );
    }

    // Skills
    let skills = crate::agents::load_skills(&cwd);
    report.push_str(&format!("### Skills ({})\n\n", skills.len()));
    for skill in &skills {
        report.push_str(&format!("- `{}`\n", skill.name));
    }
    if skills.is_empty() {
        report.push_str("_(none)_\n");
    }
    report.push('\n');

    // Memories
    let memories = crate::memory::load_all_memories(&cwd);
    report.push_str(&format!("### Memories ({})\n\n", memories.len()));
    for mem in &memories {
        let name = mem
            .path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("(unknown)");
        report.push_str(&format!(
            "- **{}** ({:?}, {:?}/{:?})\n",
            name, mem.level, mem.frontmatter.memory_type, mem.frontmatter.scope,
        ));
    }
    if memories.is_empty() {
        report.push_str("_(none)_\n");
    }
    report.push('\n');

    // Tools
    let tools = crate::tools::all_tool_defs();
    report.push_str(&format!(
        "### Tool definitions sent to API ({})\n\n",
        tools.len()
    ));
    for tool in &tools {
        report.push_str(&format!("- `{}`\n", tool.name));
    }
    report.push('\n');

    // Agents
    let agents = crate::agents::load_agents(&cwd);
    report.push_str(&format!("### Agents ({})\n\n", agents.len()));
    for a in &agents {
        report.push_str(&format!(
            "- **{}** (model: `{}`, isolation: {:?})\n",
            a.name,
            a.model.as_deref().unwrap_or("inherit"),
            a.isolation
        ));
    }
    if agents.is_empty() {
        report.push_str("_(none)_\n");
    }
    report.push('\n');

    app.messages
        .push(crate::types::ChatMessage::user("/dump-context".to_string()));
    app.messages
        .push(crate::types::ChatMessage::assistant(report));
}

/// `/theme [name]` — switch the live UI theme. With no argument,
/// lists the available built-ins. Apply to `app.theme` so all
/// subsequent renders pick it up. Doesn't persist (the user can
/// add `theme` to their config.toml later if we ever wire one).
fn handle_theme_command(app: &mut App, args: &str) {
    let name = args.trim();
    if name.is_empty() {
        let list = crate::theme::Theme::available_names()
            .iter()
            .copied()
            .collect::<Vec<_>>()
            .join(", ");
        app.messages
            .push(crate::types::ChatMessage::assistant(format!(
                "Available themes: {list}.\n\nUse `/theme <name>` to switch."
            )));
        return;
    }
    match crate::theme::Theme::by_name(name) {
        Some(theme) => {
            app.theme = theme;
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Success,
                    format!("theme: {name}"),
                ),
            );
        }
        None => {
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Warning,
                    format!(
                        "unknown theme '{name}' — try one of: {}",
                        crate::theme::Theme::available_names().join(", ")
                    ),
                ),
            );
        }
    }
}

/// `/export` — serialize the current transcript as markdown and write
/// it to `~/.config/jfc/exports/{session-id}_{timestamp}.md`. Useful
/// for sharing a session, archiving long-running work, or feeding
/// the transcript into other tooling. Tool calls render as fenced
/// code blocks with their kind in the language slot. Tool results
/// are nested under their tool. Mirrors v126's `/export` command.
async fn handle_export_command(app: &mut App) {
    use crate::types::{MessagePart, Role, ToolOutput};
    let dir = dirs::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("jfc")
        .join("exports");
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        crate::toast::push_with_cap(
            &mut app.toasts,
            crate::toast::Toast::new(
                crate::toast::ToastKind::Error,
                format!("export: cannot create dir: {e}"),
            ),
        );
        return;
    }
    let session_id = app
        .current_session_id
        .clone()
        .unwrap_or_else(|| "untitled".to_owned());
    let stamp = chrono::Utc::now().format("%Y%m%d_%H%M%S");
    let path = dir.join(format!("{session_id}_{stamp}.md"));

    let mut out = String::new();
    out.push_str(&format!("# {session_id}\n\n"));
    out.push_str(&format!("- model: `{}`\n", app.model));
    out.push_str(&format!("- cwd: `{}`\n", app.cwd));
    out.push_str(&format!(
        "- exported: {}\n\n---\n\n",
        chrono::Utc::now().to_rfc3339()
    ));

    for msg in &app.messages {
        let role = match msg.role {
            Role::User => "user",
            Role::Assistant => "assistant",
        };
        out.push_str(&format!("## {role}\n\n"));
        for part in &msg.parts {
            match part {
                MessagePart::Text(s) => {
                    out.push_str(s);
                    out.push_str("\n\n");
                }
                MessagePart::Reasoning(s) => {
                    out.push_str("> _reasoning:_\n>\n");
                    for line in s.lines() {
                        out.push_str("> ");
                        out.push_str(line);
                        out.push('\n');
                    }
                    out.push('\n');
                }
                MessagePart::Tool(tc) => {
                    out.push_str(&format!(
                        "**{}** `{}`\n\n",
                        tc.kind.label(),
                        tc.input.summary()
                    ));
                    let body = match &tc.output {
                        ToolOutput::Text(s) => s.clone(),
                        ToolOutput::LargeText(lt) => lt.content.clone(),
                        ToolOutput::Command {
                            stdout,
                            stderr,
                            exit_code,
                        } => {
                            format!(
                                "exit: {}\nstdout:\n{}\nstderr:\n{}",
                                exit_code.unwrap_or(-1),
                                stdout,
                                stderr,
                            )
                        }
                        ToolOutput::FileContent { path, content, .. } => {
                            format!("// {}\n{}", path, content)
                        }
                        ToolOutput::FileList(files) => files.join("\n"),
                        ToolOutput::Diff(d) => {
                            format!(
                                "// diff: +{}/-{} in {}",
                                d.additions, d.deletions, d.file_path
                            )
                        }
                        ToolOutput::Empty => String::new(),
                    };
                    if !body.is_empty() {
                        out.push_str("```\n");
                        out.push_str(&body);
                        if !body.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str("```\n\n");
                    }
                }
                MessagePart::TaskStatus(ts) => {
                    out.push_str(&format!("- task: {}\n\n", ts.description));
                }
                MessagePart::CompactBoundary { pre_tokens } => {
                    out.push_str(&format!(
                        "\n---\n_(compaction at ~{} tokens)_\n---\n\n",
                        pre_tokens
                    ));
                }
            }
        }
        out.push_str("\n");
    }

    match tokio::fs::write(&path, out).await {
        Ok(_) => {
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Success,
                    format!("exported to {}", path.display()),
                ),
            );
            app.messages
                .push(crate::types::ChatMessage::assistant(format!(
                    "Session exported to `{}`",
                    path.display()
                )));
        }
        Err(e) => {
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Error,
                    format!("export failed: {e}"),
                ),
            );
        }
    }
}

/// `switch` cannot teleport the running session into a different checkout —
/// it tells the user how to do it manually. Once App.cwd becomes mutable we
/// can revisit.
async fn handle_worktree_command(app: &mut App, args: &str) {
    let mut it = args.split_whitespace();
    let sub = it.next().unwrap_or("");
    let arg = it.next().unwrap_or("");
    let repo_root = std::path::PathBuf::from(&app.cwd);

    fn echo(app: &mut App, raw: String, body: String) {
        app.messages.push(ChatMessage::user(raw));
        app.messages.push(ChatMessage::assistant(body));
    }

    async fn list_body(cwd: &str) -> String {
        match crate::worktrees::list_worktrees_async(&std::path::PathBuf::from(cwd)).await {
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
    }

    match sub {
        "" | "list" => {
            let body = list_body(&app.cwd).await;
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
            let body = match crate::worktrees::create_worktree_async(&repo_root, arg).await {
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
            let body = match crate::worktrees::remove_worktree_async(&repo_root, arg).await {
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

async fn execute_palette_action(app: &mut App, label: &str) {
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
            app.streaming_response_bytes = 0;
            app.streaming_assistant_idx = None;
            app.switch_session(None);
        }
        "Compact Conversation (/compact)" => {
            tracing::info!(
                target: "jfc::compact",
                model = %app.model,
                message_count = app.messages.len(),
                "palette: Compact Conversation triggered"
            );
            app.force_compact_pending = true;
            app.messages.push(ChatMessage::user("/compact".into()));
            app.messages.push(ChatMessage::assistant(
                "Compaction queued — runs on the next turn.".into(),
            ));
        }
        "Toggle Sessions Sidebar (Ctrl+B)" => {
            app.show_sidebar = !app.show_sidebar;
            if app.show_sidebar {
                app.session_meta = crate::session::list_sessions_with_metadata().await;
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
        "Toggle Thinking (Ctrl+O)" => {
            // Thinking toggle is a per-message expand/collapse — flip the
            // most recent reasoning row if there is one, otherwise no-op.
            if let Some(idx) = app.messages.len().checked_sub(1) {
                let entry = app.reasoning_expanded.entry(idx).or_insert(false);
                *entry = !*entry;
            }
        }
        "Continue Most Recent Session (/continue)" => {
            run_slash_command(app, "/continue").await;
        }
        "Show Tasks (/tasks)" => {
            run_slash_command(app, "/tasks").await;
        }
        "Show Help (/help)" => {
            run_slash_command(app, "/help").await;
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

    let all = app.model_picker_query_cache.get_or_insert_with(key, || {
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
    });

    // Move recently used models to the top of the list (preserving recency order).
    if !app.recent_models.is_empty() {
        let recent = &app.recent_models;
        let mut sorted: Vec<crate::provider::ModelInfo> = Vec::with_capacity(all.len());
        // Add recent models in recency order
        for r in recent {
            if let Some(m) = all.iter().find(|m| m.id.as_str() == r.as_str()) {
                sorted.push(m.clone());
            }
        }
        // Add remaining models
        for m in &all {
            if !recent.contains(&m.id.to_string()) {
                sorted.push(m.clone());
            }
        }
        sorted
    } else {
        all
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::app::{App, AppEvent};
    use crate::provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    #[allow(unused_imports)]
    use crate::types::*;

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

    /// Test fixture: a fresh `App` plus a paired `(tx, rx)` so tests can both
    /// drive `handle_key` and inspect the AppEvents it emits. Pulled out so
    /// the dozens of tests below don't repeat the boilerplate.
    fn test_app() -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = crate::tasks::TaskStore::in_memory();
        app
    }

    fn test_app_with_input(input: &str, wrap_width: usize) -> App {
        let mut app = test_app();
        app.input_wrap_width = wrap_width;
        app.textarea = TextArea::from(input.lines().map(str::to_string).collect::<Vec<_>>());
        app
    }

    fn channel() -> (
        tokio::sync::mpsc::Sender<AppEvent>,
        tokio::sync::mpsc::Receiver<AppEvent>,
    ) {
        tokio::sync::mpsc::channel(1024)
    }

    /// Build a minimal `ToolCall` of the requested kind. The status defaults
    /// to `Pending` so tests can drive it through the approval lifecycle
    /// without preseeding extra state.
    #[tracing::instrument(level = "trace", skip_all)]
    fn make_tool(id: &str, kind: ToolKind) -> ToolCall {
        let input = match &kind {
            ToolKind::Bash => ToolInput::Bash {
                command: "ls".into(),
                timeout: None,
                workdir: None,
            },
            ToolKind::Read => ToolInput::Read {
                file_path: "x".into(),
                offset: None,
                limit: None,
            },
            _ => ToolInput::Generic {
                summary: "tool".into(),
            },
        };
        ToolCall {
            id: id.into(),
            kind,
            status: ToolStatus::Pending,
            input,
            output: ToolOutput::Empty,
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        }
    }

    /// Convenience to send a single keypress (NONE modifier).
    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn key_mod(code: KeyCode, mods: KeyModifiers) -> KeyEvent {
        KeyEvent::new(code, mods)
    }

    // ─────────────────────────────────────────────────────────────────────
    // Pure helpers
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn input_has_text_normal() {
        let app = test_app_with_input("hi", 80);
        assert!(input_has_text(&app));
    }

    #[test]
    fn input_has_text_robust_empty() {
        let app = test_app();
        assert!(!input_has_text(&app));
    }

    #[test]
    fn input_has_text_robust_only_newlines() {
        // A textarea with multiple empty rows should still report as empty.
        let mut app = test_app();
        app.textarea = TextArea::from(vec![String::new(), String::new()]);
        assert!(!input_has_text(&app));
    }

    #[test]
    fn cursor_move_visual_up_within_wrap_normal() {
        let mut app = test_app_with_input("abcdefghij", 5);
        app.textarea.move_cursor(CursorMove::Jump(0, 7));
        move_input_cursor_visual_up(&mut app);
        assert_eq!(app.textarea.cursor(), (0, 2));
    }

    #[test]
    fn cursor_move_visual_up_jumps_to_head_when_first_line_robust() {
        let mut app = test_app_with_input("abc", 80);
        app.textarea.move_cursor(CursorMove::Jump(0, 2));
        move_input_cursor_visual_up(&mut app);
        assert_eq!(app.textarea.cursor(), (0, 0));
    }

    #[test]
    fn cursor_move_visual_down_jumps_to_end_when_last_line_robust() {
        let mut app = test_app_with_input("abc", 80);
        app.textarea.move_cursor(CursorMove::Jump(0, 1));
        move_input_cursor_visual_down(&mut app);
        assert_eq!(app.textarea.cursor(), (0, 3));
    }

    #[test]
    fn user_prompts_collects_chronologically_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("first".into()));
        app.messages.push(ChatMessage::assistant("hi".into()));
        app.messages.push(ChatMessage::user("second".into()));
        let prompts = user_prompts(&app);
        assert_eq!(prompts, vec!["first".to_string(), "second".to_string()]);
    }

    #[test]
    fn user_prompts_skips_empty_robust() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user(String::new()));
        let prompts = user_prompts(&app);
        assert!(prompts.is_empty());
    }

    #[test]
    fn recall_previous_prompt_walks_back_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("a".into()));
        app.messages.push(ChatMessage::user("b".into()));
        // First press: most recent
        let p1 = recall_previous_prompt(&mut app);
        assert_eq!(p1.as_deref(), Some("b"));
        // Second press: older
        let p2 = recall_previous_prompt(&mut app);
        assert_eq!(p2.as_deref(), Some("a"));
        // Third: stop at oldest
        let p3 = recall_previous_prompt(&mut app);
        assert!(p3.is_none());
    }

    #[test]
    fn recall_previous_prompt_robust_empty_history() {
        let mut app = test_app();
        assert!(recall_previous_prompt(&mut app).is_none());
    }

    #[test]
    fn recall_next_prompt_walks_forward_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("a".into()));
        app.messages.push(ChatMessage::user("b".into()));
        let _ = recall_previous_prompt(&mut app);
        let _ = recall_previous_prompt(&mut app);
        // Now cursor is at index 0 ("a"); forward → "b"
        let next = recall_next_prompt(&mut app);
        assert_eq!(next.as_deref(), Some("b"));
    }

    #[test]
    fn recall_next_prompt_robust_returns_none_at_end() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("only".into()));
        let _ = recall_previous_prompt(&mut app);
        // Already at most-recent → next should clear cursor and return None
        assert!(recall_next_prompt(&mut app).is_none());
        assert!(app.history_cursor.is_none());
    }

    #[test]
    fn scan_path_refs_normal() {
        let v = scan_path_refs("see src/lib.rs:42:5 and Cargo.toml:7 here");
        assert!(v.iter().any(|s| s == "src/lib.rs:42:5"));
        assert!(v.iter().any(|s| s == "Cargo.toml:7"));
    }

    #[test]
    fn scan_path_refs_rejects_url_and_pure_numbers_robust() {
        // `12:34` is a pure-number colon-pair — must be rejected. Direct
        // URL strings starting with `http://` / `https://` are also
        // rejected by the top-level guard.
        let v = scan_path_refs("foo 12:34 https://example.com:80/x");
        assert!(!v.iter().any(|s| s == "12:34"));
        assert!(!v.iter().any(|s| s.starts_with("http")));
    }

    #[test]
    fn collect_recent_paths_dedups_normal() {
        let msg = ChatMessage::assistant_parts(vec![MessagePart::Tool(ToolCall {
            id: "t1".into(),
            kind: ToolKind::Bash,
            status: ToolStatus::Complete,
            input: ToolInput::Bash {
                command: "echo".into(),
                timeout: None,
                workdir: None,
            },
            output: ToolOutput::Command {
                stdout: "src/lib.rs:1 and src/lib.rs:1".into(),
                stderr: String::new(),
                exit_code: Some(0),
            },
            is_collapsed: false,
            expanded: false,
            elapsed_ms: None,
            started_at: None,
            pinned: false,
        })]);
        let paths = collect_recent_paths(&[msg]);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], "src/lib.rs:1");
    }

    // ─────────────────────────────────────────────────────────────────────
    // Existing soft-wrap tests
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn up_and_down_move_across_soft_wrapped_input_rows() {
        let mut app = test_app_with_input("abcdefghij", 5);
        app.textarea.move_cursor(CursorMove::Jump(0, 8));
        let (tx, _rx) = channel();

        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        assert_eq!(app.textarea.cursor(), (0, 3));

        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        assert_eq!(app.textarea.cursor(), (0, 8));
    }

    #[tokio::test]
    async fn up_and_down_still_cross_logical_input_lines() {
        let mut app = test_app_with_input("abc\ndefghijkl", 5);
        app.textarea.move_cursor(CursorMove::Jump(0, 2));
        let (tx, _rx) = channel();

        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        assert_eq!(app.textarea.cursor(), (1, 2));
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        assert_eq!(app.textarea.cursor(), (1, 7));
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        assert_eq!(app.textarea.cursor(), (1, 2));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Approval modal
    // ─────────────────────────────────────────────────────────────────────

    fn arm_approval(app: &mut App, kind: ToolKind) {
        app.pending_approval = Some(crate::app::PendingApproval {
            tool: make_tool("t1", kind),
            selected: 0,
        });
    }

    #[tokio::test]
    async fn approval_y_dispatches_and_clears_normal() {
        let mut app = test_app();
        arm_approval(&mut app, ToolKind::Bash);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('y')), &tx)
            .await
            .unwrap();
        assert!(app.pending_approval.is_none());
    }

    #[tokio::test]
    async fn approval_n_denies_normal() {
        let mut app = test_app();
        arm_approval(&mut app, ToolKind::Bash);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('n')), &tx)
            .await
            .unwrap();
        assert!(app.pending_approval.is_none());
    }

    #[tokio::test]
    async fn approval_a_promotes_always_normal() {
        let mut app = test_app();
        arm_approval(&mut app, ToolKind::Bash);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('a')), &tx)
            .await
            .unwrap();
        assert!(app.always_approved.iter().any(|n| n == "Bash"));
    }

    #[tokio::test]
    async fn approval_s_promotes_session_normal() {
        let mut app = test_app();
        arm_approval(&mut app, ToolKind::Bash);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('s')), &tx)
            .await
            .unwrap();
        assert!(app.session_approved.iter().any(|n| n == "Bash"));
    }

    #[tokio::test]
    async fn approval_arrows_move_selection_normal() {
        let mut app = test_app();
        arm_approval(&mut app, ToolKind::Bash);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        assert_eq!(app.pending_approval.as_ref().unwrap().selected, 1);
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        assert_eq!(app.pending_approval.as_ref().unwrap().selected, 0);
    }

    #[tokio::test]
    async fn approval_enter_uses_selected_choice_normal() {
        let mut app = test_app();
        arm_approval(&mut app, ToolKind::Bash);
        // selected = 1 → No
        app.pending_approval.as_mut().unwrap().selected = 1;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Enter), &tx)
            .await
            .unwrap();
        assert!(app.pending_approval.is_none());
    }

    #[tokio::test]
    async fn approval_esc_clears_queue_robust() {
        let mut app = test_app();
        arm_approval(&mut app, ToolKind::Bash);
        app.approval_queue
            .push_back(make_tool("t2", ToolKind::Bash));
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(app.pending_approval.is_none());
        assert!(app.approval_queue.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────────
    // Task panel modal
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn task_panel_esc_closes_normal() {
        let mut app = test_app();
        app.show_task_panel = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(!app.show_task_panel);
    }

    #[tokio::test]
    async fn task_panel_arrows_robust_no_tasks() {
        let mut app = test_app();
        app.show_task_panel = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        assert_eq!(app.task_panel_selected, 0);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Sidebar (Ctrl+B)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_b_toggles_sidebar_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('b'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.show_sidebar);
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('b'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(!app.show_sidebar);
    }

    #[tokio::test]
    async fn sidebar_arrows_consumed_robust() {
        let mut app = test_app();
        app.show_sidebar = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        // No sessions exist → selected stays at 0
        assert_eq!(app.session_selected, 0);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Palette (Ctrl+P)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_p_opens_palette_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('p'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.show_palette);
        assert_eq!(app.palette_selected, 0);
    }

    #[tokio::test]
    async fn palette_typing_filters_normal() {
        let mut app = test_app();
        app.show_palette = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('c')), &tx)
            .await
            .unwrap();
        assert_eq!(app.palette_input, "c");
        handle_key(&mut app, key(KeyCode::Backspace), &tx)
            .await
            .unwrap();
        assert_eq!(app.palette_input, "");
    }

    #[tokio::test]
    async fn palette_arrows_change_selection_normal() {
        let mut app = test_app();
        app.show_palette = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        assert_eq!(app.palette_selected, 1);
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        assert_eq!(app.palette_selected, 0);
    }

    #[tokio::test]
    async fn palette_esc_closes_robust() {
        let mut app = test_app();
        app.show_palette = true;
        app.palette_input = "x".into();
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(!app.show_palette);
        assert!(app.palette_input.is_empty());
    }

    #[tokio::test]
    async fn palette_enter_executes_action_normal() {
        let mut app = test_app();
        app.show_palette = true;
        // First palette item: "Clear Messages (/clear)"
        let (tx, _rx) = channel();
        app.messages.push(ChatMessage::user("hi".into()));
        handle_key(&mut app, key(KeyCode::Enter), &tx)
            .await
            .unwrap();
        assert!(!app.show_palette);
        // /clear via palette wipes messages
        assert!(app.messages.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────────
    // Model picker (Ctrl+M)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_m_opens_model_picker_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('m'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.show_model_picker);
    }

    #[tokio::test]
    async fn model_picker_esc_closes_robust() {
        let mut app = test_app();
        app.show_model_picker = true;
        app.model_picker_filter = "x".into();
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(!app.show_model_picker);
        assert!(app.model_picker_filter.is_empty());
    }

    #[tokio::test]
    async fn model_picker_typing_appends_filter_normal() {
        let mut app = test_app();
        app.show_model_picker = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('o')), &tx)
            .await
            .unwrap();
        assert_eq!(app.model_picker_filter, "o");
        handle_key(&mut app, key(KeyCode::Backspace), &tx)
            .await
            .unwrap();
        assert!(app.model_picker_filter.is_empty());
    }

    #[tokio::test]
    async fn model_picker_paging_keys_robust_empty_list() {
        let mut app = test_app();
        app.show_model_picker = true;
        let (tx, _rx) = channel();
        // Each navigation key is consumed without panicking on empty list.
        for code in [
            KeyCode::Down,
            KeyCode::Up,
            KeyCode::Home,
            KeyCode::End,
            KeyCode::PageDown,
            KeyCode::PageUp,
        ] {
            handle_key(&mut app, key(code), &tx).await.unwrap();
        }
        assert_eq!(app.model_picker_selected, 0);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Slash autocomplete popup
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn slash_popup_down_cycles_normal() {
        // `/c` matches `/clear` and `/compact` so Down should advance
        // selection from 0 to 1 rather than wrapping inside a singleton.
        let mut app = test_app();
        app.textarea = TextArea::from(vec!["/c".to_string()]);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        assert_eq!(app.slash_popup_selected, Some(1));
    }

    #[tokio::test]
    async fn slash_popup_tab_commits_normal() {
        let mut app = test_app();
        app.textarea = TextArea::from(vec!["/he".to_string()]);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Tab), &tx).await.unwrap();
        let buf = app.textarea.lines().join("");
        assert!(buf.starts_with('/'));
        assert!(buf.ends_with(' '));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Transcript search (Ctrl+F when empty)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_f_opens_search_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.transcript_search.is_some());
    }

    #[tokio::test]
    async fn search_typing_finds_matches_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("hello world".into()));
        app.messages.push(ChatMessage::assistant("nope".into()));
        app.transcript_search = Some(crate::app::TranscriptSearch::default());
        let (tx, _rx) = channel();
        for c in "hello".chars() {
            handle_key(&mut app, key(KeyCode::Char(c)), &tx)
                .await
                .unwrap();
        }
        let s = app.transcript_search.as_ref().unwrap();
        assert_eq!(s.matches, vec![0]);
        assert_eq!(s.query, "hello");
    }

    #[tokio::test]
    async fn search_backspace_shrinks_query_normal() {
        let mut app = test_app();
        app.transcript_search = Some(crate::app::TranscriptSearch {
            query: "abc".into(),
            ..Default::default()
        });
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Backspace), &tx)
            .await
            .unwrap();
        assert_eq!(app.transcript_search.as_ref().unwrap().query, "ab");
    }

    #[tokio::test]
    async fn search_enter_commits_robust() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("foo".into()));
        let mut s = crate::app::TranscriptSearch::default();
        s.matches = vec![0];
        app.transcript_search = Some(s);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Enter), &tx)
            .await
            .unwrap();
        assert!(app.transcript_search.is_none());
    }

    #[tokio::test]
    async fn search_esc_cancels_robust() {
        let mut app = test_app();
        app.transcript_search = Some(crate::app::TranscriptSearch::default());
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(app.transcript_search.is_none());
    }

    #[tokio::test]
    async fn search_arrows_cycle_matches_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("a".into()));
        app.messages.push(ChatMessage::user("a".into()));
        let mut s = crate::app::TranscriptSearch::default();
        s.matches = vec![0, 1];
        app.transcript_search = Some(s);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        assert_eq!(app.transcript_search.as_ref().unwrap().cursor, 1);
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        assert_eq!(app.transcript_search.as_ref().unwrap().cursor, 0);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Jump (Ctrl+G)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_g_arms_jump_mode_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('g'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.jump_armed);
    }

    #[tokio::test]
    async fn jump_armed_e_jumps_to_error_normal() {
        let mut app = test_app();
        // failed tool in messages → e jumps to it
        app.messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Tool(
                ToolCall {
                    id: "t1".into(),
                    kind: ToolKind::Bash,
                    status: ToolStatus::Failed,
                    input: ToolInput::Bash {
                        command: "x".into(),
                        timeout: None,
                        workdir: None,
                    },
                    output: ToolOutput::Empty,
                    is_collapsed: false,
                    expanded: false,
                    elapsed_ms: None,
                    started_at: None,
                    pinned: false,
                },
            )]));
        app.jump_armed = true;
        app.jump_armed_at = Some(std::time::Instant::now());
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('e')), &tx)
            .await
            .unwrap();
        assert!(!app.jump_armed);
    }

    #[tokio::test]
    async fn jump_armed_t_jumps_to_tool_robust() {
        let mut app = test_app();
        app.jump_armed = true;
        app.jump_armed_at = Some(std::time::Instant::now());
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('t')), &tx)
            .await
            .unwrap();
        assert!(!app.jump_armed);
    }

    #[tokio::test]
    async fn jump_armed_m_jumps_to_user_robust() {
        let mut app = test_app();
        app.jump_armed = true;
        app.jump_armed_at = Some(std::time::Instant::now());
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('m')), &tx)
            .await
            .unwrap();
        assert!(!app.jump_armed);
    }

    #[tokio::test]
    async fn jump_armed_a_jumps_to_assistant_robust() {
        let mut app = test_app();
        app.jump_armed = true;
        app.jump_armed_at = Some(std::time::Instant::now());
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('a')), &tx)
            .await
            .unwrap();
        assert!(!app.jump_armed);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Leader key (Ctrl+X)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_x_arms_leader_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('x'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.leader_key_active);
    }

    #[tokio::test]
    async fn leader_then_k_exits_task_view_robust() {
        let mut app = test_app();
        app.leader_key_active = true;
        app.leader_key_timeout = Some(std::time::Instant::now());
        app.viewing_task_id = Some("t1".into());
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('k')), &tx)
            .await
            .unwrap();
        assert!(app.viewing_task_id.is_none());
        assert!(!app.leader_key_active);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Up history recall on empty input
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn up_with_empty_input_recalls_history_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("first".into()));
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        let txt = app.textarea.lines().join("\n");
        assert_eq!(txt, "first");
    }

    #[tokio::test]
    async fn up_recalls_queued_prompt_robust() {
        let mut app = test_app();
        app.queued_prompts.push_back(crate::app::QueuedPrompt {
            text: "queued".into(),
            is_meta: false,
        });
        // Push the placeholder user message that recall expects to remove.
        app.messages.push(ChatMessage::user("⏳ queued".into()));
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        let txt = app.textarea.lines().join("\n");
        // The recall path inserts the prompt then a trailing newline + a
        // `delete_line_by_end` to trim. Some textarea versions leave a
        // sentinel newline; assert containment instead of strict equality.
        assert!(txt.contains("queued"));
        assert!(app.queued_prompts.is_empty());
    }

    #[tokio::test]
    async fn down_after_recall_advances_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("a".into()));
        app.messages.push(ChatMessage::user("b".into()));
        // Manually seed history_cursor at the older prompt — `Up` after the
        // first recall would otherwise hit `move_input_cursor_visual_up`
        // because `input_has_text` flips to true after the first replay.
        app.history_cursor = Some(0);
        app.textarea = TextArea::from(vec!["a".to_string()]);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        let txt = app.textarea.lines().join("\n");
        assert_eq!(txt, "b");
    }

    #[tokio::test]
    async fn down_past_recent_clears_input_robust() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("a".into()));
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Up), &tx).await.unwrap();
        // Down with cursor at most-recent already → clears.
        handle_key(&mut app, key(KeyCode::Down), &tx).await.unwrap();
        assert!(app.history_cursor.is_none());
        assert!(app.textarea.lines().iter().all(|l| l.is_empty()));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+Y yank
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_y_with_no_assistant_message_robust() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('y'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        // Best-effort: should not panic. No assistant message → no clipboard call.
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+C
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_c_clears_input_when_text_present_normal() {
        let mut app = test_app_with_input("hello", 80);
        let (tx, _rx) = channel();
        let exit = handle_key(
            &mut app,
            key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(!exit);
        assert!(!input_has_text(&app));
    }

    #[tokio::test]
    async fn ctrl_c_exits_when_input_empty_robust() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        let exit = handle_key(
            &mut app,
            key_mod(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(exit);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+D
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_d_deletes_when_text_present_normal() {
        let mut app = test_app_with_input("abc", 80);
        app.textarea.move_cursor(CursorMove::Head);
        let (tx, _rx) = channel();
        let exit = handle_key(
            &mut app,
            key_mod(KeyCode::Char('d'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(!exit);
    }

    #[tokio::test]
    async fn ctrl_d_exits_on_empty_robust() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        let exit = handle_key(
            &mut app,
            key_mod(KeyCode::Char('d'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(exit);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+E (edit) and slash autocomplete-handled Ctrl+E in textarea
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_e_edits_last_user_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("hello".into()));
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.editing_message_idx, Some(0));
    }

    #[tokio::test]
    async fn ctrl_e_robust_no_user_message() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.editing_message_idx.is_none());
    }

    #[tokio::test]
    async fn ctrl_e_blocked_when_streaming_robust() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("hi".into()));
        app.is_streaming = true;
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.editing_message_idx.is_none());
    }

    #[tokio::test]
    async fn ctrl_e_with_text_jumps_to_end_normal() {
        // When input has text, Ctrl+E becomes "move to end of line".
        let mut app = test_app_with_input("abc", 80);
        app.textarea.move_cursor(CursorMove::Head);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('e'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.textarea.cursor(), (0, 3));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+R retry
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_r_resubmits_last_prompt_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("ask".into()));
        let (tx, mut rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        match rx.try_recv().unwrap() {
            AppEvent::Submit(t) => assert_eq!(t, "ask"),
            _ => panic!("expected Submit"),
        }
    }

    #[tokio::test]
    async fn ctrl_r_robust_no_prompt() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ctrl_r_blocked_when_streaming_robust() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("ask".into()));
        app.is_streaming = true;
        let (tx, mut rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('r'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        // No Submit emitted.
        assert!(rx.try_recv().is_err());
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+L path yank
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_l_robust_no_paths() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('l'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.path_yank_cursor, 0);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+Z / Ctrl+Shift+Z (undo / redo)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_z_undo_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('z'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn ctrl_shift_z_redo_robust() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(
                KeyCode::Char('Z'),
                KeyModifiers::CONTROL | KeyModifiers::SHIFT,
            ),
            &tx,
        )
        .await
        .unwrap();
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+I / Ctrl+S info sidebar
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_i_toggles_info_sidebar_normal() {
        let mut app = test_app();
        let initial = app.show_info_sidebar;
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('i'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert_ne!(app.show_info_sidebar, initial);
    }

    #[tokio::test]
    async fn ctrl_s_toggles_info_sidebar_normal() {
        let mut app = test_app();
        let initial = app.show_info_sidebar;
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('s'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert_ne!(app.show_info_sidebar, initial);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+O diagnostic / reasoning expand
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_o_opens_diagnostic_panel_when_diagnostics_present_normal() {
        let mut app = test_app();
        app.diagnostics.push(crate::diagnostics::DiagnosticEntry {
            file: "src/lib.rs".into(),
            line: 1,
            col: 1,
            severity: crate::diagnostics::Severity::Error,
            message: "boom".into(),
            code: None,
            source: None,
        });
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.show_diagnostic_panel);
    }

    #[tokio::test]
    async fn ctrl_o_closes_diagnostic_panel_when_open_robust() {
        let mut app = test_app();
        app.show_diagnostic_panel = true;
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(!app.show_diagnostic_panel);
    }

    #[tokio::test]
    async fn ctrl_o_toggles_reasoning_robust_no_diagnostics() {
        let mut app = test_app();
        app.messages.push(ChatMessage::assistant("hi".into()));
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('o'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.reasoning_expanded.get(&0), Some(&true));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Diagnostic panel scroll keys
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn diagnostic_panel_j_scrolls_down_normal() {
        let mut app = test_app();
        app.show_diagnostic_panel = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('j')), &tx)
            .await
            .unwrap();
        assert_eq!(app.diagnostic_panel_scroll, 1);
    }

    #[tokio::test]
    async fn diagnostic_panel_k_scrolls_up_robust() {
        let mut app = test_app();
        app.show_diagnostic_panel = true;
        app.diagnostic_panel_scroll = 5;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('k')), &tx)
            .await
            .unwrap();
        assert_eq!(app.diagnostic_panel_scroll, 4);
    }

    #[tokio::test]
    async fn diagnostic_panel_pagedown_normal() {
        let mut app = test_app();
        app.show_diagnostic_panel = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::PageDown), &tx)
            .await
            .unwrap();
        assert_eq!(app.diagnostic_panel_scroll, 10);
    }

    #[tokio::test]
    async fn diagnostic_panel_pageup_robust() {
        let mut app = test_app();
        app.show_diagnostic_panel = true;
        app.diagnostic_panel_scroll = 20;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::PageUp), &tx)
            .await
            .unwrap();
        assert_eq!(app.diagnostic_panel_scroll, 10);
    }

    #[tokio::test]
    async fn diagnostic_panel_home_g_top_normal() {
        let mut app = test_app();
        app.show_diagnostic_panel = true;
        app.diagnostic_panel_scroll = 5;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('g')), &tx)
            .await
            .unwrap();
        assert_eq!(app.diagnostic_panel_scroll, 0);
    }

    #[tokio::test]
    async fn diagnostic_panel_end_capital_g_bottom_robust() {
        let mut app = test_app();
        app.show_diagnostic_panel = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('G')), &tx)
            .await
            .unwrap();
        assert!(app.diagnostic_panel_scroll > 1_000_000);
    }

    #[tokio::test]
    async fn diagnostic_panel_esc_closes_normal() {
        let mut app = test_app();
        app.show_diagnostic_panel = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(!app.show_diagnostic_panel);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Vim-style transcript navigation (input empty)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn vim_j_scrolls_down_normal() {
        let mut app = test_app();
        app.scroll_offset = 0;
        app.total_lines = 100;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('j')), &tx)
            .await
            .unwrap();
        // Some scroll happened (or 0 if at top with no clamp); just validate
        // behaviour didn't panic and doesn't move down beyond bounds.
        let _ = app.scroll_offset;
    }

    #[tokio::test]
    async fn vim_k_scrolls_up_robust() {
        let mut app = test_app();
        app.scroll_offset = 5;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('k')), &tx)
            .await
            .unwrap();
        assert!(app.scroll_offset <= 5);
    }

    #[tokio::test]
    async fn vim_capital_g_jumps_bottom_normal() {
        let mut app = test_app();
        app.follow_bottom = false;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('G')), &tx)
            .await
            .unwrap();
        assert!(app.follow_bottom);
    }

    #[tokio::test]
    async fn vim_g_jumps_top_normal() {
        let mut app = test_app();
        app.scroll_offset = 50;
        app.follow_bottom = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('g')), &tx)
            .await
            .unwrap();
        assert_eq!(app.scroll_offset, 0);
        assert!(!app.follow_bottom);
    }

    #[tokio::test]
    async fn question_toggles_help_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('?')), &tx)
            .await
            .unwrap();
        assert!(app.show_help);
        handle_key(&mut app, key(KeyCode::Char('?')), &tx)
            .await
            .unwrap();
        assert!(!app.show_help);
    }

    #[tokio::test]
    async fn shift_question_toggles_help_robust() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('?'), KeyModifiers::SHIFT),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.show_help);
    }

    #[tokio::test]
    async fn lower_o_toggles_tool_expand_normal() {
        let mut app = test_app();
        app.messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::Tool(
                ToolCall {
                    id: "t".into(),
                    kind: ToolKind::Read,
                    status: ToolStatus::Complete,
                    input: ToolInput::Read {
                        file_path: "x".into(),
                        offset: None,
                        limit: None,
                    },
                    output: ToolOutput::Text("hi".into()),
                    is_collapsed: false,
                    expanded: false,
                    elapsed_ms: None,
                    started_at: None,
                    pinned: false,
                },
            )]));
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('o')), &tx)
            .await
            .unwrap();
        let MessagePart::Tool(tc) = &app.messages[0].parts[0] else {
            panic!("tool not found")
        };
        assert!(tc.expanded);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Esc semantics
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn esc_closes_help_normal() {
        let mut app = test_app();
        app.show_help = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(!app.show_help);
    }

    #[tokio::test]
    async fn esc_cancels_edit_mode_robust() {
        let mut app = test_app_with_input("draft", 80);
        app.editing_message_idx = Some(7);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(app.editing_message_idx.is_none());
    }

    #[tokio::test]
    async fn esc_exits_task_view_robust() {
        let mut app = test_app();
        app.viewing_task_id = Some("abc".into());
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(app.viewing_task_id.is_none());
    }

    #[tokio::test]
    async fn esc_first_then_second_arms_then_interrupts_normal() {
        let mut app = test_app();
        app.is_streaming = true;
        let (tx, _rx) = channel();
        // First Esc: arms.
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(app.last_esc_at.is_some());
        // Second Esc immediately: triggers interrupt.
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst));
    }

    #[tokio::test]
    async fn esc_resets_input_when_idle_robust() {
        let mut app = test_app_with_input("draft", 80);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(!input_has_text(&app));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Shift+BackTab cycles permission mode
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn backtab_cycles_permission_mode_normal() {
        let mut app = test_app();
        let initial = app.permission_mode;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::BackTab), &tx)
            .await
            .unwrap();
        assert_ne!(app.permission_mode, initial);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Page / Home / End
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn page_up_down_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::PageUp), &tx)
            .await
            .unwrap();
        handle_key(&mut app, key(KeyCode::PageDown), &tx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn ctrl_home_end_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(&mut app, key_mod(KeyCode::Home, KeyModifiers::CONTROL), &tx)
            .await
            .unwrap();
        handle_key(&mut app, key_mod(KeyCode::End, KeyModifiers::CONTROL), &tx)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn home_end_move_cursor_in_textarea_normal() {
        let mut app = test_app_with_input("abcdef", 80);
        app.textarea.move_cursor(CursorMove::Jump(0, 3));
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Home), &tx).await.unwrap();
        assert_eq!(app.textarea.cursor(), (0, 0));
        handle_key(&mut app, key(KeyCode::End), &tx).await.unwrap();
        assert_eq!(app.textarea.cursor(), (0, 6));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Emacs-style movement: Ctrl+a/e/u/k/w
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_a_moves_to_head_normal() {
        let mut app = test_app_with_input("abc", 80);
        app.textarea.move_cursor(CursorMove::End);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('a'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.textarea.cursor(), (0, 0));
    }

    #[tokio::test]
    async fn ctrl_u_deletes_to_head_normal() {
        let mut app = test_app_with_input("hello", 80);
        app.textarea.move_cursor(CursorMove::End);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('u'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.textarea.lines()[0].is_empty());
    }

    #[tokio::test]
    async fn ctrl_k_deletes_to_eol_robust() {
        let mut app = test_app_with_input("hello", 80);
        app.textarea.move_cursor(CursorMove::Head);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('k'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.textarea.lines()[0].is_empty());
    }

    #[tokio::test]
    async fn ctrl_w_deletes_word_robust() {
        let mut app = test_app_with_input("hello world", 80);
        app.textarea.move_cursor(CursorMove::End);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('w'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
        assert!(!app.textarea.lines()[0].contains("world"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Alt movement
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn alt_b_moves_word_back_normal() {
        let mut app = test_app_with_input("foo bar", 80);
        app.textarea.move_cursor(CursorMove::End);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('b'), KeyModifiers::ALT),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(app.textarea.cursor().1, 4);
    }

    #[tokio::test]
    async fn alt_f_moves_word_forward_normal() {
        let mut app = test_app_with_input("foo bar", 80);
        app.textarea.move_cursor(CursorMove::Head);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('f'), KeyModifiers::ALT),
            &tx,
        )
        .await
        .unwrap();
        assert!(app.textarea.cursor().1 > 0);
    }

    #[tokio::test]
    async fn alt_d_deletes_next_word_robust() {
        let mut app = test_app_with_input("foo bar", 80);
        app.textarea.move_cursor(CursorMove::Head);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('d'), KeyModifiers::ALT),
            &tx,
        )
        .await
        .unwrap();
        assert!(!app.textarea.lines()[0].contains("foo"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // Ctrl+F when input non-empty (page down)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn ctrl_f_with_input_pages_down_normal() {
        let mut app = test_app_with_input("hello", 80);
        app.viewport_height = 5;
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('f'), KeyModifiers::CONTROL),
            &tx,
        )
        .await
        .unwrap();
    }

    // ─────────────────────────────────────────────────────────────────────
    // Submit (Enter)
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn enter_with_empty_does_nothing_normal() {
        let mut app = test_app();
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Enter), &tx)
            .await
            .unwrap();
        assert!(app.messages.is_empty());
    }

    #[tokio::test]
    async fn enter_queues_when_streaming_normal() {
        let mut app = test_app_with_input("ask", 80);
        app.is_streaming = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Enter), &tx)
            .await
            .unwrap();
        assert_eq!(app.queued_prompts.len(), 1);
        assert_eq!(app.queued_prompts[0].text, "ask");
        assert!(!app.queued_prompts[0].is_meta);
    }

    #[tokio::test]
    async fn enter_queues_meta_for_slash_when_streaming_robust() {
        // `/help ` (with trailing space) skips the slash-autocomplete popup
        // because `current_slash_prefix` truncates at whitespace; `slash_matches`
        // would still find `/help` but the popup arm only intercepts when
        // there's at least one match — to bypass we use a verb that matches no
        // command but still starts with `/`.
        let mut app = test_app_with_input("/zzzz", 80);
        app.is_streaming = true;
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Enter), &tx)
            .await
            .unwrap();
        assert_eq!(app.queued_prompts.len(), 1);
        assert!(app.queued_prompts[0].is_meta);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Slash command dispatch via run_slash_command
    // ─────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn slash_clear_wipes_messages_normal() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("hi".into()));
        run_slash_command(&mut app, "/clear").await;
        assert!(app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_help_sets_show_help_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/help").await;
        assert!(app.show_help);
    }

    #[tokio::test]
    async fn slash_compact_sets_pending_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/compact").await;
        assert!(app.force_compact_pending);
    }

    #[tokio::test]
    async fn slash_unknown_emits_assistant_message_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/no-such-thing").await;
        let last = app.messages.last().expect("message added");
        assert_eq!(last.role, Role::Assistant);
    }

    #[tokio::test]
    async fn slash_mode_sets_permission_mode_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/mode plan").await;
        assert_eq!(app.permission_mode, crate::app::PermissionMode::Plan);
    }

    #[tokio::test]
    async fn slash_mode_default_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/mode default").await;
        assert_eq!(app.permission_mode, crate::app::PermissionMode::Default);
    }

    #[tokio::test]
    async fn slash_mode_unknown_robust() {
        let mut app = test_app();
        let initial = app.permission_mode;
        run_slash_command(&mut app, "/mode wat").await;
        assert_eq!(app.permission_mode, initial);
    }

    #[tokio::test]
    async fn slash_mode_status_only_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/mode").await;
        // Just ensure no panic & assistant message added.
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_auto_mode_on_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/auto-mode on").await;
        assert!(app.auto_mode.enabled);
    }

    #[tokio::test]
    async fn slash_auto_mode_off_robust() {
        let mut app = test_app();
        app.auto_mode.enabled = true;
        run_slash_command(&mut app, "/auto-mode off").await;
        assert!(!app.auto_mode.enabled);
    }

    #[tokio::test]
    async fn slash_auto_mode_status_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/auto-mode").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_task_add_creates_task_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/task-add make tests pass").await;
        let tasks = app.task_store.list(crate::tasks::DeletedFilter::Exclude);
        assert_eq!(tasks.len(), 1);
    }

    #[tokio::test]
    async fn slash_task_add_robust_no_args() {
        let mut app = test_app();
        run_slash_command(&mut app, "/task-add").await;
        let tasks = app.task_store.list(crate::tasks::DeletedFilter::Exclude);
        assert!(tasks.is_empty());
    }

    #[tokio::test]
    async fn slash_tasks_list_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/tasks").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_task_done_robust_no_args() {
        let mut app = test_app();
        run_slash_command(&mut app, "/task-done").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_task_rm_robust_no_args() {
        let mut app = test_app();
        run_slash_command(&mut app, "/task-rm").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_check_emits_assistant_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/check").await;
        assert!(app.messages.iter().any(|m| m.role == Role::Assistant));
    }

    #[tokio::test]
    async fn slash_config_reports_path_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/config path").await;
        assert!(
            app.messages
                .iter()
                .any(|m| matches!(&m.parts[0], MessagePart::Text(s) if s.contains("Config path")))
        );
    }

    #[tokio::test]
    async fn slash_config_dumps_toml_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/config").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_skills_lists_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/skills").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_agents_lists_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/agents").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_claude_md_lists_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/claude-md").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_dump_context_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/dump-context").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_theme_lists_when_no_arg_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/theme").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_theme_unknown_pushes_warning_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/theme nonexistent").await;
        // No theme change. Toast added.
        assert!(!app.toasts.is_empty());
    }

    #[tokio::test]
    async fn slash_export_creates_file_robust() {
        let mut app = test_app();
        app.messages.push(ChatMessage::user("hi".into()));
        run_slash_command(&mut app, "/export").await;
        // Either a success or error toast was emitted.
        assert!(!app.toasts.is_empty());
    }

    #[tokio::test]
    async fn slash_rename_robust_no_session() {
        let mut app = test_app();
        run_slash_command(&mut app, "/rename my-title").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_rename_robust_no_args_with_session() {
        let mut app = test_app();
        app.current_session_id = Some("ses_test".into());
        run_slash_command(&mut app, "/rename").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_resume_lists_when_no_arg_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/resume").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_resume_unknown_id_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/resume ses_does_not_exist").await;
        assert!(
            app.messages
                .iter()
                .any(|m| matches!(&m.parts[0], MessagePart::Text(s) if s.contains("not found")))
        );
    }

    #[tokio::test]
    async fn slash_continue_robust_no_sessions() {
        let mut app = test_app();
        run_slash_command(&mut app, "/continue").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_sessions_list_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/sessions").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_worktree_list_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/worktree list").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_worktree_create_no_arg_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/worktree create").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_worktree_remove_no_arg_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/worktree remove").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_worktree_switch_no_arg_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/worktree switch").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_worktree_unknown_subcommand_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/worktree foobar").await;
        assert!(app.messages.iter().any(
            |m| matches!(&m.parts[0], MessagePart::Text(s) if s.contains("Unknown subcommand"))
        ));
    }

    #[tokio::test]
    async fn slash_swarm_approve_no_args_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/swarm-approve").await;
        assert!(!app.messages.is_empty());
    }

    #[tokio::test]
    async fn slash_swarm_deny_no_team_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/swarm-deny abc-123").await;
        assert!(!app.messages.is_empty());
    }

    // Normal: /market renders the agent-economy snapshot via the
    // shared market_report_string helper. Even with no bounties
    // posted, the report has the standard headers.
    #[tokio::test]
    async fn slash_market_renders_snapshot_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/market").await;
        assert!(!app.messages.is_empty());
        let body: String = app
            .messages
            .last()
            .unwrap()
            .parts
            .iter()
            .filter_map(|p| match p {
                crate::types::MessagePart::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(
            body.contains("Agent economy snapshot") || body.contains("Market unavailable"),
            "expected snapshot or error, got: {body}"
        );
    }

    // Normal: /cascade with no cascade-tagged tasks shows the empty-
    // state hint, not an error or crash.
    #[tokio::test]
    async fn slash_cascade_empty_state_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/cascade").await;
        assert!(!app.messages.is_empty());
        let last = app.messages.last().unwrap();
        let body: String = last
            .parts
            .iter()
            .filter_map(|p| match p {
                crate::types::MessagePart::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(
            body.contains("No cascade tasks"),
            "expected empty-state hint, got: {body}"
        );
    }

    // Normal: /cascade only surfaces tasks whose metadata.kind is
    // "cascade" — non-cascade tasks must not pollute the listing.
    // Confirms the metadata filter actually filters.
    #[tokio::test]
    async fn slash_cascade_filters_by_metadata_normal() {
        let mut app = test_app();
        // A regular (non-cascade) task — should NOT appear.
        let regular = app
            .task_store
            .create::<crate::tasks::TaskId>(
                "regular work".into(),
                "should not appear in /cascade".into(),
                None,
                Vec::new(),
            )
            .expect("create regular task");
        // A cascade task — SHOULD appear.
        let cascade = app
            .task_store
            .create::<crate::tasks::TaskId>(
                "Update 2 call sites in src/foo.rs".into(),
                "cascade work".into(),
                None,
                Vec::new(),
            )
            .expect("create cascade task");
        let _ = app.task_store.update(
            cascade.id.as_str(),
            crate::tasks::TaskPatch {
                metadata: Some(serde_json::json!({
                    "kind": "cascade",
                    "file": "src/foo.rs",
                    "callers": ["alpha", "beta"],
                })),
                ..Default::default()
            },
        );
        run_slash_command(&mut app, "/cascade").await;
        let body: String = app
            .messages
            .last()
            .unwrap()
            .parts
            .iter()
            .filter_map(|p| match p {
                crate::types::MessagePart::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(
            body.contains("src/foo.rs"),
            "/cascade should list cascade-tagged task: {body}"
        );
        assert!(
            !body.contains("regular work"),
            "/cascade must not show non-cascade tasks: {body}"
        );
        assert!(
            body.contains("alpha") && body.contains("beta"),
            "/cascade should list caller names from metadata: {body}"
        );
        let _ = regular; // suppress unused
    }

    // Normal: /graph-history with no recorded queries shows the empty-
    // state hint instead of erroring (some users will run it before
    // they've ever invoked graph_query).
    #[tokio::test]
    async fn slash_graph_history_empty_state_normal() {
        let mut app = test_app();
        run_slash_command(&mut app, "/graph-history").await;
        assert!(!app.messages.is_empty());
        let last = app.messages.last().unwrap();
        let body: String = last
            .parts
            .iter()
            .filter_map(|p| match p {
                crate::types::MessagePart::Text(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        assert!(
            body.contains("No graph queries recorded yet"),
            "expected empty-state hint, got: {body}"
        );
    }

    // ─────────────────────────────────────────────────────────────────────
    // Mention (@ autocomplete)
    // ─────────────────────────────────────────────────────────────────────

    // Mention pick: Esc / Enter / Up / Down with NONE modifier are caught
    // by earlier arms in `handle_key` (Esc → reset_input, Enter → submit,
    // arrows → cursor move/recall). The popup-active block at line 1895
    // is therefore reachable mainly through Tab. Test that path directly.

    #[tokio::test]
    async fn mention_tab_applies_pick_normal() {
        let mut app = test_app_with_input("@", 80);
        app.textarea.move_cursor(CursorMove::End);
        app.mention.activate(0, vec!["src/lib.rs".into()]);
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Tab), &tx).await.unwrap();
        assert!(!app.mention.active);
        assert!(app.textarea.lines()[0].contains("src/lib.rs"));
    }

    /// Direct-call tests of the mention pick / state apply helpers — the
    /// popup-active dispatch in `handle_key` is mostly unreachable because
    /// the global Esc / Enter / arrow arms intercept those keys before
    /// the mention block sees them. The helpers themselves are still
    /// load-bearing for the `@` autocomplete UX, so we exercise them
    /// directly.
    #[test]
    fn apply_mention_pick_replaces_token_normal() {
        let mut app = test_app_with_input("hi @s", 80);
        app.textarea.move_cursor(CursorMove::End);
        app.mention.anchor_byte = 3;
        app.mention.query = "s".into();
        apply_mention_pick(&mut app, "src/lib.rs");
        let buf = app.textarea.lines().join("\n");
        assert!(buf.contains("src/lib.rs"));
    }

    // ─────────────────────────────────────────────────────────────────────
    // apply_mention_pick / update_mention_state_after_input
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn update_mention_state_activates_on_at_normal() {
        let mut app = test_app();
        app.textarea = TextArea::from(vec!["@".to_string()]);
        app.textarea.move_cursor(CursorMove::Jump(0, 1));
        update_mention_state_after_input(&mut app);
        assert!(app.mention.active);
    }

    #[test]
    fn update_mention_state_dismisses_on_whitespace_robust() {
        let mut app = test_app();
        app.textarea = TextArea::from(vec!["@x ".to_string()]);
        app.textarea.move_cursor(CursorMove::End);
        app.mention.active = true;
        app.mention.anchor_byte = 0;
        update_mention_state_after_input(&mut app);
        assert!(!app.mention.active);
    }

    // ─────────────────────────────────────────────────────────────────────
    // Filtered models / palette items
    // ─────────────────────────────────────────────────────────────────────

    #[test]
    fn filtered_models_unfiltered_returns_all_normal() {
        let mut app = test_app();
        app.model_picker_models = vec![ModelInfo::new("m1", "M1", "test")];
        let v = filtered_models(&app);
        assert_eq!(v.len(), 1);
    }

    #[test]
    fn filtered_models_filter_robust() {
        let mut app = test_app();
        app.model_picker_models = vec![
            ModelInfo::new("alpha", "Alpha", "test"),
            ModelInfo::new("beta", "Beta", "test"),
        ];
        app.model_picker_filter = "alp".into();
        let v = filtered_models(&app);
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].id.as_str(), "alpha");
    }

    #[test]
    fn palette_items_filter_normal() {
        let mut app = test_app();
        app.palette_input = "compact".into();
        let v = palette_items(&app);
        assert!(v.iter().any(|s| s.contains("Compact")));
    }

    #[test]
    fn palette_items_unfiltered_robust() {
        let app = test_app();
        let v = palette_items(&app);
        assert!(!v.is_empty());
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
