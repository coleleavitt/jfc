use super::*;
use super::submit::handle_submit;
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

    if handle_session_picker_key(app, key, tx) {
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
                // Enter on an exact match (already-typed-out command) should
                // SUBMIT the command, not re-insert it with a trailing space.
                // Otherwise typing `/compact` + Enter just inserts another
                // space and the user has to press Enter again — the popup
                // ate the submit. Tab always tab-completes.
                KeyCode::Enter if matches.iter().any(|(cmd, _)| *cmd == prefix.as_str()) => {
                    app.slash_popup_selected = None;
                    // Fall through to the normal Enter handler below by
                    // not returning here — the popup dismissal is the
                    // only state change we needed.
                }
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
                    let streaming_before = app.streaming_assistant_idx;
                    let editing_before = app.editing_message_idx;
                    app.messages.remove(i);
                    // Removing a message shifts every subsequent index down
                    // by one. `streaming_assistant_idx` would otherwise point
                    // one slot past the live assistant if a fresh sub-stream
                    // already staged a slot after the queued user (agentic
                    // continuation, pause_turn resume). A stale index lets
                    // `StreamEvent::Tool` push `MessagePart::Tool` into a
                    // `Role::User` message → API 400 on the next request:
                    // "tool_use blocks can only appear in assistant messages".
                    // Reproduced as session ses_20260516_071052 msg[20]/msg[21].
                    if let Some(streaming_idx) = app.streaming_assistant_idx
                        && i < streaming_idx
                    {
                        app.streaming_assistant_idx = Some(streaming_idx - 1);
                    }
                    if let Some(edit_idx) = app.editing_message_idx {
                        if i == edit_idx {
                            app.editing_message_idx = None;
                        } else if i < edit_idx {
                            app.editing_message_idx = Some(edit_idx - 1);
                        }
                    }
                    tracing::info!(
                        target: "jfc::ui::queue::recall",
                        removed_at = i,
                        message_count = app.messages.len(),
                        streaming_before = ?streaming_before,
                        streaming_after = ?app.streaming_assistant_idx,
                        editing_before = ?editing_before,
                        editing_after = ?app.editing_message_idx,
                        is_streaming = app.is_streaming,
                        "up_recall: removed queued placeholder, adjusted indices"
                    );
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
            // ↓ at empty input with alive sub-agents → enter agent
            // select. Matches Claude Code's "↓ to manage" hint at the
            // top of the agent card. The user no longer needs the
            // Ctrl+X leader chord to dive into the fan — same muscle
            // memory as VS Code's command-palette `↓` to reach the
            // results list.
            if !input_has_text(app)
                && app.viewing_task_id.is_none()
                && app.background_tasks.values().any(|bt| bt.status.is_alive())
            {
                // Pick the most-recent alive agent (matches the
                // existing `↓ jump to latest` semantics inside the
                // task view).
                let mut alive_ids: Vec<String> = app
                    .background_tasks
                    .iter()
                    .filter(|(_, bt)| bt.status.is_alive())
                    .map(|(id, _)| id.clone())
                    .collect();
                alive_ids.sort();
                if let Some(latest) = alive_ids.last().cloned() {
                    app.viewing_task_id = Some(latest);
                    app.scroll_to_bottom();
                }
                return Ok(false);
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
                // Also clear pasted images so the next paste starts fresh.
                app.pasted_images.clear();
                app.image_counter = 0;
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
        // Ctrl+T cycles the expanded view: none → tasks → teammates → none.
        // Mirrors Claude Code's `app:toggleTodos` keybinding behavior.
        (KeyModifiers::CONTROL, KeyCode::Char('t')) => {
            use crate::app::ExpandedView;
            let has_teammates = app.team_context.is_active()
                || app.background_tasks.values().any(|bt| bt.status.is_alive());
            app.expanded_view = match app.expanded_view {
                ExpandedView::None => ExpandedView::Tasks,
                ExpandedView::Tasks if has_teammates => ExpandedView::Teammates,
                ExpandedView::Tasks => ExpandedView::None,
                ExpandedView::Teammates => ExpandedView::None,
            };
            // Sync the legacy show_task_panel bool for backward compat
            app.show_task_panel = app.expanded_view == ExpandedView::Tasks;
            return Ok(false);
        }
        // Alt+S opens the session picker popup — same shape as the
        // model picker (Alt+M) and theme picker, so the muscle memory
        // transfers. Ctrl+B keeps the legacy left sidebar; users
        // who prefer filter-and-go grab Alt+S, browse-and-stay grab
        // Ctrl+B.
        (KeyModifiers::ALT, KeyCode::Char('s')) => {
            open_session_picker(app);
            return Ok(false);
        }
        // Alt+Up / Alt+Down scroll the right-side info sidebar when it's
        // visible — surfaces overflow rows from the Tasks section without
        // stealing the main transcript scroll keys.
        (KeyModifiers::ALT, KeyCode::Up) if app.show_info_sidebar => {
            app.info_sidebar_scroll = app.info_sidebar_scroll.saturating_sub(2);
            return Ok(false);
        }
        (KeyModifiers::ALT, KeyCode::Down) if app.show_info_sidebar => {
            app.info_sidebar_scroll = app.info_sidebar_scroll.saturating_add(2);
            return Ok(false);
        }
        (KeyModifiers::ALT, KeyCode::PageUp) if app.show_info_sidebar => {
            app.info_sidebar_scroll = app.info_sidebar_scroll.saturating_sub(10);
            return Ok(false);
        }
        (KeyModifiers::ALT, KeyCode::PageDown) if app.show_info_sidebar => {
            app.info_sidebar_scroll = app.info_sidebar_scroll.saturating_add(10);
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
            // Persist the mode change to config.toml so it survives sessions.
            crate::config::save_permission_mode(&app.permission_mode);
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
            let compacting = app.compacting_started_at.is_some();
            if app.is_streaming || pipeline_busy || compacting {
                // Check if we can interrupt: if streaming with only
                // safe/interruptible tools, abort and inject instead
                // of queuing. This gives real-time steering.
                let can_interrupt = app.is_streaming
                    && !compacting
                    && app.pending_approval.is_none()
                    && app
                        .pending_tool_calls
                        .iter()
                        .all(|t| crate::scheduler::is_concurrency_safe(&t.kind));
                if can_interrupt {
                    tracing::info!(
                        target: "jfc::input::interrupt",
                        "interrupt-on-submit: aborting interruptible stream"
                    );
                    // Cancel the current stream
                    app.cancel_token.cancel();
                    app.cancel_token = tokio_util::sync::CancellationToken::new();
                    app.interrupt_flag
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    app.is_streaming = false;
                    app.streaming_started_at = None;
                    app.last_stream_event_at = None;
                    // Don't queue: submit the new turn immediately now that
                    // the interruptible stream has been cancelled.
                    handle_submit(app, text, tx).await?;
                } else {
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
                    app.queued_prompts.push(crate::app::QueuedPrompt {
                        text: text.clone(),
                        is_meta,
                        priority: crate::app::QueuePriority::Later,
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
                } // end else (not can_interrupt)
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

    // Image chip atomic delete: when Backspace is pressed and the cursor is
    // immediately after `]` of an `[Image #N]` token, delete the entire
    // chip as one unit (10+ chars) instead of requiring per-char deletion.
    if key.code == KeyCode::Backspace {
        let cursor = app.textarea.cursor();
        let (row, col) = (cursor.0, cursor.1);
        if let Some(line) = app.textarea.lines().get(row) {
            let before_cursor = &line[..col.min(line.len())];
            if let Some(start) = before_cursor.rfind("[Image #") {
                let chip = &before_cursor[start..];
                if chip.ends_with(']') {
                    let chip_len = chip.len();
                    // Delete the entire chip by moving cursor back and deleting forward
                    for _ in 0..chip_len {
                        app.textarea.input(crossterm::event::KeyEvent::new(
                            KeyCode::Backspace,
                            KeyModifiers::NONE,
                        ));
                    }
                    update_mention_state_after_input(app);
                    return Ok(false);
                }
            }
        }
    }

    app.textarea.input(key);
    update_mention_state_after_input(app);
    Ok(false)
}

