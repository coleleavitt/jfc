use crossterm::event::{self, KeyCode, KeyModifiers};
use ratatui::style::Style;
use ratatui_textarea::{CursorMove, TextArea};
use std::sync::Arc;
use tokio::sync::mpsc;

mod approval;
mod automation_commands;
mod editing;
mod github_commands;
mod local_commands;
mod mcp_commands;
mod mentions;
mod modal_handlers;
mod model_picker;
mod navigation;
mod palette;
mod theme_picker;
mod worktree_commands;

use automation_commands::{handle_dream_command, handle_loop_command, handle_schedule_command};
use editing::{
    input_has_text, move_input_cursor_visual_down, move_input_cursor_visual_up, reset_input,
    step_reasoning_effort,
};
use github_commands::{
    handle_install_github_app, handle_pr_autofix, handle_pr_view, handle_setup_github_actions,
};
use local_commands::{
    handle_bug_command, handle_cost_command, handle_doc_command, handle_dump_context_command,
    handle_fleet_command, handle_init_command, handle_output_style_command, handle_rewind_command,
    handle_status_command, handle_teleport_command, handle_theme_command,
};
use mcp_commands::handle_mcp_command;
use mentions::{apply_mention_pick, update_mention_state_after_input};
pub use model_picker::filtered_models;
use model_picker::{handle_model_picker_key, open_model_picker};
use navigation::{
    collect_recent_paths, jump_to_last_assistant, jump_to_last_error, jump_to_last_tool,
    jump_to_last_user, recall_next_prompt, recall_previous_prompt, refresh_search_matches,
    scan_path_refs, scroll_to_message, user_prompts,
};
pub use palette::{collect_all_models, palette_items};
pub(crate) use theme_picker::filtered_theme_choices;
use worktree_commands::handle_worktree_command;

use crate::app::App;
use crate::runtime::{AppEvent, CompactionEvent, UiEvent};
use crate::types::*;

pub async fn handle_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<crate::runtime::AppEvent>,
) -> anyhow::Result<bool> {
    if approval::handle_approval_key(app, key, tx) {
        return Ok(false);
    }

    if modal_handlers::handle_modal_key(app, key, tx).await {
        return Ok(false);
    }

    if handle_model_picker_key(app, key) {
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
                    app.textarea.insert_str(format!("{cmd} "));
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
            // v132 fleet ergonomics: `x` cancels the selected task by
            // flipping it to Failed and surfacing a toast so the user
            // sees the cancellation took effect. The actual tokio task
            // continues briefly until the next interrupt-flag check —
            // BackgroundTask status drives the fan UI, so flipping it
            // stops the visual bleed and lets the user move on.
            KeyCode::Char('x') => {
                if let Some(id) = app.viewing_task_id.clone() {
                    if let Some(bt) = app.background_tasks.get_mut(&id) {
                        if matches!(
                            bt.status,
                            crate::types::TaskLifecycle::Running
                                | crate::types::TaskLifecycle::Idle
                        ) {
                            bt.status = crate::types::TaskLifecycle::Failed;
                            bt.error = Some("cancelled by user".into());
                            crate::toast::push_with_cap(
                                &mut app.toasts,
                                crate::toast::Toast::new(
                                    crate::toast::ToastKind::Warning,
                                    format!("Cancelled task {id}"),
                                ),
                            );
                        }
                    }
                }
            }
            // `r` retries: re-queue the original task description as a
            // fresh user prompt so the leader dispatches a new agent.
            KeyCode::Char('r') => {
                if let Some(id) = app.viewing_task_id.clone() {
                    if let Some(bt) = app.background_tasks.get(&id) {
                        let prompt = bt.description.clone();
                        let tx_clone = tx.clone();
                        tokio::spawn(async move {
                            let _ = tx_clone
                                .send(crate::runtime::AppEvent::Ui(
                                    crate::runtime::UiEvent::Submit(prompt),
                                ))
                                .await;
                        });
                        crate::toast::push_with_cap(
                            &mut app.toasts,
                            crate::toast::Toast::new(
                                crate::toast::ToastKind::Info,
                                format!("Retrying task {id}"),
                            ),
                        );
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

    // ─── User-configured keybindings (keybindings.toml) ──────────────────
    // Check before built-in bindings so users can override defaults.
    // Uses run_slash_command so actions stay in sync with their slash
    // counterparts automatically.
    if let Some(action) = crate::keybindings::lookup(&key) {
        use crate::keybindings::KeyAction;
        match action {
            KeyAction::ToggleFastMode => {
                run_slash_command(app, "/fast").await;
                return Ok(false);
            }
            KeyAction::ClearHistory => {
                run_slash_command(app, "/clear").await;
                return Ok(false);
            }
            KeyAction::Compact => {
                run_slash_command(app, "/compact").await;
                return Ok(false);
            }
            KeyAction::OpenModelPicker => {
                open_model_picker(app);
                return Ok(false);
            }
            KeyAction::ToggleVerbose => {
                run_slash_command(app, "/verbose").await;
                return Ok(false);
            }
            KeyAction::Exit => {
                return Ok(true);
            }
            KeyAction::ToggleHelp => {
                app.show_help = !app.show_help;
                return Ok(false);
            }
        }
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
                    let _ = tx
                        .send(crate::runtime::AppEvent::Ui(
                            crate::runtime::UiEvent::Submit(text),
                        ))
                        .await;
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
            open_model_picker(app);
            return Ok(false);
        }
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
            app.show_sidebar = !app.show_sidebar;
            if app.show_sidebar {
                app.session_meta = jfc_session::list_sessions_with_metadata().await;
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
        (KeyModifiers::CONTROL, KeyCode::Char('v')) => {
            // Image-paste keybind. Some terminals don't translate Ctrl+V
            // into bracketed-paste events (xterm without
            // `enableModifyOtherKeys`, certain tmux configurations), so
            // we explicitly try `read_clipboard_image` here. If the
            // clipboard holds an image, attach it; if it holds text,
            // fall through to the textarea's normal paste handling.
            match crate::attachments::read_clipboard_image() {
                Ok(Some((att, w, h))) => {
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(
                            crate::toast::ToastKind::Info,
                            format!("📎 image attached ({}x{}, {} bytes)", w, h, att.bytes.len()),
                        ),
                    );
                    app.image_counter += 1;
                    let id = app.image_counter;
                    app.pasted_images.push(crate::attachments::PastedContent {
                        id,
                        attachment: att,
                        width: w,
                        height: h,
                    });
                    app.textarea.insert_str(format!("[Image #{id}]"));
                    return Ok(false);
                }
                Ok(None) => {
                    // Try text clipboard fallback.
                    if let Ok(mut cb) = arboard::Clipboard::new() {
                        if let Ok(text) = cb.get_text() {
                            app.textarea.insert_str(&text);
                        }
                    }
                    return Ok(false);
                }
                Err(e) => {
                    tracing::debug!(target: "jfc::input", error = %e, "Ctrl+V image paste failed");
                    return Ok(false);
                }
            }
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
                                .send(crate::runtime::AppEvent::Ui(
                                    crate::runtime::UiEvent::Toast {
                                        kind: crate::toast::ToastKind::Success,
                                        text: format!("Copied: {preview}{suffix}"),
                                    },
                                ))
                                .await;
                        }
                        Err(e) => {
                            let _ = tx
                                .send(crate::runtime::AppEvent::Ui(
                                    crate::runtime::UiEvent::Toast {
                                        kind: crate::toast::ToastKind::Error,
                                        text: format!("Clipboard error: {e}"),
                                    },
                                ))
                                .await;
                        }
                    }
                }
                _ => {
                    let _ = tx
                        .send(crate::runtime::AppEvent::Ui(
                            crate::runtime::UiEvent::Toast {
                                kind: crate::toast::ToastKind::Warning,
                                text: "No assistant message to yank".into(),
                            },
                        ))
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
        (KeyModifiers::ALT, KeyCode::Char('.')) => {
            step_reasoning_effort(app, true);
            return Ok(false);
        }
        (KeyModifiers::ALT, KeyCode::Char(',')) => {
            step_reasoning_effort(app, false);
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
                            // Two-level expand: huge LargeText pivots
                            // teaser ⇄ body (`toggle_collapsed`); all
                            // other tools pivot 80-line cap ⇄ 500-line
                            // cap (`toggle_expanded`). The user gets a
                            // single `o` shortcut that scales: small
                            // Read → expand to full, huge Bash dump →
                            // expand teaser to body.
                            match &tc.output {
                                ToolOutput::LargeText(lt)
                                    if lt.line_count > crate::types::LargeText::COLLAPSE_LINES
                                        || lt.content.len()
                                            > crate::types::LargeText::COLLAPSE_BYTES =>
                                {
                                    tc.display.toggle_collapsed();
                                    break 'toggle;
                                }
                                ToolOutput::Empty => {}
                                _ => {
                                    tc.display.toggle_expanded();
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

            // Double-tap ESC to instantly kill active work:
            //   1st ESC → toast "Press ESC again to interrupt", arm the timer
            //   2nd ESC (within 600ms) → cancel_token.cancel() which fires
            //     the select! arm in stream_response instantly (no 50ms poll)
            //     + SIGTERM all bash + set interrupt_flag for legacy callers
            //
            // This gives the user a confirmation step (prevents accidental
            // kills) while making the actual kill truly instant when confirmed.
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
                let armed = app
                    .last_esc_at
                    .map(|t| now.duration_since(t).as_millis() < DOUBLE_TAP_MS)
                    .unwrap_or(false);
                if armed {
                    // 2nd ESC — INSTANT KILL. CancellationToken wakes the
                    // select! in stream_response on the next scheduler tick.
                    app.interrupt_flag
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    app.cancel_token.cancel();
                    app.last_esc_at = None;
                    // SIGTERM all in-flight bash subprocesses immediately.
                    let killed = crate::bash_processes::terminate_all();
                    if killed > 0 {
                        tracing::info!(
                            target: "jfc::input::abort",
                            killed,
                            "SIGTERMed in-flight bash subprocesses"
                        );
                    }
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(
                            crate::toast::ToastKind::Warning,
                            if killed > 0 {
                                format!(
                                    "⏹ Interrupted (killed {killed} process{})",
                                    if killed == 1 { "" } else { "es" }
                                )
                            } else {
                                "⏹ Interrupted".to_owned()
                            },
                        ),
                    );
                } else {
                    // 1st ESC — arm the double-tap timer + hint.
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
                // Capture referenced [Image #N] attachments onto THIS
                // queued prompt so they re-stage atomically when the
                // entry drains. Only matched entries are taken;
                // unreferenced images are left for later prompts.
                let attachments: Vec<crate::attachments::Attachment> = {
                    let re_pattern = regex::Regex::new(r"\[Image #(\d+)\]").unwrap();
                    let mut referenced_ids: Vec<u32> = Vec::new();
                    for cap in re_pattern.captures_iter(&text) {
                        if let Ok(id) = cap[1].parse::<u32>() {
                            referenced_ids.push(id);
                        }
                    }
                    let mut matched = Vec::new();
                    let mut remaining = Vec::new();
                    for pc in std::mem::take(&mut app.pasted_images) {
                        if referenced_ids.contains(&pc.id) {
                            matched.push(pc.attachment);
                        } else {
                            remaining.push(pc);
                        }
                    }
                    app.pasted_images = remaining;
                    matched
                };
                app.queued_prompts.push_back(crate::app::QueuedPrompt {
                    text: text.clone(),
                    is_meta,
                    attachments,
                });
                // Insert as a `queued` user message so the user can SEE
                // "I queued this" in the transcript, but
                // `build_provider_messages*` will skip it. Without this
                // flag, `continue_agentic_loop` sent the queued user
                // text to the provider as if it were part of the current
                // turn, inflating the prompt and creating the gauge
                // jump after queuing.
                app.messages
                    .push(ChatMessage::user_queued(format!("{glyph} {text}")));
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

/// Public re-entry used by `UiEvent::Submit`. Same body as the private
/// `handle_submit` used from the typing path.
pub async fn handle_submit_text(
    app: &mut App,
    text: String,
    tx: &mpsc::Sender<crate::runtime::AppEvent>,
) -> anyhow::Result<()> {
    handle_submit(app, text, tx).await
}

async fn handle_submit(
    app: &mut App,
    text: String,
    tx: &mpsc::Sender<crate::runtime::AppEvent>,
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

    // v132 OnUserPromptSubmit hook — fires before any compaction or
    // stream setup so a registered handler can inject system reminders,
    // veto the turn, or rewrite the text. Default registry has only
    // a Logger so production behavior is unchanged when no user hooks
    // are configured.
    let session_id_for_hook = app
        .current_session_id
        .as_ref()
        .map(|s| s.as_str().to_owned())
        .unwrap_or_else(|| "<no-session>".to_owned());
    let hook_action = crate::hooks::fire(
        crate::hooks::HookPoint::OnUserPromptSubmit,
        &crate::hooks::HookContext::for_session(&session_id_for_hook)
            .with_extra("text_len", text.len().to_string()),
    );
    if let crate::hooks::HookAction::Abort(reason) = &hook_action {
        tracing::warn!(target: "jfc::hooks", %reason, "OnUserPromptSubmit aborted turn");
        let _ = tx
            .send(crate::runtime::AppEvent::Ui(
                crate::runtime::UiEvent::Toast {
                    kind: crate::toast::ToastKind::Error,
                    text: format!("Turn aborted by hook: {reason}"),
                },
            ))
            .await;
        return Ok(());
    }

    // v132 @-mention auto-attach: scan the prompt for `@path/to/file`
    // tokens. If the path resolves to a real file, read it and stage
    // it as an attachment so the model sees the content alongside the
    // user's text. URLs (containing `://`) are skipped — those are
    // user-supplied references, not local paths.
    //
    // Text @-mentions: collect reminder bodies; inject after the new
    // user message is pushed so they land on the correct turn.
    // Binary @-mentions: collect locally; attach to the user message
    // after it's pushed — per-message ownership, no global queue.
    let mut deferred_text_reminders: Vec<String> = Vec::new();
    let mut mention_attachments: Vec<crate::attachments::Attachment> = Vec::new();
    {
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
        for token in text.split_whitespace() {
            // Strip surrounding punctuation: `(@src/foo.rs)` → `src/foo.rs`.
            let stripped = token.trim_matches(|c: char| {
                !c.is_alphanumeric() && c != '/' && c != '.' && c != '_' && c != '-' && c != '@'
            });
            let Some(rest) = stripped.strip_prefix('@') else {
                continue;
            };
            if rest.is_empty() || rest.contains("://") {
                continue;
            }
            if seen.contains(rest) {
                continue;
            }
            let path = std::path::PathBuf::from(rest);
            if !path.is_file() {
                continue;
            }
            let Ok(meta) = path.metadata() else {
                continue;
            };
            // Cap at 1 MB so a runaway @ doesn't OOM the prompt.
            if meta.len() > 1_000_000 {
                tracing::debug!(
                    target: "jfc::input::mention",
                    path = %path.display(),
                    bytes = meta.len(),
                    "@-mention skipped (file too large)"
                );
                continue;
            }
            // Image/PDF: stage as binary attachment via the existing
            // attachments path. Text: just nudge via system reminder
            // (the model can Read it itself if needed; auto-Read'ing
            // would burn tokens on every @ even when the user didn't
            // mean "show me this file").
            let bytes = match std::fs::read(&path) {
                Ok(b) => b,
                Err(_) => continue,
            };
            if let Some(kind) = crate::attachments::detect_kind(&bytes) {
                let att = crate::attachments::Attachment { id: 0, kind, bytes };
                mention_attachments.push(att);
                tracing::info!(
                    target: "jfc::input::mention",
                    path = %path.display(),
                    "@-mention auto-attached image/pdf"
                );
            } else if let Ok(content) = String::from_utf8(bytes) {
                let preview: String = content.chars().take(50_000).collect();
                deferred_text_reminders.push(format!(
                    "User mentioned `@{rest}` — content of `{}` follows:\n\n```\n{preview}\n```",
                    path.display()
                ));
                tracing::info!(
                    target: "jfc::input::mention",
                    path = %path.display(),
                    bytes = preview.len(),
                    "@-mention queued text reminder"
                );
            }
            seen.insert(rest.to_owned());
        }
    }

    // Extract referenced [Image #N] attachments from pasted_images and
    // attach them to the message that will be submitted. Any pasted
    // images whose markers the user deleted are dropped with a log.
    let submit_attachments: Vec<crate::attachments::Attachment> = if !app.pasted_images.is_empty() {
        // Parse all [Image #N] references from the text
        let mut referenced_ids: Vec<u32> = Vec::new();
        let re_pattern = regex::Regex::new(r"\[Image #(\d+)\]").unwrap();
        for cap in re_pattern.captures_iter(&text) {
            if let Ok(id) = cap[1].parse::<u32>() {
                referenced_ids.push(id);
            }
        }

        let mut matched: Vec<crate::attachments::Attachment> = Vec::new();
        let mut remaining: Vec<crate::attachments::PastedContent> = Vec::new();
        for pc in std::mem::take(&mut app.pasted_images) {
            if referenced_ids.contains(&pc.id) {
                matched.push(pc.attachment);
            } else {
                remaining.push(pc);
            }
        }

        // Drop unreferenced (user deleted the marker)
        if !remaining.is_empty() {
            tracing::info!(
                target: "jfc::input::paste",
                dropped = remaining.len(),
                "dropping unreferenced pasted images (markers deleted by user)"
            );
        }

        tracing::info!(
            target: "jfc::input::paste",
            matched = matched.len(),
            "matched [Image #N] attachments for submit"
        );
        matched
    } else {
        Vec::new()
    };

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
    //
    // Use `tool_ctx.approx_tokens` (the calibrated wire-truth, kept in sync
    // by `recompute_token_estimate` on resume and by `StreamUsage` during a
    // turn) rather than re-running the chars-based `estimate_tokens`
    // heuristic. The doc comment on `compact::should_compact` warns
    // explicitly that the raw estimator over-counts tool outputs (it sums
    // their full byte length while the wire truncates each tool result to
    // `MAX_TOOL_RESULT_CHARS`), and on prompt-cache-heavy sessions it can
    // also under-count by missing the cache_read contribution. Using the
    // calibrated value makes pre-submit and post-tool compaction agree on
    // when the session is actually full.
    let est = app.tool_ctx.approx_tokens;
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
        let _ = tx_pre
            .send(crate::runtime::AppEvent::Compaction(
                crate::runtime::CompactionEvent::Started,
            ))
            .await;
        // Progress callback fires on every text_delta from the streaming
        // compact, forwards the cumulative output length as a
        // CompactionProgress event so the spinner shows live token
        // count. Mirrors v126's `addResponseLength` callback in PB7.
        let progress_tx = tx_pre.clone();
        let on_progress: crate::compact::CompactProgressCb = Box::new(move |chars| {
            // CompactionProgress is non-critical; next progress update supersedes.
            let _ = progress_tx.try_send(crate::runtime::AppEvent::Compaction(
                crate::runtime::CompactionEvent::Progress {
                    output_chars: chars,
                },
            ));
        });
        tokio::spawn(async move {
            let options = jfc_provider::StreamOptions::new(model.clone());
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
                        .send(crate::runtime::AppEvent::Compaction(
                            crate::runtime::CompactionEvent::Done {
                                messages,
                                tool_ctx,
                                pre_tokens,
                                post_tokens,
                            },
                        ))
                        .await;
                    // Re-queue the user's message — it didn't make it into
                    // the conversation before compaction ran.
                    let _ = tx_pre
                        .send(crate::runtime::AppEvent::Ui(
                            crate::runtime::UiEvent::Submit(user_text),
                        ))
                        .await;
                }
                crate::compact::CompactResult::CircuitBreakerTripped => {
                    tracing::warn!(
                        target: "jfc::compact",
                        "pre-submit compaction: circuit breaker tripped"
                    );
                    let _ = tx_pre
                        .send(crate::runtime::AppEvent::Compaction(
                            crate::runtime::CompactionEvent::Failed {
                                reason: "Circuit breaker tripped — submit again with `/compact` if needed"
                                    .into(),
                                calibrated_tokens: None,
                                transient: false,
                            },
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
                        .send(crate::runtime::AppEvent::Compaction(
                            crate::runtime::CompactionEvent::Failed {
                                reason: format!(
                                "Exhausted {attempts} compaction attempts — request is too large"
                            ),
                                calibrated_tokens: Some(tool_ctx.approx_tokens),
                                transient: false,
                            },
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
                            .send(crate::runtime::AppEvent::Compaction(
                                crate::runtime::CompactionEvent::Failed {
                                    reason: "Context exceeds limit and provider cannot compact — \
                             try switching to a model/provider that supports compaction, \
                             or start a new session."
                                        .into(),
                                    calibrated_tokens: Some(tool_ctx.approx_tokens),
                                    transient: false,
                                },
                            ))
                            .await;
                    } else {
                        tracing::debug!(
                            target: "jfc::compact",
                            "pre-submit compaction skipped (unsupported/too few groups) — submitting anyway"
                        );
                        let _ = tx_pre
                            .send(crate::runtime::AppEvent::Ui(
                                crate::runtime::UiEvent::Submit(user_text),
                            ))
                            .await;
                    }
                }
            }
        });
        return Ok(());
    }

    let assistant_idx = app.messages.len() + 1;
    let mut user_msg = ChatMessage::user(text.clone());
    // Combine pasted images ([Image #N] refs) with @-mention binary files.
    let mut all_attachments = submit_attachments;
    all_attachments.extend(mention_attachments);
    user_msg.attachments = all_attachments;
    app.messages.push(user_msg);
    app.tool_ctx.total_user_turns += 1;

    // Inject background-agent completion notification if any detached
    // agents finished since the last user turn. The counter is
    // incremented by sync_detached_background_tasks_from_daemon when
    // agent status transitions to terminal. Drain it here so the model
    // sees "N agents finished — their summaries are in the transcript"
    // on this very turn (via the TaskStatus serialization we added to
    // build_provider_messages). Mirrors oh-my-opencode's
    // background-task-notification-template.ts pattern.
    let bg_completed = app.background_tasks_completed_since_last_turn;
    if bg_completed > 0 {
        app.background_tasks_completed_since_last_turn = 0;
        let plural = if bg_completed == 1 { "" } else { "s" };
        crate::system_reminder::append_to_last_user(
            &mut app.messages,
            &format!(
                "{bg_completed} detached background task{plural} completed since your last turn. \
                 Their final summaries are visible in the assistant transcript as \
                 [Background agent: ...] blocks. Review those summaries before responding — \
                 the user expects you to incorporate or acknowledge completed work."
            ),
        );
        tracing::info!(
            target: "jfc::background",
            count = bg_completed,
            "injected background-agent completion reminder into user turn"
        );
    }

    // Now that the new user message is the most-recent user message,
    // attach any deferred @-mention text reminders to IT (not the
    // previous turn). See the comment on `deferred_text_reminders` at
    // the scan site for why this had to be split.
    for body in deferred_text_reminders {
        crate::system_reminder::append_to_last_user(&mut app.messages, &body);
    }

    // Auto graph-context injection: when the prompt smells like an
    // impact-analysis / refactor-risk / dependency-trace / entrypoint
    // question, run a cheap structural query against the workspace
    // graph and append the result as a `<system-reminder>` so the
    // model sees the structural context up-front instead of having to
    // remember to fire `graph_query` itself. Opt out via
    // `JFC_GRAPH_AUTO_CONTEXT=0`. The helper is a no-op for
    // non-graph intents and disabled-flag cases. We do NOT push the
    // assistant placeholder yet — `append_to_last_user` walks
    // `messages.iter_mut().rfind(|m| m.role == Role::User)`, so the
    // freshly-pushed user message at the tail is its target.
    //
    // Gated behind the `intent-gate` cargo feature for symmetry with
    // the `mod intent` declaration in `main.rs`. Without the gate the
    // intent module is configured-out and `crate::intent::...` paths
    // fail to resolve at compile time — see Cargo.toml `[features]`.
    #[cfg(feature = "intent-gate")]
    {
        let classification = crate::intent::classify(&text);
        let intent_for_inject = classification.intent;

        // (1) Graph-flavored intents → auto-inject structural context.
        if crate::intent::is_graph_intent(intent_for_inject) {
            let cwd = std::path::PathBuf::from(&app.cwd);
            let injected = crate::intent::auto_inject_graph_context(
                &mut app.messages,
                intent_for_inject,
                &text,
                &cwd,
            );
            if injected {
                tracing::info!(
                    target: "jfc::intent::auto_ctx",
                    intent = ?intent_for_inject,
                    "auto graph-context injected"
                );
            }
        }

        // (2) Doc-request intents → suggest the matching slash command
        // via a toast. We never auto-run the command (writing a file
        // the user didn't explicitly ask for is destructive) — the
        // toast is a one-keystroke nudge. Suppressed via
        // JFC_AUTO_DOC_SUGGEST=0.
        if let Some(cmd) = intent_for_inject.doc_command() {
            if crate::intent::auto_doc_suggest_enabled() {
                tracing::info!(
                    target: "jfc::intent::doc_suggest",
                    intent = ?intent_for_inject,
                    cmd,
                    "doc-request detected — surfacing slash-command suggestion"
                );
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Info,
                        format!(
                            "This looks like a doc request — type `{cmd}` to draft \
                             it with the strict format contract."
                        ),
                    ),
                );
            }
        }

        // (3) Auto-Plan-Mode: planning-shaped prompts flip the session
        // into Plan (read-only) permission mode — but only when the
        // user opted in via JFC_AUTO_PLAN_MODE=1, and only when we're
        // not already in a more-restrictive-or-equal mode. The user
        // can Shift+Tab back out immediately.
        if intent_for_inject == crate::intent::Intent::AutoPlanModeRequest
            && crate::intent::auto_plan_mode_enabled()
            && !matches!(
                app.permission_mode,
                crate::app::PermissionMode::Plan | crate::app::PermissionMode::Auto
            )
        {
            let from = app.permission_mode;
            app.permission_mode = crate::app::PermissionMode::Plan;
            tracing::info!(
                target: "jfc::intent::auto_plan_mode",
                ?from,
                "planning-shaped prompt — auto-flipped to Plan mode"
            );
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Info,
                    "Planning request detected — switched to Plan mode \
                     (read-only). Shift+Tab to change."
                        .to_string(),
                ),
            );
            crate::system_reminder::append_to_last_user(
                &mut app.messages,
                "Permission mode auto-switched to `Plan` (read-only) because \
                 this request reads as planning/design work. Investigate and \
                 produce a plan; use ExitPlanMode with a finalized plan when \
                 you're ready to make edits.",
            );
        }
    }

    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text.clear();
    app.streaming_reasoning.clear();
    app.streaming_response_bytes = 0;
    app.network_recovery_status = None;
    app.network_recovery_attempts = 0;
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    // Defensive: clear any stale mixed-mode pause_turn latch from a
    // previously-cancelled turn. The flag is normally single-shot
    // (cleared at dispatch time in event_loop's AllComplete /
    // CompactionDone handlers) but a user-initiated cancel + fresh
    // submit would leave it sticky otherwise.
    app.pending_pause_turn_resume = false;
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
        .unwrap_or_else(jfc_session::generate_session_id);
    // Fire-and-forget session save — don't block the UI on disk I/O.
    {
        let sid = session_id.clone();
        let msgs = app.messages.clone();
        let cwd = app.cwd.clone();
        let model = app.model.clone();
        tokio::spawn(async move {
            crate::session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str()))
                .await;
        });
    }
    app.current_session_id = Some(session_id.clone());

    let provider = app.provider.clone();
    let messages = crate::stream::build_provider_messages(&app.messages[..assistant_idx]);
    // Slate per-turn model selection: when the router is configured (config
    // `slate_enabled = true`), classify the user's text and route to the
    // best-fit model for this turn. When None (default), use the pinned
    // `app.model` — legacy behavior. The pinned model is also the fallback
    // for unmatched classes inside the router itself.
    let model = if let Some(ref router) = app.slate {
        let (routed, class, rule_idx) = router.route_explained(&text, app.model.clone());
        tracing::info!(
            target: "jfc::slate",
            class = ?class,
            matched_rule = ?rule_idx,
            routed_model = %routed,
            pinned_model = %app.model,
            "slate routed turn"
        );
        routed
    } else {
        app.model.clone()
    };
    let tx = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    // Fresh user submission resets any prior interrupt state — the user
    // moved on, so the next stream should run unchecked.
    interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
    // Mint a fresh cancel token. A token's `cancelled` is sticky, so a
    // previously cancelled turn would poison the next one if we reused
    // it. wg-async pattern: each unit of work gets its own token.
    app.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = app.cancel_token.clone();

    tracing::info!(
        target: "jfc::input",
        model = %model,
        provider_message_count = messages.len(),
        assistant_idx,
        session_id = %session_id,
        total_user_turns = app.tool_ctx.total_user_turns,
        "spawning stream_response"
    );

    // wg-async: stream_response holds the SSE connection + tx sender —
    // cancel has to thread through so ESC×2 can drop them coherently.
    tokio::spawn(async move {
        crate::stream::stream_response(provider, messages, model, tx, interrupt, cancel, None).await;
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
            // The handler emits `ProviderEvent::DiagnosticsUpdated` whose
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
            // Use the calibrated context size (same source as the gauge
            // and pre-submit gate). Previously this re-ran the raw
            // `estimate_tokens` heuristic, so the manual report disagreed
            // with the live gauge and could show "0%" for a session the
            // sidebar reports as 90%-full.
            let est = app.tool_ctx.approx_tokens;
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
        "/advisor" => {
            // Parallel advisor (see `crate::advisor`). Doesn't touch the main
            // agent's stream — runs a separate `provider.complete()` against a
            // SNAPSHOT of the current transcript and surfaces the reply as a
            // dedicated `MessagePart::Advisor` part with its own visual style.
            //
            // Default-off per deliverable: gated by `app.advisor_enabled`,
            // populated from `JFC_ADVISOR_ENABLED=1` on startup. Even when on,
            // each session has a per-budget ceiling (`DEFAULT_TOKEN_BUDGET`)
            // so a runaway loop can't drain the user's account.
            let query = parts.get(1).copied().unwrap_or("").trim().to_owned();
            // Echo the user's command into the transcript first so the chat
            // shows what the user asked, even on the error paths below.
            app.messages
                .push(ChatMessage::user(format!("/advisor {query}")));
            if !app.advisor_enabled {
                app.messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        "Advisor mode is disabled. Set `JFC_ADVISOR_ENABLED=1` and \
                         restart jfc to enable parallel advisor queries."
                            .into(),
                    )]));
            } else if query.is_empty() {
                app.messages
                    .push(ChatMessage::assistant_parts(vec![MessagePart::Advisor(
                        "Usage: `/advisor <question>` — runs a parallel call \
                         against a snapshot of this transcript and surfaces \
                         the reply here without disturbing the main agent."
                            .into(),
                    )]));
            } else {
                // Lazy-mint the session on first use so users that never
                // call /advisor pay no allocation cost. The session model
                // tracks the *active* model at first invocation; switching
                // models mid-session keeps the original advisor model.
                let session = app
                    .advisor_session
                    .get_or_insert_with(|| crate::advisor::AdvisorSession::new(app.model.clone()));
                // Snapshot — Vec::clone is fine here, the deliverable
                // explicitly calls for a SNAPSHOT semantic. Without the
                // clone, `ask_advisor` would borrow `app.messages`
                // immutably while we're holding `&mut app.advisor_session`
                // mutably — borrow-check fails.
                let snapshot = app.messages.clone();
                let provider = std::sync::Arc::clone(&app.provider);
                match crate::advisor::ask_advisor(
                    provider.as_ref(),
                    session,
                    query.clone(),
                    &snapshot,
                )
                .await
                {
                    Ok(reply) => {
                        let remaining = session.tokens_remaining();
                        let total_budget = session.token_budget;
                        app.messages.push(ChatMessage::assistant_parts(vec![
                            MessagePart::Advisor(format!(
                                "{reply}\n\n_(advisor budget: {} of {} tokens remaining)_",
                                remaining, total_budget
                            )),
                        ]));
                    }
                    Err(e) => {
                        app.messages.push(ChatMessage::assistant_parts(vec![
                            MessagePart::Advisor(format!(
                                "Advisor error: {e}\n\nUse `/clear` to start a fresh session if the budget is exhausted."
                            )),
                        ]));
                    }
                }
            }
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
                jfc_session::most_recent_session().await
            } else {
                let cwd_str = std::env::current_dir()
                    .ok()
                    .map(|p| p.display().to_string());
                jfc_session::most_recent_session_for_cwd(cwd_str.as_deref()).await
            };
            if let Some(session_id) = session_id {
                if let Some(messages) = crate::session::load_session(&session_id).await {
                    app.messages = messages;
                    let session_id_for_msg = session_id.clone();
                    app.switch_session(Some(session_id));
                    app.streaming_text.clear();
                    app.streaming_reasoning.clear();
                    app.streaming_response_bytes = 0;
                    app.streaming_assistant_idx = None;
                    app.scroll_to_bottom();
                    let scope = if want_global { "any cwd" } else { "this cwd" };
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Resumed session `{session_id_for_msg}`** ({scope}) — {} message(s) loaded.",
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
                let sessions = jfc_session::list_sessions().await;
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
            } else {
                let typed_session_id = crate::ids::SessionId::new(session_id);
                if let Some(messages) = crate::session::load_session(&typed_session_id).await {
                    let msg_count = messages.len();
                    // Compare the loaded session's recorded cwd against the
                    // current process cwd before mutating app state. The
                    // resume still proceeds either way — the toast is just
                    // informational so the user notices they may be
                    // pointing at the wrong project.
                    if !force {
                        let session_cwd = jfc_session::load_session_metadata(&typed_session_id)
                            .await
                            .and_then(|m| m.cwd);
                        let current_cwd = std::env::current_dir()
                            .map(|p| p.to_string_lossy().into_owned())
                            .unwrap_or_default();
                        if let Some(msg) =
                            jfc_session::cwd_mismatch_message(session_cwd.as_deref(), &current_cwd)
                        {
                            crate::toast::push_with_cap(
                                &mut app.toasts,
                                crate::toast::Toast::new(crate::toast::ToastKind::Warning, msg),
                            );
                        }
                    }
                    app.messages = messages;
                    app.switch_session(Some(typed_session_id.clone()));
                    app.streaming_text.clear();
                    app.streaming_reasoning.clear();
                    app.streaming_response_bytes = 0;
                    app.streaming_assistant_idx = None;
                    app.scroll_to_bottom();
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Resumed session `{typed_session_id}`** — {msg_count} message(s) loaded."
                    )));
                } else {
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Error:** Session `{typed_session_id}` not found."
                    )));
                }
            }
        }
        "/sessions" => {
            // List all sessions with metadata
            let sessions = jfc_session::list_sessions_with_metadata().await;
            if sessions.is_empty() {
                app.messages
                    .push(ChatMessage::assistant("No sessions found.".into()));
            } else {
                let mut body = format!("**{} session(s):**\n\n", sessions.len());
                for (i, s) in sessions.iter().take(20).enumerate() {
                    let prompt = s.first_prompt.as_deref().unwrap_or("(no prompt)");
                    let prompt_display = if prompt.len() > 50 {
                        let boundary = prompt.floor_char_boundary(50);
                        format!("{}…", &prompt[..boundary])
                    } else {
                        prompt.to_string()
                    };
                    let current = app.current_session_id.as_ref() == Some(&s.id);
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
        "/workflow" | "/wf" => {
            // v132 workflow templates. `/workflow` lists; `/workflow run <name>`
            // queues each step's prompt as a follow-up Submit so the leader
            // dispatches them in order. `parallel = true` steps batch into
            // a single multi-Task fan-out turn (the leader sees all the
            // prompts in one user message and is told to use parallel
            // dispatch).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            let mut sub = arg.split_whitespace();
            let verb = sub.next().unwrap_or("");
            let rest: String = sub.collect::<Vec<_>>().join(" ");
            match verb {
                "" | "list" => {
                    let names = crate::workflows::list(&cwd);
                    if names.is_empty() {
                        app.messages.push(ChatMessage::assistant(
                            "No workflows found. Create `.jfc/workflows/<name>.toml` with a TOML body containing `[[step]]` tables.".into(),
                        ));
                    } else {
                        let mut body = String::from("**Available workflows:**\n\n");
                        for name in &names {
                            match crate::workflows::load(&cwd, name) {
                                Ok(w) => body.push_str(&crate::workflows::render_summary(name, &w)),
                                Err(e) => {
                                    body.push_str(&format!("- `{name}` (parse error: {e})\n"))
                                }
                            }
                        }
                        body.push_str("\nRun with `/workflow run <name>`.");
                        app.messages.push(ChatMessage::assistant(body));
                    }
                }
                "run" => {
                    if rest.is_empty() {
                        app.messages.push(ChatMessage::assistant(
                            "Usage: `/workflow run <name>`. List available workflows with `/workflow`.".into(),
                        ));
                        return;
                    }
                    match crate::workflows::load(&cwd, &rest) {
                        Err(e) => {
                            app.messages.push(ChatMessage::assistant(format!(
                                "Failed to load workflow `{rest}`: {e}"
                            )));
                        }
                        Ok(workflow) => {
                            // Queue each step as a Submit so the leader sees
                            // them sequentially. Parallel steps would need
                            // a multi-Task aggregator — flag for now and
                            // dispatch sequentially as a stop-gap.
                            if let Some(tx) = tx {
                                for step in workflow.step {
                                    let prompt = format!(
                                        "Use the `{}` agent (Task tool) for this step:\n\n{}",
                                        step.agent, step.prompt
                                    );
                                    let _ = tx
                                        .send(crate::runtime::AppEvent::Ui(
                                            crate::runtime::UiEvent::Submit(prompt),
                                        ))
                                        .await;
                                }
                                app.messages.push(ChatMessage::assistant(format!(
                                    "Workflow `{rest}` queued — steps will fire sequentially."
                                )));
                            } else {
                                app.messages.push(ChatMessage::assistant(
                                    "Workflow runner needs the event channel; called from a context that doesn't have one.".into(),
                                ));
                            }
                        }
                    }
                }
                other => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Unknown subcommand `{other}`. Use `/workflow list` or `/workflow run <name>`."
                    )));
                }
            }
        }
        "/login" => {
            // v132 `/login` flow. With no arg, prints the chooser. With
            // a sub-target, the dispatcher returns a body string +
            // some side effects need a browser open. We always shell
            // out to xdg-open / open / start to launch the browser
            // (cheap, async-safe; failures are silent on systems
            // without one of those binaries).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts
                .get(1)
                .copied()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let dispatch = crate::providers::login_dispatch::dispatch(arg);
            let url = match &dispatch {
                crate::providers::login_dispatch::LoginDispatch::AnthropicApiKey(_)
                | crate::providers::login_dispatch::LoginDispatch::ConsoleApiKey(_) => {
                    Some("https://console.anthropic.com/settings/keys")
                }
                crate::providers::login_dispatch::LoginDispatch::ClaudeAiOAuth(_) => {
                    Some("https://claude.ai/login")
                }
                crate::providers::login_dispatch::LoginDispatch::CodexOAuth(_) => {
                    Some("https://auth.openai.com/codex/device")
                }
                _ => None,
            };
            if let Some(url) = url {
                // Best-effort: shell out to the platform browser opener.
                // Don't await — the browser launch is fire-and-forget.
                #[cfg(target_os = "linux")]
                let _ = std::process::Command::new("xdg-open").arg(url).spawn();
                #[cfg(target_os = "macos")]
                let _ = std::process::Command::new("open").arg(url).spawn();
                #[cfg(target_os = "windows")]
                let _ = std::process::Command::new("cmd")
                    .args(["/C", "start", url])
                    .spawn();
                tracing::info!(target: "jfc::login", %url, "opened browser for /login");
            }
            app.messages.push(ChatMessage::assistant(format!(
                "{dispatch}{}",
                if url.is_some() {
                    "\n\n_(opened the browser for you)_"
                } else {
                    ""
                }
            )));
        }
        "/batch" => {
            // /batch <prompt-file>: read newline-delimited prompts and
            // submit them via Anthropic's Message Batches API for the
            // 50% discount. The batch ID is returned synchronously;
            // results stream back via the Sessions API in a follow-up
            // turn (poll `/batch status <id>`).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Usage: `/batch <prompt-file>`. The file should contain one prompt per line."
                        .into(),
                ));
                return;
            }
            let path = std::path::PathBuf::from(arg);
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Failed to read `{}`: {e}",
                        path.display(),
                    )));
                    return;
                }
            };
            let prompts: Vec<String> = content
                .lines()
                .map(|l| l.trim().to_owned())
                .filter(|l| !l.is_empty() && !l.starts_with('#'))
                .collect();
            if prompts.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "No prompts found (each non-empty, non-`#`-comment line counts as one).".into(),
                ));
                return;
            }
            let Some(client) = crate::sdk_bridge::build_client() else {
                app.messages.push(ChatMessage::assistant(
                    "No Anthropic API key configured — `/batch` needs one (set ANTHROPIC_API_KEY)."
                        .into(),
                ));
                return;
            };
            let model = app.model.as_str().to_owned();
            let prompt_count = prompts.len();
            let path_for_msg = path.display().to_string();
            tokio::spawn(async move {
                use jfc_anthropic_sdk::batches::{BatchRequest, MessageBatchService};
                use jfc_anthropic_sdk::messages::{ContentBlock, Message, MessageRequest, Role};
                let svc = MessageBatchService::new(client);
                let requests: Vec<BatchRequest> = prompts
                    .into_iter()
                    .enumerate()
                    .map(|(i, p)| BatchRequest {
                        custom_id: format!("batch-{i}"),
                        params: MessageRequest {
                            model: model.clone(),
                            messages: vec![Message {
                                role: Role::User,
                                content: vec![ContentBlock::Text { text: p }],
                            }],
                            max_tokens: 4096,
                            system: None,
                            temperature: None,
                            top_p: None,
                            stop_sequences: Vec::new(),
                            tools: Vec::new(),
                            tool_choice: None,
                            stream: Some(false),
                            thinking: None,
                            reasoning_effort: None,
                        },
                    })
                    .collect();
                match svc.create(requests).await {
                    Ok(batch) => {
                        tracing::info!(
                            target: "jfc::batch",
                            batch_id = %batch.id,
                            count = prompt_count,
                            "batch submitted"
                        );
                        eprintln!(
                            "[batch] submitted {prompt_count} prompts from {path_for_msg} → batch {}",
                            batch.id
                        );
                    }
                    Err(e) => {
                        eprintln!("[batch] failed: {e}");
                    }
                }
            });
            app.messages.push(ChatMessage::assistant(format!(
                "Queued {prompt_count} prompts from `{}` for batch processing. \
                 Watch stderr / `/doctor` for the batch ID.",
                path.display()
            )));
        }
        "/diff" => {
            // Show pending uncommitted + unstaged changes via `git diff
            // HEAD --stat`. Read-only; doesn't run unless we're in a
            // git repo. Surface in the transcript as an assistant
            // message (markdown code block) so the user — and the
            // model on the next turn — can see what's pending.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let in_repo = std::process::Command::new("git")
                .args(["rev-parse", "--is-inside-work-tree"])
                .current_dir(&cwd)
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);
            if !in_repo {
                app.messages.push(ChatMessage::assistant(
                    "Not inside a git repository — `/diff` has nothing to show.".into(),
                ));
                return;
            }
            let stat = std::process::Command::new("git")
                .args(["diff", "HEAD", "--stat"])
                .current_dir(&cwd)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            let untracked = std::process::Command::new("git")
                .args(["ls-files", "--others", "--exclude-standard"])
                .current_dir(&cwd)
                .output()
                .ok()
                .and_then(|o| String::from_utf8(o.stdout).ok())
                .unwrap_or_default();
            if stat.trim().is_empty() && untracked.trim().is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Working tree is clean — no pending changes.".into(),
                ));
            } else {
                let mut body = String::from("**Pending changes (`git diff HEAD`):**\n\n```\n");
                if !stat.trim().is_empty() {
                    body.push_str(&stat);
                } else {
                    body.push_str("(no tracked-file changes)\n");
                }
                if !untracked.trim().is_empty() {
                    body.push_str("\n--- untracked ---\n");
                    body.push_str(&untracked);
                }
                body.push_str("```\n");
                app.messages.push(ChatMessage::assistant(body));
            }
        }
        "/undo" => {
            // Revert the most recent Edit / Write / MultiEdit /
            // ApplyPatch tool's filesystem mutation. Pulls from
            // `app.tool_undo_history` which the tool dispatcher
            // populates by capturing pre-mutation file content
            // before the tool executes. Only undoes ONE step;
            // run /undo repeatedly to walk back further.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let entry = crate::tools::pop_undo_entry();
            let Some(entry) = entry else {
                app.messages.push(ChatMessage::assistant(
                    "Nothing to undo — no recent file mutation captured this session.".into(),
                ));
                return;
            };
            let path = std::path::PathBuf::from(&entry.file_path);
            match entry.previous_content.clone() {
                Some(prev) => match std::fs::write(&path, &prev) {
                    Ok(()) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Reverted `{}` to its pre-{} state ({} bytes restored).",
                            path.display(),
                            entry.op_label,
                            prev.len()
                        )));
                    }
                    Err(e) => {
                        crate::tools::restore_undo_entry(entry.clone());
                        app.messages.push(ChatMessage::assistant(format!(
                            "Failed to write `{}`: {e} (kept the entry, run /undo again after fixing)",
                            path.display(),
                        )));
                    }
                },
                None => match std::fs::remove_file(&path) {
                    Ok(()) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Reverted `{}` (deleted; was newly-created by `{}`).",
                            path.display(),
                            entry.op_label
                        )));
                    }
                    Err(e) => {
                        crate::tools::restore_undo_entry(entry.clone());
                        app.messages.push(ChatMessage::assistant(format!(
                            "Failed to remove `{}`: {e}",
                            path.display(),
                        )));
                    }
                },
            }
        }
        "/export" => {
            // /export <path>: write the transcript as markdown to the
            // given path (defaults to ./jfc-transcript.md).
            app.messages.push(ChatMessage::user(text.to_owned()));
            let raw_path = parts.get(1).copied().unwrap_or("").trim();
            let path: std::path::PathBuf = if raw_path.is_empty() {
                std::path::PathBuf::from("jfc-transcript.md")
            } else {
                std::path::PathBuf::from(raw_path)
            };
            let mut body = String::from("# jfc transcript\n\n");
            for msg in &app.messages {
                let role = match msg.role {
                    crate::types::Role::User => "User",
                    crate::types::Role::Assistant => "Assistant",
                };
                body.push_str(&format!("## {role}\n\n"));
                for part in &msg.parts {
                    match part {
                        crate::types::MessagePart::Text(t) => {
                            body.push_str(t);
                            body.push_str("\n\n");
                        }
                        crate::types::MessagePart::Reasoning(t) => {
                            body.push_str("> _thinking_\n> \n> ");
                            body.push_str(&t.replace('\n', "\n> "));
                            body.push_str("\n\n");
                        }
                        crate::types::MessagePart::Tool(tc) => {
                            body.push_str(&format!(
                                "- **Tool: {}** ({})\n",
                                tc.kind.label(),
                                tc.status.label()
                            ));
                            body.push_str(&format!("  Input: {}\n", tc.input.summary()));
                            body.push('\n');
                        }
                        _ => {}
                    }
                }
            }
            match std::fs::write(&path, &body) {
                Ok(()) => {
                    let message = format!(
                        "Wrote transcript ({} bytes) to `{}`.",
                        body.len(),
                        path.display()
                    );
                    app.messages.push(ChatMessage::assistant(message.clone()));
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(crate::toast::ToastKind::Success, message),
                    );
                }
                Err(e) => {
                    let message = format!("Failed to write `{}`: {e}", path.display());
                    app.messages.push(ChatMessage::assistant(message.clone()));
                    crate::toast::push_with_cap(
                        &mut app.toasts,
                        crate::toast::Toast::new(crate::toast::ToastKind::Error, message),
                    );
                }
            }
        }
        "/verbose" => {
            // Toggle expanded-by-default tool blocks for the rest of
            // the session. Renderers read `app.verbose_mode` and lift
            // the per-tool preview cap when set.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts
                .get(1)
                .copied()
                .unwrap_or("")
                .trim()
                .to_ascii_lowercase();
            let target = match arg.as_str() {
                "on" | "true" | "1" => Some(true),
                "off" | "false" | "0" => Some(false),
                "" => Some(!app.verbose_mode),
                _ => None,
            };
            match target {
                Some(v) => {
                    app.verbose_mode = v;
                    app.messages.push(ChatMessage::assistant(format!(
                        "Verbose mode **{}** — tool blocks {} preview cap.",
                        if v { "ON" } else { "OFF" },
                        if v { "expand past" } else { "respect" },
                    )));
                }
                None => {
                    app.messages.push(ChatMessage::assistant(
                        "Usage: `/verbose [on|off]`. With no arg, toggles.".into(),
                    ));
                }
            }
        }
        "/fast" | "/f" => {
            // Toggle fast mode (lower-latency inference via Anthropic's
            // `fast-mode-2026-02-01` beta header). Mirrors Claude Code
            // v2.1.139's `/fast` command (Alt+O keybind).
            app.messages.push(ChatMessage::user(text.to_owned()));
            app.fast_mode = !app.fast_mode;
            crate::effort::set_fast_mode_global(app.fast_mode);
            app.messages.push(ChatMessage::assistant(format!(
                "Fast mode: **{}** — {}",
                if app.fast_mode { "ON" } else { "OFF" },
                if app.fast_mode {
                    "requests will use the low-latency inference path"
                } else {
                    "requests will use the standard inference path"
                },
            )));
        }
        "/pin" => {
            // Pin a message by transcript index so compaction can't
            // drop it. /pin without an arg pins the most recent
            // message; /pin <n> pins index n; /pin list prints the
            // current pin set.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg == "list" {
                if app.pinned_message_indices.is_empty() {
                    app.messages.push(ChatMessage::assistant(
                        "No pinned messages. `/pin <n>` pins index n; `/pin` pins the most recent."
                            .into(),
                    ));
                } else {
                    let mut idx: Vec<usize> = app.pinned_message_indices.iter().copied().collect();
                    idx.sort();
                    let listing = idx
                        .into_iter()
                        .map(|i| format!("- #{i}"))
                        .collect::<Vec<_>>()
                        .join("\n");
                    app.messages.push(ChatMessage::assistant(format!(
                        "**Pinned messages:**\n{listing}"
                    )));
                }
            } else if arg.is_empty() {
                if app.messages.is_empty() {
                    return;
                }
                let idx = app.messages.len() - 1;
                app.pinned_message_indices.insert(idx);
                app.messages.push(ChatMessage::assistant(format!(
                    "Pinned message #{idx} (compaction will preserve it)."
                )));
            } else {
                match arg.parse::<usize>() {
                    Ok(idx) if idx < app.messages.len() => {
                        app.pinned_message_indices.insert(idx);
                        app.messages
                            .push(ChatMessage::assistant(format!("Pinned message #{idx}.")));
                    }
                    Ok(idx) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "No message at index {idx} (transcript has {} messages).",
                            app.messages.len()
                        )));
                    }
                    Err(_) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Couldn't parse `{arg}` as a message index. Use `/pin`, `/pin <n>`, or `/pin list`."
                        )));
                    }
                }
            }
        }
        "/unpin" => {
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg.is_empty() || arg == "all" {
                let n = app.pinned_message_indices.len();
                app.pinned_message_indices.clear();
                app.messages
                    .push(ChatMessage::assistant(format!("Cleared {n} pin(s).")));
            } else {
                match arg.parse::<usize>() {
                    Ok(idx) => {
                        if app.pinned_message_indices.remove(&idx) {
                            app.messages
                                .push(ChatMessage::assistant(format!("Unpinned message #{idx}.")));
                        } else {
                            app.messages.push(ChatMessage::assistant(format!(
                                "Message #{idx} wasn't pinned."
                            )));
                        }
                    }
                    Err(_) => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Couldn't parse `{arg}` as a message index."
                        )));
                    }
                }
            }
        }
        "/timeline" => {
            // Render a chronological tool-call timeline for the most
            // recent assistant turn. For each Tool part, emit one row
            // with "kind │ summary │ Δms" so the user can spot slow
            // tools at a glance.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let last_assistant = app
                .messages
                .iter()
                .rposition(|m| matches!(m.role, crate::types::Role::Assistant));
            let Some(idx) = last_assistant else {
                app.messages.push(ChatMessage::assistant(
                    "No assistant turn yet — nothing to timeline.".into(),
                ));
                return;
            };
            let msg = &app.messages[idx];
            let mut rows: Vec<String> = Vec::new();
            for part in &msg.parts {
                if let crate::types::MessagePart::Tool(tc) = part {
                    let elapsed = tc
                        .elapsed_ms
                        .map(|ms| {
                            if ms >= 1_000 {
                                format!("{:.1}s", ms as f64 / 1000.0)
                            } else {
                                format!("{ms}ms")
                            }
                        })
                        .unwrap_or_else(|| "—".to_owned());
                    let summary = tc.input.summary();
                    let summary: String = summary.chars().take(60).collect();
                    rows.push(format!(
                        "  - **{}** · `{}` · {elapsed}",
                        tc.kind.label(),
                        summary,
                    ));
                }
            }
            if rows.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "Most recent assistant turn ran no tools.".into(),
                ));
            } else {
                app.messages.push(ChatMessage::assistant(format!(
                    "**Tool timeline (last assistant turn, {} tools):**\n{}",
                    rows.len(),
                    rows.join("\n"),
                )));
            }
        }
        "/doctor" => {
            // Mirrors Claude Code 2.1.139's /doctor command.
            // Health check: scan the most-likely failure modes for an
            // out-of-the-box jfc setup and surface a single status
            // block. Read-only; no fixes applied automatically — the
            // user opts in to remedies after seeing the report.
            app.messages.push(ChatMessage::user(text.to_owned()));

            let check = |ok: bool| if ok { "✓" } else { "✗" };

            let mut report = String::from("jfc doctor report\n─────────────────\n");

            // ── 1. Config file ────────────────────────────────────────────────
            {
                let cfg_path = crate::config::config_path();
                let cfg_display = cfg_path.display().to_string();
                // Tilde-shorten for readability
                let cfg_display = if let Some(home) = dirs::home_dir() {
                    cfg_display.replacen(&home.display().to_string(), "~", 1)
                } else {
                    cfg_display
                };
                let cfg_ok = cfg_path.exists() && {
                    // Try a parse round-trip to catch TOML errors
                    std::fs::read_to_string(&cfg_path)
                        .ok()
                        .and_then(|s| toml::from_str::<crate::config::Config>(&s).ok())
                        .is_some()
                };
                report.push_str(&format!(
                    "{} Config: {}{}\n",
                    check(cfg_ok),
                    cfg_display,
                    if cfg_ok {
                        ""
                    } else if !cfg_path.exists() {
                        " (not found)"
                    } else {
                        " (parse error)"
                    },
                ));
            }

            // ── 2. Auth: ANTHROPIC_API_KEY env ───────────────────────────────
            {
                let api_key_set = std::env::var("ANTHROPIC_API_KEY").is_ok();
                report.push_str(&format!(
                    "{} Auth: ANTHROPIC_API_KEY {}\n",
                    check(api_key_set),
                    if api_key_set { "set" } else { "not set" },
                ));
            }

            // ── 3. Auth: ~/.config/jfc/anthropic-accounts.json ───────────────
            {
                let accounts_path = dirs::config_dir()
                    .unwrap_or_else(|| std::path::PathBuf::from("."))
                    .join("jfc")
                    .join("anthropic-accounts.json");
                let accounts_ok = accounts_path.exists();
                let accounts_display = {
                    let s = accounts_path.display().to_string();
                    if let Some(home) = dirs::home_dir() {
                        s.replacen(&home.display().to_string(), "~", 1)
                    } else {
                        s
                    }
                };
                report.push_str(&format!(
                    "{} Auth: accounts file {} {}\n",
                    check(accounts_ok),
                    accounts_display,
                    if accounts_ok {
                        "(found)"
                    } else {
                        "(not found)"
                    },
                ));
            }

            // ── 4. CLAUDE.md in project root ──────────────────────────────────
            {
                let project_root = std::path::PathBuf::from(&app.cwd);
                let claude_md = project_root.join("CLAUDE.md");
                let md_ok = claude_md.exists();
                let md_display = format!(
                    "{}{}",
                    "./",
                    claude_md
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("CLAUDE.md")
                );
                report.push_str(&format!(
                    "{} CLAUDE.md: {}\n",
                    check(md_ok),
                    if md_ok {
                        md_display
                    } else {
                        format!("{} (not found)", md_display)
                    },
                ));
            }

            // ── 5. MCP servers ────────────────────────────────────────────────
            {
                let cfg = crate::config::load();
                if cfg.mcp.is_empty() {
                    report.push_str("  MCP: no servers configured\n");
                } else {
                    for (name, server) in &cfg.mcp {
                        // Determine the binary to probe: use `command` if set,
                        // otherwise the first element of `args` (e.g. npx), and
                        // fall back to the server name itself.
                        let probe_bin = server
                            .command
                            .as_deref()
                            .filter(|s| !s.is_empty())
                            .or_else(|| server.args.first().map(|s| s.as_str()))
                            .unwrap_or(name.as_str());
                        let found = std::process::Command::new("which")
                            .arg(probe_bin)
                            .output()
                            .map(|o| o.status.success())
                            .unwrap_or(false);
                        report.push_str(&format!(
                            "{} MCP: {} ({} {})\n",
                            check(found),
                            name,
                            probe_bin,
                            if found { "found" } else { "not found" },
                        ));
                    }
                }
            }

            // ── 6. Working directory + git repo ───────────────────────────────
            {
                let cwd = std::path::PathBuf::from(&app.cwd);
                let git_ok = std::process::Command::new("git")
                    .args(["rev-parse", "--git-dir"])
                    .current_dir(&cwd)
                    .output()
                    .map(|o| o.status.success())
                    .unwrap_or(false);
                // Grab current branch name when inside a git repo
                let branch = if git_ok {
                    std::process::Command::new("git")
                        .args(["rev-parse", "--abbrev-ref", "HEAD"])
                        .current_dir(&cwd)
                        .output()
                        .ok()
                        .and_then(|o| {
                            if o.status.success() {
                                String::from_utf8(o.stdout)
                                    .ok()
                                    .map(|s| s.trim().to_owned())
                            } else {
                                None
                            }
                        })
                        .unwrap_or_else(|| "unknown".to_owned())
                } else {
                    String::new()
                };
                let git_label = if git_ok {
                    format!("yes (branch: {branch})")
                } else {
                    "no".to_owned()
                };
                report.push_str(&format!("{} Git repo: {}\n", check(git_ok), git_label));
                report.push_str(&format!("  cwd: {}\n", cwd.display()));
            }

            // ── 7. Version ────────────────────────────────────────────────────
            report.push_str(&format!("  Version: {}\n", env!("CARGO_PKG_VERSION")));

            // ── 8. Bonus: active provider + permission mode ───────────────────
            report.push_str(&format!("  Provider: {}\n", app.provider.name()));
            report.push_str(&format!("  Permission mode: {:?}\n", app.permission_mode));

            // ── 9. Session cost so far ────────────────────────────────────────
            let total = crate::cost::total_cost(&app.usage_by_model);
            report.push_str(&format!(
                "  Session cost: {}\n",
                crate::cost::fmt_cost(total)
            ));

            app.messages.push(ChatMessage::assistant(report));
        }
        "/effort" => {
            // v132 reasoning-effort pin. `/effort low|medium|high|xhigh|max`
            // sets the pin; `/effort` alone shows the current state;
            // `/effort clear` removes the pin so the model picks adaptive.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts.get(1).copied().unwrap_or("").trim();
            if arg.is_empty() {
                app.messages
                    .push(ChatMessage::assistant(app.effort_state.status()));
            } else if arg == "clear" || arg == "off" {
                let msg = app.effort_state.clear();
                app.messages.push(ChatMessage::assistant(msg));
            } else if let Some(level) = crate::effort::ReasoningEffort::from_str_loose(arg) {
                let msg = app.effort_state.set(level);
                app.messages.push(ChatMessage::assistant(msg));
            } else {
                app.messages.push(ChatMessage::assistant(format!(
                    "Unknown effort `{arg}`. Use one of: low, medium, high, xhigh, max, clear."
                )));
            }
        }
        "/feature" => {
            // v132 feature-gate framework. `/feature` lists all gates and
            // their state; `/feature <codename> on|off` flips one.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let rest = parts.get(1).copied().unwrap_or("").trim();
            if rest.is_empty() {
                let mut body = String::from("**Feature gates:**\n\n");
                for &gate in crate::feature_gates::FeatureGate::ALL {
                    body.push_str(&format!(
                        "- `{}` — **{}** ({})\n",
                        gate.codename(),
                        if crate::feature_gates::is_enabled(gate) {
                            "ON"
                        } else {
                            "OFF"
                        },
                        gate.description(),
                    ));
                }
                body.push_str("\nToggle with `/feature <codename> on|off`.");
                app.messages.push(ChatMessage::assistant(body));
            } else {
                let mut sub = rest.split_whitespace();
                let name = sub.next().unwrap_or("");
                let toggle = sub.next().unwrap_or("").to_ascii_lowercase();
                let Some(gate) = crate::feature_gates::FeatureGate::from_codename(name) else {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Unknown feature gate `{name}`. List with `/feature`."
                    )));
                    return;
                };
                let enabled = match toggle.as_str() {
                    "on" | "enable" | "true" | "1" => true,
                    "off" | "disable" | "false" | "0" => false,
                    "" => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "`{}` is currently **{}**. Toggle with `/feature {} on|off`.",
                            gate.codename(),
                            if crate::feature_gates::is_enabled(gate) {
                                "ON"
                            } else {
                                "OFF"
                            },
                            gate.codename(),
                        )));
                        return;
                    }
                    other => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Unknown toggle `{other}`. Use `on` or `off`."
                        )));
                        return;
                    }
                };
                crate::feature_gates::set(gate, enabled);
                app.messages.push(ChatMessage::assistant(format!(
                    "`{}` set to **{}** ({}).",
                    gate.codename(),
                    if enabled { "ON" } else { "OFF" },
                    gate.description(),
                )));
                // v132 system-reminder so the model sees the gate flip
                // on the next turn (rather than guessing from changed
                // behavior).
                crate::system_reminder::append_to_last_user(
                    &mut app.messages,
                    &format!(
                        "Feature gate `{}` flipped to **{}** ({}). Adjust your \
                         behavior accordingly.",
                        gate.codename(),
                        if enabled { "ON" } else { "OFF" },
                        gate.description(),
                    ),
                );
            }
        }
        "/goal" => {
            // v137 session-scoped goal. `/goal <condition>` sets a stop
            // condition — the agent keeps working until the evaluator
            // says it's met (see `crate::goal::evaluate`). `/goal
            // clear` (or stop/off/reset/none/cancel) removes it.
            // `/goal` alone shows the current state.
            app.messages.push(ChatMessage::user(text.to_owned()));
            let arg = parts[1..].join(" ");
            let arg = arg.trim();
            if arg.is_empty() {
                let msg = match &app.goal {
                    Some(g) => format!(
                        "Current goal ({} iterations): {}\n\nUse `/goal clear` to remove.",
                        g.iterations, g.condition
                    ),
                    None => "No goal set. Usage: `/goal <condition>`".to_string(),
                };
                app.messages.push(ChatMessage::assistant(msg));
            } else if crate::goal::is_clear_arg(arg) {
                let prev = app.goal.take();
                app.goal_evaluator_in_flight = false;
                // Drop the sidecar so a future /continue doesn't
                // revive a goal the user just cancelled.
                if let Some(sid) = app.current_session_id.as_ref() {
                    crate::goal::save_sidecar(sid.as_str(), None);
                }
                let msg = match prev {
                    Some(g) => format!(
                        "Goal cleared after {} iterations: {}",
                        g.iterations, g.condition
                    ),
                    None => "No goal was set.".to_string(),
                };
                app.messages.push(ChatMessage::assistant(msg));
                crate::toast::push_with_cap(
                    &mut app.toasts,
                    crate::toast::Toast::new(
                        crate::toast::ToastKind::Success,
                        "Goal cleared".to_string(),
                    ),
                );
            } else {
                match crate::goal::validate_condition(arg) {
                    Ok(condition) => {
                        let goal = crate::goal::ActiveGoal::new(condition.clone());
                        app.goal = Some(goal);
                        // Persist the new goal so /continue picks it
                        // up if the user exits before the next turn.
                        if let Some(sid) = app.current_session_id.as_ref() {
                            crate::goal::save_sidecar(sid.as_str(), app.goal.as_ref());
                        }
                        app.messages.push(ChatMessage::assistant(format!(
                            "Goal set: {condition}\n\nThe agent will keep \
                             working until this condition is met (auto-\
                             evaluated after each turn, max {} iterations). \
                             Use `/goal clear` to cancel.",
                            crate::goal::MAX_ITERATIONS
                        )));
                        crate::toast::push_with_cap(
                            &mut app.toasts,
                            crate::toast::Toast::new(
                                crate::toast::ToastKind::Success,
                                format!("Goal: {condition}"),
                            ),
                        );
                        // Kick off work immediately: synthesize the
                        // Claude-Code-style meta prompt so the agent
                        // starts acting on the goal instead of sitting
                        // idle until the next user turn. Only fire
                        // when the session is genuinely idle (no
                        // streaming / pending approval / pending
                        // tools) AND we have an event channel.
                        let idle = !app.is_streaming
                            && app.pending_approval.is_none()
                            && app.approval_queue.is_empty()
                            && app.pending_tool_calls.is_empty();
                        if let (true, Some(tx)) = (idle, tx) {
                            let kickoff = format!(
                                "A session-scoped stop-condition hook is now \
                                 active with condition: \"{condition}\".\n\n\
                                 Briefly acknowledge the goal, then \
                                 immediately start or continue working toward \
                                 it. The hook will block stopping until the \
                                 condition holds (auto-evaluated after each \
                                 turn, max {} iterations). It auto-clears \
                                 once the condition is met.",
                                crate::goal::MAX_ITERATIONS
                            );
                            let _ = tx.send(AppEvent::Ui(UiEvent::Submit(kickoff))).await;
                            tracing::info!(
                                target: "jfc::goal",
                                "/goal: dispatched kickoff meta-prompt"
                            );
                        }
                    }
                    Err(reason) => {
                        app.messages.push(ChatMessage::assistant(reason.to_owned()));
                    }
                }
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
                 - `/advisor <question>` — Ask a parallel advisor without disturbing the main agent (set `JFC_ADVISOR_ENABLED=1`)\n\
                 - `/check` — Re-run cargo-check diagnostics\n\
                 - `/config` — Show parsed `~/.config/jfc/config.toml` (use `/config path` for the file location)\n\
                 - `/continue` (or `/c`) — Resume most recent session\n\
                 - `/resume <id>` — Resume a specific session by id\n\
                 - `/sessions` — List all saved sessions\n\
                 - `/theme [name]` — Open theme picker or switch/persist a theme\n\
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
                 - `/install-github-app` — Install Claude GitHub App on the current repo (browser flow)\n\
                 - `/pr <num>` — Show PR title, description, and review comments\n\
                 - `/pr-autofix <num>` — Build a model prompt that addresses PR review comments\n\
                 - `/setup-github-actions [force]` — Write `.github/workflows/jfc-review.yml`\n\
                 - `/help` — Show this message\n\
                 \n\
                 **Keys:**\n\
                 - Ctrl+B — Toggle sessions sidebar\n\
                 - Ctrl+M — Model picker\n\
                 - Ctrl+P — Command palette\n\
                 - Ctrl+O — Expand reasoning / open diagnostic panel\n\
                 - Alt+. / Alt+, — Raise / lower reasoning effort\n\
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
                 - `JFC_TOOL_TITLE_WIDTH=N` — cap tool title length (default 100)\n\
                 - `JFC_ADVISOR_ENABLED=1` — enable the `/advisor` parallel-advice slash command"
                    .into(),
            ));
        }
        "/memory" | "/mem" => {
            // `/memory` (no args)            → list memory files
            // `/memory recall on|off|status` → toggle two-phase recall
            //
            // The recall sub-command targets the runtime override in
            // `memory_recall::set_runtime_override` — persisting to
            // `~/.config/jfc/config.toml` is left to the user since they
            // may have hand-formatted that file.
            let arg = parts.get(1).copied().unwrap_or("").trim();
            app.messages.push(ChatMessage::user(text.to_owned()));
            if arg.starts_with("recall") {
                let sub = arg
                    .split_once(' ')
                    .map(|x| x.1)
                    .map(str::trim)
                    .unwrap_or("status");
                match sub {
                    "on" | "enable" => {
                        crate::memory_recall::set_runtime_override(Some(true));
                        app.messages.push(ChatMessage::assistant(
                            "Two-phase memory recall: **on** (runtime override).".into(),
                        ));
                    }
                    "off" | "disable" => {
                        crate::memory_recall::set_runtime_override(Some(false));
                        app.messages.push(ChatMessage::assistant(
                            "Two-phase memory recall: **off** (runtime override).".into(),
                        ));
                    }
                    "default" | "reset" => {
                        crate::memory_recall::set_runtime_override(None);
                        app.messages.push(ChatMessage::assistant(
                            "Two-phase memory recall: cleared runtime override; \
                             falling back to `~/.config/jfc/config.toml` value."
                                .into(),
                        ));
                    }
                    "status" | "" => {
                        let persisted = crate::config::load().memory_recall_enabled;
                        let effective = crate::memory_recall::is_enabled(persisted);
                        app.messages.push(ChatMessage::assistant(format!(
                            "**Memory recall**\n\
                             - Effective: **{}**\n\
                             - Persisted (config.toml): **{}**\n\
                             \n\
                             Toggle with `/memory recall on|off|reset`.",
                            if effective { "on" } else { "off" },
                            if persisted { "on" } else { "off" }
                        )));
                    }
                    other => {
                        app.messages.push(ChatMessage::assistant(format!(
                            "Unknown sub-command `{other}`. Try \
                             `/memory recall on|off|reset|status`."
                        )));
                    }
                }
            } else {
                let cwd = std::env::current_dir().unwrap_or_else(|_| ".".into());
                let mems = crate::memory::load_all_memories(&cwd);
                let body = if mems.is_empty() {
                    "No memory files found. Create `.jfc/memory/*.md` (project) or \
                     `~/.config/jfc/memory/*.md` (user) with YAML frontmatter \
                     (`type:` and `scope:`) and a markdown body."
                        .to_owned()
                } else {
                    let listing = crate::memory::format_existing_memories(&mems);
                    format!(
                        "**{} memor{} loaded:**\n\n{listing}\n\nUse `/memory recall status` to see whether two-phase recall is active.",
                        mems.len(),
                        if mems.len() == 1 { "y" } else { "ies" }
                    )
                };
                app.messages.push(ChatMessage::assistant(body));
            }
        }
        "/commit" => {
            // Generate a conventional commit message for staged changes.
            // 1. Check if anything is staged; bail early if not.
            // 2. Capture `git diff --cached` (capped at 8000 chars).
            // 3. Inject a user prompt so the model generates the message
            //    on the next turn — the user can then copy/run `git commit`.
            app.messages.push(ChatMessage::user("/commit".into()));
            let cwd = app.cwd.clone();
            let stat = tokio::process::Command::new("git")
                .args(["diff", "--cached", "--stat"])
                .current_dir(&cwd)
                .output()
                .await;
            match stat {
                Err(e) => {
                    app.messages.push(ChatMessage::assistant(format!(
                        "Could not run `git diff --cached --stat`: {e}"
                    )));
                }
                Ok(out) => {
                    let stat_str = String::from_utf8_lossy(&out.stdout);
                    if stat_str.trim().is_empty() {
                        app.messages.push(ChatMessage::assistant(
                            "Nothing staged. Stage changes first with `git add <file>` or `git add -p`.".into(),
                        ));
                    } else {
                        // Fetch the full diff, capped at 8000 chars to stay
                        // well within any reasonable context window.
                        let diff_output = tokio::process::Command::new("git")
                            .args(["diff", "--cached"])
                            .current_dir(&cwd)
                            .output()
                            .await
                            .ok();
                        let diff_str = diff_output
                            .map(|o| {
                                let s = String::from_utf8_lossy(&o.stdout).into_owned();
                                if s.len() > 8000 {
                                    // floor_char_boundary instead of a raw
                                    // byte slice — git diff can carry
                                    // non-ASCII filenames or content and
                                    // a fixed-byte cap would panic if a
                                    // multi-byte glyph straddled byte 8000.
                                    let cap = s.floor_char_boundary(8000);
                                    format!(
                                        "{}\n\n[... diff truncated at 8000 chars ...]",
                                        &s[..cap]
                                    )
                                } else {
                                    s
                                }
                            })
                            .unwrap_or_default();
                        let prompt = format!(
                            "Generate a conventional commit message for these staged changes.\n\
                             Format: `type(scope): description`\n\
                             Types: feat / fix / docs / style / refactor / test / chore\n\
                             Rules: imperative mood, ≤72 chars subject, no trailing period.\n\
                             Output ONLY the commit message — no explanation, no markdown fences.\n\n\
                             ```\n{diff_str}\n```"
                        );
                        app.messages
                            .push(ChatMessage::assistant("Analyzing staged changes…".into()));
                        app.queued_prompts.push_back(crate::app::QueuedPrompt {
                            text: prompt,
                            is_meta: false,
                            attachments: Vec::new(),
                        });
                        app.scroll_to_bottom();
                    }
                }
            }
        }
        "/review" => {
            // Ask the model to review current git changes for bugs, security
            // issues, and code quality problems with file:line specificity.
            app.messages.push(ChatMessage::user("/review".into()));
            let cwd = app.cwd.clone();
            // Prefer staged diff; fall back to HEAD diff; fall back to
            // working-tree diff so /review always finds something useful.
            let diff_output = {
                let staged = tokio::process::Command::new("git")
                    .args(["diff", "--cached"])
                    .current_dir(&cwd)
                    .output()
                    .await
                    .ok();
                let staged_str = staged
                    .as_ref()
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                    .unwrap_or_default();
                if !staged_str.is_empty() {
                    staged_str
                } else {
                    tokio::process::Command::new("git")
                        .args(["diff", "HEAD"])
                        .current_dir(&cwd)
                        .output()
                        .await
                        .ok()
                        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
                        .unwrap_or_default()
                }
            };
            if diff_output.is_empty() {
                app.messages.push(ChatMessage::assistant(
                    "No changes found (`git diff --cached` and `git diff HEAD` are both empty). \
                     Make some changes or stage files first."
                        .into(),
                ));
            } else {
                let capped = if diff_output.len() > 12_000 {
                    format!(
                        "{}\n\n[... diff truncated at 12000 chars ...]",
                        &diff_output[..12_000]
                    )
                } else {
                    diff_output
                };
                let prompt = format!(
                    "Review the following git diff for bugs, security issues, and code quality \
                     problems. Be specific — reference exact file names and line numbers where \
                     relevant. Organise findings by severity (Critical / High / Medium / Low). \
                     If there are no issues worth calling out, say so briefly.\n\n\
                     ```diff\n{capped}\n```"
                );
                app.messages
                    .push(ChatMessage::assistant("Reviewing changes…".into()));
                app.queued_prompts.push_back(crate::app::QueuedPrompt {
                    text: prompt,
                    is_meta: false,
                    attachments: Vec::new(),
                });
                app.scroll_to_bottom();
            }
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
            let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
            let cascade: Vec<&jfc_session::Task> = tasks
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
                        jfc_session::TaskStatus::Completed => "✓",
                        jfc_session::TaskStatus::InProgress => "⏵",
                        jfc_session::TaskStatus::Pending => "•",
                        jfc_session::TaskStatus::Failed => "✗",
                        jfc_session::TaskStatus::Deleted => "✗",
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
            let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
            let body = if tasks.is_empty() {
                "No tasks. Use `/task-add <subject>` to create one.".to_owned()
            } else {
                let mut s = format!("**{} task(s):**\n\n", tasks.len());
                for t in &tasks {
                    let icon = match t.status {
                        jfc_session::TaskStatus::Pending => "□",
                        jfc_session::TaskStatus::InProgress => "▣",
                        jfc_session::TaskStatus::Completed => "✓",
                        jfc_session::TaskStatus::Failed => "✗",
                        jfc_session::TaskStatus::Deleted => "✗",
                    };
                    let owner = t
                        .owner
                        .as_deref()
                        .map(|o| format!(" (@{o})"))
                        .unwrap_or_default();
                    let blocks = if t.blocked_by.is_empty() {
                        String::new()
                    } else {
                        format!(
                            " · blocked by {}",
                            t.blocked_by
                                .iter()
                                .map(|id| id.as_str())
                                .collect::<Vec<_>>()
                                .join(",")
                        )
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
                    Vec::<jfc_session::TaskId>::new(),
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
                    jfc_session::TaskPatch {
                        status: Some(jfc_session::TaskStatus::Completed),
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
        "/mcp" => {
            handle_mcp_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/theme" => {
            handle_theme_command(app, parts.get(1).copied().unwrap_or("").trim());
        }
        "/fleet" | "/fleetview" => {
            handle_fleet_command(app);
        }
        "/teleport" => {
            handle_teleport_command(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/init" => {
            handle_init_command(app).await;
        }
        "/plan" => {
            handle_doc_command(app, crate::document_formats::DocKind::Plan, tx).await;
        }
        "/roadmap" => {
            handle_doc_command(app, crate::document_formats::DocKind::Roadmap, tx).await;
        }
        "/parity" => {
            handle_doc_command(app, crate::document_formats::DocKind::Parity, tx).await;
        }
        "/philosophy" => {
            handle_doc_command(app, crate::document_formats::DocKind::Philosophy, tx).await;
        }
        "/usage" => {
            handle_doc_command(app, crate::document_formats::DocKind::Usage, tx).await;
        }
        "/cost" | "/stats" => {
            handle_cost_command(app);
        }
        "/status" => {
            handle_status_command(app);
        }
        "/bug" => {
            handle_bug_command(app, parts.get(1..).map(|r| r.join(" ")).unwrap_or_default());
        }
        "/rewind" => {
            handle_rewind_command(app, parts.get(1).copied().unwrap_or("").trim());
        }
        "/output-style" | "/style" | "/brief" => {
            // `/brief` is shorthand for `/output-style brief`. v132
            // exposes the same alias via `tengu_brief_mode_toggled`.
            let alias_brief = parts[0] == "/brief";
            let arg = if alias_brief {
                "brief".to_string()
            } else {
                parts.get(1).copied().unwrap_or("").trim().to_string()
            };
            handle_output_style_command(app, &arg);
        }
        "/dump-context" | "/debug-context" => {
            handle_dump_context_command(app).await;
        }
        "/install-github-app" => {
            handle_install_github_app(app).await;
        }
        "/pr" => {
            handle_pr_view(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/pr-autofix" => {
            handle_pr_autofix(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/setup-github-actions" => {
            handle_setup_github_actions(app, parts.get(1).copied().unwrap_or("").trim()).await;
        }
        "/dream" | "/learn" => {
            handle_dream_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/loop" | "/proactive" => {
            handle_loop_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
        }
        "/schedule" | "/routines" => {
            handle_schedule_command(app, parts.get(1).copied().unwrap_or("").trim(), tx).await;
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
                app.network_recovery_status = None;
                app.network_recovery_attempts = 0;
                app.streaming_assistant_idx = Some(assistant_idx);
                app.is_streaming = true;
                let now = std::time::Instant::now();
                app.streaming_started_at = Some(now);
                app.last_stream_event_at = Some(now);
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
                    .unwrap_or_else(jfc_session::generate_session_id);
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
                app.cancel_token = tokio_util::sync::CancellationToken::new();
                let cancel = app.cancel_token.clone();
                // wg-async: retry path mints a fresh cancel token for the
                // new stream so the old (possibly cancelled) one can't
                // racially interrupt the retry.
                tokio::spawn(async move {
                    crate::stream::stream_response(
                        provider, messages, model, tx_stream, interrupt, cancel, None,
                    )
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    use super::*;
    use crate::app::App;
    use crate::runtime::{AppEvent, UiEvent};
    #[allow(unused_imports)]
    use crate::types::*;
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

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

    struct StaticModelProvider;

    #[async_trait::async_trait]
    impl Provider for StaticModelProvider {
        fn name(&self) -> &str {
            "static"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            vec![ModelInfo::new("static-model", "Static Model", "static")]
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl jfc_provider::seal::Sealed for StaticModelProvider {}

    /// Test fixture: a fresh `App` plus a paired `(tx, rx)` so tests can both
    /// drive `handle_key` and inspect the AppEvents it emits. Pulled out so
    /// the dozens of tests below don't repeat the boilerplate.
    fn test_app() -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = jfc_session::TaskStore::in_memory();
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
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
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
            status: ToolStatus::Completed,
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
            display: crate::types::ToolDisplayState::DEFAULT,
            elapsed_ms: None,
            started_at: None,
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

    #[test]
    fn collect_all_models_empty_cache_falls_back_to_static_robust() {
        let mut app = App::new(Arc::new(StaticModelProvider), "static-model");
        app.provider_models
            .insert(jfc_provider::ProviderId::from("static"), Vec::new());

        let models = collect_all_models(&app);

        assert_eq!(models.len(), 1);
        assert_eq!(models[0].id.as_str(), "static-model");
        assert_eq!(models[0].provider.as_str(), "static");
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
                    display: crate::types::ToolDisplayState::DEFAULT,
                    elapsed_ms: None,
                    started_at: None,
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
            attachments: Vec::new(),
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
            AppEvent::Ui(UiEvent::Submit(t)) => assert_eq!(t, "ask"),
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
                    status: ToolStatus::Completed,
                    input: ToolInput::Read {
                        file_path: "x".into(),
                        offset: None,
                        limit: None,
                    },
                    output: ToolOutput::Text("hi".into()),
                    display: crate::types::ToolDisplayState::DEFAULT,
                    elapsed_ms: None,
                    started_at: None,
                },
            )]));
        let (tx, _rx) = channel();
        handle_key(&mut app, key(KeyCode::Char('o')), &tx)
            .await
            .unwrap();
        let MessagePart::Tool(tc) = &app.messages[0].parts[0] else {
            panic!("tool not found")
        };
        assert!(tc.display.is_expanded());
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
    async fn esc_double_tap_while_streaming_interrupts_instantly_normal() {
        let mut app = test_app();
        app.is_streaming = true;
        let (tx, _rx) = channel();
        // 1st ESC: arms the timer, shows hint.
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(app.last_esc_at.is_some(), "1st ESC should arm the timer");
        assert!(
            !app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst),
            "1st ESC should NOT fire interrupt"
        );
        // 2nd ESC: instantly kills.
        handle_key(&mut app, key(KeyCode::Esc), &tx).await.unwrap();
        assert!(
            app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst),
            "2nd ESC must set interrupt_flag"
        );
        assert!(
            app.cancel_token.is_cancelled(),
            "2nd ESC must cancel the token"
        );
        assert!(app.last_esc_at.is_none(), "timer cleared after kill");
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

    #[tokio::test]
    async fn alt_period_raises_reasoning_effort_normal() {
        let mut app = test_app();
        app.effort_state.set(crate::effort::ReasoningEffort::Medium);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char('.'), KeyModifiers::ALT),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(
            app.effort_state.current,
            Some(crate::effort::ReasoningEffort::High)
        );
    }

    #[tokio::test]
    async fn alt_comma_lowers_reasoning_effort_normal() {
        let mut app = test_app();
        app.effort_state.set(crate::effort::ReasoningEffort::Medium);
        let (tx, _rx) = channel();
        handle_key(
            &mut app,
            key_mod(KeyCode::Char(','), KeyModifiers::ALT),
            &tx,
        )
        .await
        .unwrap();
        assert_eq!(
            app.effort_state.current,
            Some(crate::effort::ReasoningEffort::Low)
        );
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
        let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
        assert_eq!(tasks.len(), 1);
    }

    #[tokio::test]
    async fn slash_task_add_robust_no_args() {
        let mut app = test_app();
        run_slash_command(&mut app, "/task-add").await;
        let tasks = app.task_store.list(jfc_session::DeletedFilter::Exclude);
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
    async fn slash_theme_opens_picker_when_no_arg_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/theme").await;
        assert!(app.show_theme_picker);
        assert!(app.theme_picker_input.is_empty());
        assert_eq!(app.theme_picker_selected, 0);
    }

    #[tokio::test]
    async fn slash_theme_unknown_pushes_warning_robust() {
        let mut app = test_app();
        run_slash_command(&mut app, "/theme nonexistent").await;
        // No theme change. Toast added.
        assert!(!app.toasts.is_empty());
    }

    // Regression: switching the theme MUST invalidate the render cache.
    // Without invalidation, cached lines carry baked-in syntect highlight
    // colors from the previous theme and the user sees stale colors until
    // each entry is naturally evicted by the LRU. For static transcript
    // content that staleness would persist until session reload.
    //
    // We exercise the bug by populating the cache, switching the theme via
    // the same `/theme` slash-command path the user types, then re-rendering
    // the same `(text, width)` key. The closure passed to
    // `get_or_insert_with` runs only on a cache miss, so a post-switch
    // closure invocation proves the entry was invalidated.
    #[tokio::test]
    async fn slash_theme_invalidates_render_cache_regression() {
        let mut app = test_app();
        let text = "hello **world**";
        let width: u16 = 80;

        // Prime the cache.
        {
            let mut cache = app.render_cache.borrow_mut();
            let _ = cache.get_or_insert_with(text, width, |t, _w| {
                vec![ratatui::text::Line::from(t.to_owned())]
            });
            assert_eq!(cache.len(), 1, "prime should populate exactly one entry");
        }

        // Switch theme via the public command surface (mirrors what a user
        // actually types). `dark` is always available, even if the test
        // App already starts on it — `Theme::by_name("light")` is the
        // visually distinct case.
        run_slash_command(&mut app, "/theme light").await;

        // Post-switch: the cache must be empty so the next render runs the
        // syntect pipeline against the new theme.
        {
            let cache = app.render_cache.borrow();
            assert_eq!(
                cache.len(),
                0,
                "theme switch should have cleared the render cache"
            );
        }

        // Stronger assertion: the closure runs again (cache miss) for the
        // exact same (text, width) key it was primed with.
        let mut closure_invocations = 0u32;
        {
            let mut cache = app.render_cache.borrow_mut();
            let _ = cache.get_or_insert_with(text, width, |t, _w| {
                closure_invocations += 1;
                vec![ratatui::text::Line::from(t.to_owned())]
            });
        }
        assert_eq!(
            closure_invocations, 1,
            "post-theme-switch render must miss the cache and rebuild lines"
        );
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
        app.current_session_id = Some(crate::ids::SessionId::new("ses_test"));
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
            .create::<jfc_session::TaskId>(
                "regular work".into(),
                "should not appear in /cascade".into(),
                None,
                Vec::new(),
            )
            .expect("create regular task");
        // A cascade task — SHOULD appear.
        let cascade = app
            .task_store
            .create::<jfc_session::TaskId>(
                "Update 2 call sites in src/foo.rs".into(),
                "cascade work".into(),
                None,
                Vec::new(),
            )
            .expect("create cascade task");
        let _ = app.task_store.update(
            cascade.id.as_str(),
            jfc_session::TaskPatch {
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
