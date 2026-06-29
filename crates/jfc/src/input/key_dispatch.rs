//! Key-event dispatch.
//!
//! `handle_key` is a thin router that consults a priority-ordered chain of
//! focused handlers (slash popup → transcript search → jump nav → leader key →
//! arrow history → command keys → enter-submit → mention/backspace). Each
//! handler follows one shared contract:
//!
//! ```text
//! fn handle_X(...) -> Option<anyhow::Result<bool>>
//!   Some(result) → the handler consumed the key; `handle_key` returns `result`
//!                  immediately (the inner bool is the existing "should quit"
//!                  signal — true = exit the app, false = stay).
//!   None         → the key wasn't for this handler; fall through to the next.
//! ```
//!
//! This `Option` wrapper is what lets each block become a standalone fn while
//! preserving the original `handle_key`'s *exact* control flow — including the
//! deliberate fall-throughs (e.g. the slash popup's Enter-on-exact-match
//! dismisses the popup, returns `None`, and lets the Enter-submit handler fire;
//! Esc chains the same way). Don't "simplify" a `None` arm into an early
//! `return` without checking whether a later handler relies on the fall-through.

use super::submit::handle_submit;
use super::*;
pub async fn handle_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> anyhow::Result<bool> {
    // MCP elicitation modal has highest priority — it blocks tool execution.
    if elicitation::handle_elicitation_key(app, key, tx) {
        return Ok(false);
    }

    // Voice push-to-talk: Space drives recording when voice is enabled and no
    // modal is blocking. Press/Repeat → activate (the recorder dedups held-key
    // repeats); the key-up half lives in `handle_term_event` (Kitty Release).
    //
    // Start only from an *empty* prompt so Space mid-typing still types a space.
    // Once recording, absorb Space (and its repeats) regardless of prompt
    // contents — the live interim transcript types into the box, so the
    // "empty" guard no longer holds and a held key would otherwise leak spaces.
    if app.voice_enabled
        && crate::voice::is_initialized()
        && key.code == crossterm::event::KeyCode::Char(' ')
        && key.modifiers == crossterm::event::KeyModifiers::NONE
        && app.engine.pending_approval.is_none()
        && app.engine.pending_question.is_none()
        && app.engine.pending_elicitations.is_empty()
        && app.pending_rewrite_proposal.is_none()
    {
        let recording = app.voice_state != jfc_voice::VoiceState::Idle;
        let input_empty = app.textarea.lines().iter().all(|l| l.is_empty());
        if recording || input_empty {
            crate::voice::activate(true).await;
            return Ok(false);
        }
    }

    if approval::handle_approval_key(app, key, tx) {
        return Ok(false);
    }

    if question::handle_question_key(app, key, tx) {
        return Ok(false);
    }

    // Prompt-rewrite proposal modal — blocks the composer until the user
    // accepts/rejects/edits the reworded prompt (never silent).
    if prompt_rewrite::handle_prompt_rewrite_key(app, key, tx).await {
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

    if handle_bash_picker_key(app, key, tx) {
        return Ok(false);
    }

    if app.leader_key_active
        && let Some(t) = app.leader_key_timeout
        && t.elapsed() >= std::time::Duration::from_secs(2)
    {
        app.leader_key_active = false;
        app.leader_key_timeout = None;
    }

    // ─── Slash autocomplete popup ─────────────────────────────────────────
    // Active whenever the input bar is exactly one line starting with
    // `/` and there's at least one matching command. Tab/Enter
    // commits the highlighted entry, Up/Down navigate, Esc dismisses.
    if let Some(result) = handle_slash_popup_keys(app, key) {
        return result;
    }

    // ─── Transcript search (Ctrl+F) ──────────────────────────────────────
    if let Some(result) = handle_transcript_search_keys(app, key) {
        return result;
    }

    // ─── Prompt history search (Ctrl+R) ──────────────────────────────────
    if let Some(result) = handle_prompt_search_keys(app, key) {
        return result;
    }

    // ─── Jump-to navigation (Ctrl+G prefix) ──────────────────────────────
    if app.jump_armed
        && let Some(t) = app.jump_armed_at
        && t.elapsed() >= std::time::Duration::from_secs(2)
    {
        app.jump_armed = false;
        app.jump_armed_at = None;
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

    // Leader chord `Ctrl+X` then `b` opens the background-shell picker. Handled
    // here (before the sync leader handler) because populating the modal needs
    // the async `list_bash_tasks()` snapshot.
    if app.leader_key_active && matches!(key.code, KeyCode::Char('b')) {
        app.leader_key_active = false;
        app.leader_key_timeout = None;
        open_bash_picker(app).await;
        return Ok(false);
    }

    if let Some(result) = handle_leader_key_keys(app, key, tx) {
        return result;
    }

    // Up-arrow recall: when the textarea is empty and prompts are queued,
    // pressing Up pops the most recent queued prompt back into the textarea
    // for editing. Mirrors v126's "Press up to edit queued messages". Also
    // removes the corresponding queued placeholder from the transcript so the
    // user sees the action took effect — they can re-edit and re-submit.
    if let Some(result) = handle_up_recall_keys(app, key) {
        return result;
    }

    // Ctrl+Y yanks the last assistant message text to the system clipboard.
    // Alt+Y yanks the current draft/input buffer. Both go through the runtime
    // clipboard owner so OSC52 and platform clipboard behavior stay unified.
    if let Some(result) = handle_yank_key(app, key) {
        return result;
    }

    if let Some(result) = focused_widgets::handle_focused_widget_key(app, key, tx).await {
        return result;
    }

    // ─── User-configured keybindings (keybindings.toml) ──────────────────
    // Check before built-in bindings so users can override defaults.
    // Uses run_slash_command so actions stay in sync with their slash
    // counterparts automatically.
    if let Some(result) = handle_configured_keybinding(app, key, tx).await {
        return result;
    }

    if let Some(result) = handle_arrow_history_keys(app, key) {
        return result;
    }

    if let Some(result) = handle_command_keys(app, key, tx).await {
        return result;
    }

    if let Some(result) = handle_enter_submit(app, key, tx).await {
        return result;
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

    if let Some(result) = handle_chip_atomic_delete(app, key) {
        return result;
    }

    // Vim mode owns the prompt when enabled: Normal-mode keys are commands,
    // Insert-mode keys type. Split-borrow `vim` (Clone) from `textarea` so
    // both can be mutated. Plain insert editing when vim is off.
    if let Some(mut state) = app.vim.clone() {
        crate::input::vim::handle_key(&mut state, &mut app.textarea, key);
        app.vim = Some(state);
    } else {
        app.textarea.input(key);
    }
    update_mention_state_after_input(app);
    Ok(false)
}

/// Whether message `idx` carries a `Reasoning` part — i.e. there's a
/// `∴ Thinking` block Ctrl+O can collapse/expand.
fn message_has_reasoning(app: &App, idx: usize) -> bool {
    app.engine.messages.get(idx).is_some_and(|m| {
        m.parts
            .iter()
            .any(|p| matches!(p, jfc_core::MessagePart::Reasoning(_)))
    })
}

/// Index of the most recent message that has a reasoning block, if any.
/// Ctrl+O targets this when nothing is actively streaming.
fn last_reasoning_message_idx(app: &App) -> Option<usize> {
    (0..app.engine.messages.len())
        .rev()
        .find(|&i| message_has_reasoning(app, i))
}

pub(crate) fn request_user_interrupt(
    app: &mut App,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) {
    // View bookkeeping: a real interrupt resets the double-Esc timer.
    app.last_esc_at = None;
    crate::runtime::ops::interrupt(&mut app.engine, tx);
}

fn cycle_permission_mode(app: &mut App) {
    app.engine.permission_mode = app.engine.permission_mode.next();
    jfc_engine::config::save_permission_mode(&app.engine.permission_mode);
    jfc_engine::toast::push_with_cap(
        &mut app.engine.toasts,
        jfc_engine::toast::Toast::new(
            jfc_engine::toast::ToastKind::Info,
            format!(
                "{} Mode: {}",
                app.engine.permission_mode.symbol(),
                app.engine.permission_mode.label()
            ),
        ),
    );
}

fn toggle_syntax_highlighting(app: &mut App) {
    let disabled = !crate::markdown::syntax_highlighting_disabled();
    crate::markdown::set_syntax_highlighting_disabled(disabled);
    app.render_cache.borrow_mut().clear();
    app.height_index.borrow_mut().clear();
    let state = if disabled { "off" } else { "on" };
    jfc_engine::toast::push_with_cap(
        &mut app.engine.toasts,
        jfc_engine::toast::Toast::new(
            jfc_engine::toast::ToastKind::Info,
            format!("Syntax highlighting {state}"),
        ),
    );
}

/// User-configured keybindings (`keybindings.toml`), checked before built-in
/// bindings so users can override defaults. Returns `Some(result)` when a
/// configured action fired, `None` to fall through. Actions route through
/// `run_slash_command` so they stay in sync with their slash counterparts.
async fn handle_configured_keybinding(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> Option<anyhow::Result<bool>> {
    use crate::keybindings::KeyAction;
    let action = crate::keybindings::lookup(&key)?;
    match action {
        KeyAction::ToggleFastMode => run_slash_command_with_tx(app, "/fast", tx).await,
        KeyAction::ClearHistory => run_slash_command_with_tx(app, "/clear", tx).await,
        KeyAction::Compact => run_slash_command_with_tx(app, "/compact", tx).await,
        KeyAction::OpenModelPicker => open_model_picker(app),
        KeyAction::CyclePermissionMode => cycle_permission_mode(app),
        KeyAction::ToggleSyntaxHighlighting => toggle_syntax_highlighting(app),
        KeyAction::ToggleVerbose => run_slash_command_with_tx(app, "/verbose", tx).await,
        KeyAction::Exit => return Some(Ok(true)),
        KeyAction::ToggleHelp => app.show_help = !app.show_help,
    }
    Some(Ok(false))
}

/// Chip atomic delete: when Backspace is pressed with the cursor immediately
/// after the `]` of an `[Image #N]` or `[Pasted #N · …]` token, delete the
/// whole chip as one unit instead of char-by-char. Returns `Some(Ok(false))`
/// when a chip was deleted, `None` to let normal Backspace handling proceed.
fn handle_chip_atomic_delete(app: &mut App, key: event::KeyEvent) -> Option<anyhow::Result<bool>> {
    if key.code != KeyCode::Backspace {
        return None;
    }
    let cursor = app.textarea.cursor();
    let (row, col) = (cursor.0, cursor.1);
    let line = app.textarea.lines().get(row)?;
    let byte_col = line.char_indices().nth(col).map_or(line.len(), |(i, _)| i);
    let before_cursor = &line[..byte_col];
    let start = [
        before_cursor.rfind("[Image #"),
        before_cursor.rfind("[Pasted #"),
    ]
    .into_iter()
    .flatten()
    .max()?;
    let chip = &before_cursor[start..];
    if !chip.ends_with(']') {
        return None;
    }
    let chip_len = chip.len();
    for _ in 0..chip_len {
        app.textarea.input(crossterm::event::KeyEvent::new(
            KeyCode::Backspace,
            KeyModifiers::NONE,
        ));
    }
    update_mention_state_after_input(app);
    Some(Ok(false))
}

/// Command-key handling: the global keybinding match (Ctrl/Alt combos,
/// transcript navigation, diagnostic-panel keys, cursor motions). Split
/// out of `handle_key` per the cohesion guidance — every real arm
/// `return`s, so this returns `Some(result)` when a binding fired and
/// `None` to fall through to Enter-submit / textarea handling. Behavior
/// is identical to the prior inline match.
async fn handle_command_keys(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> Option<anyhow::Result<bool>> {
    match (key.modifiers, key.code) {
        (KeyModifiers::CONTROL, KeyCode::Char('c')) => {
            if input_has_text(app) {
                reset_input(app);
                // Also clear pasted images so the next paste starts fresh.
                app.pasted_images.clear();
                app.image_counter = 0;
                return Some(Ok(false));
            }
            if app.engine.has_interruptible_work() {
                request_user_interrupt(app, tx);
                return Some(Ok(false));
            }
            Some(Ok(true))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('g')) => {
            // Arm jump-to mode. The next single keystroke (e / t / m /
            // a) picks a target and scrolls the transcript to it. Esc
            // or any unbound key cancels. Auto-disarms after 2s so a
            // forgotten chord doesn't intercept user typing.
            app.jump_armed = true;
            app.jump_armed_at = Some(std::time::Instant::now());
            jfc_engine::toast::push_with_cap(
                &mut app.engine.toasts,
                jfc_engine::toast::Toast::new(
                    jfc_engine::toast::ToastKind::Info,
                    "jump: e=last error · t=last tool · m=last user · a=last assistant".to_string(),
                ),
            );
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('f')) if !input_has_text(app) => {
            // Arm transcript search. Empty bar (input has no text)
            // gates this so the user can still type literal Ctrl+F
            // sequences if some legacy keybinding software passes
            // them through. The search overlay renders at the bottom
            // of the screen via `app.transcript_search.is_some()`.
            app.transcript_search = Some(crate::app::TranscriptSearch::default());
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) if !input_has_text(app) => {
            cmd_edit_last_user_message(app)
        }
        (KeyModifiers::CONTROL, KeyCode::Char('l')) => cmd_yank_path_ref(app),
        (KeyModifiers::CONTROL, KeyCode::Char('r')) => cmd_open_prompt_search(app),
        (KeyModifiers::CONTROL, KeyCode::Char('z')) => {
            // Undo the last textarea edit. ratatui-textarea tracks
            // history internally — Ctrl+Z is the universal undo
            // gesture and was previously unbound. Returns false when
            // there's nothing to undo, which we silently ignore so
            // the keystroke isn't reflected.
            app.textarea.undo();
            Some(Ok(false))
        }
        (mods, KeyCode::Char('Z'))
            if mods.contains(KeyModifiers::CONTROL) && mods.contains(KeyModifiers::SHIFT) =>
        {
            // Ctrl+Shift+Z redo. The shift modifier may or may not be
            // exposed depending on the kitty-protocol negotiation, so
            // match the modifier-set explicitly.
            app.textarea.redo();
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('p')) => {
            app.palette.open();
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('m')) => {
            open_model_picker(app);
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('b')) => {
            app.session_sidebar.visible = !app.session_sidebar.visible;
            if app.session_sidebar.visible {
                app.session_sidebar.meta = jfc_session::list_sessions_with_metadata().await;
                app.session_sidebar.selected = 0;
                app.session_sidebar.list.select(Some(0));
            }
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('x')) => {
            app.leader_key_active = true;
            app.leader_key_timeout = Some(std::time::Instant::now());
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('i')) => {
            app.info_sidebar.visible = !app.info_sidebar.visible;
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('s')) => {
            app.info_sidebar.visible = !app.info_sidebar.visible;
            Some(Ok(false))
        }
        // Ctrl+T cycles the expanded view: none → tasks → teammates → none.
        // Mirrors Claude Code's `app:toggleTodos` keybinding behavior.
        // Gated on `todoFeatureEnabled` (CC 2.1.167 settings key).
        (KeyModifiers::CONTROL, KeyCode::Char('t')) => {
            use crate::app::ExpandedView;
            // todoFeatureEnabled defaults to true; only hide when explicitly false.
            let todo_enabled = jfc_engine::config::load_arc()
                .claude
                .todo_feature_enabled
                .unwrap_or(true);
            if !todo_enabled {
                return Some(Ok(false));
            }
            let has_teammates = app.engine.team_context.is_active()
                || app
                    .engine
                    .background_tasks
                    .values()
                    .any(|bt| bt.status.is_alive());
            app.task_panel.expanded_view = match app.task_panel.expanded_view {
                ExpandedView::None => ExpandedView::Tasks,
                ExpandedView::Tasks if has_teammates => ExpandedView::Teammates,
                ExpandedView::Tasks => ExpandedView::None,
                ExpandedView::Teammates => ExpandedView::None,
            };
            // Keep the task panel's visibility aligned with the expanded view.
            app.task_panel.visible = app.task_panel.expanded_view == ExpandedView::Tasks;
            Some(Ok(false))
        }
        // Alt+S opens the session picker popup — same shape as the
        // model picker (Alt+M) and theme picker, so the muscle memory
        // transfers. Ctrl+B keeps the legacy left sidebar; users
        // who prefer filter-and-go grab Alt+S, browse-and-stay grab
        // Ctrl+B.
        (KeyModifiers::ALT, KeyCode::Char('y')) => cmd_yank_current_input(app),
        (KeyModifiers::ALT, KeyCode::Char('s')) => {
            open_session_picker(app);
            Some(Ok(false))
        }
        // Alt+Up / Alt+Down scroll the right-side info sidebar when it's
        // visible — surfaces overflow rows from the Tasks section without
        // stealing the main transcript scroll keys.
        (KeyModifiers::ALT, KeyCode::Up) if app.info_sidebar.visible => {
            app.info_sidebar.scroll = app.info_sidebar.scroll.saturating_sub(2);
            Some(Ok(false))
        }
        (KeyModifiers::ALT, KeyCode::Down) if app.info_sidebar.visible => {
            app.info_sidebar.scroll = app.info_sidebar.scroll.saturating_add(2);
            Some(Ok(false))
        }
        (KeyModifiers::ALT, KeyCode::PageUp) if app.info_sidebar.visible => {
            app.info_sidebar.scroll = app.info_sidebar.scroll.saturating_sub(10);
            Some(Ok(false))
        }
        (KeyModifiers::ALT, KeyCode::PageDown) if app.info_sidebar.visible => {
            app.info_sidebar.scroll = app.info_sidebar.scroll.saturating_add(10);
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('v')) => cmd_paste_clipboard_image(app),
        // NOTE: Ctrl+Y (yank last assistant message) is handled earlier by
        // `handle_yank_key`, which runs before this match. No arm here.
        (KeyModifiers::CONTROL, KeyCode::Char('o')) => {
            // Ctrl+O toggles the reasoning ("∴ Thinking") block on the
            // streaming / most-recent assistant message — that's what the
            // inline `ctrl+o to collapse` hint advertises, so the toggle
            // must win. Previously *any* existing diagnostic hijacked this
            // key and opened the diagnostic panel instead, so the hint lied
            // in every session that had a warning. Now diagnostics are only
            // the fallback when there's no reasoning block to toggle, and
            // closing an open panel still takes precedence.
            //
            // Default-state fix: the renderer treats a missing entry as
            // *expanded* (`unwrap_or(true)`), so the toggle must seed `true`
            // before flipping — otherwise the first press seeded `false` and
            // flipped back to `true`, a visible no-op (the block stayed open
            // while the hint said "collapse").
            let target = app
                .engine
                .streaming_assistant_idx
                .filter(|&i| message_has_reasoning(app, i))
                .or_else(|| last_reasoning_message_idx(app));
            if app.show_diagnostic_panel {
                app.show_diagnostic_panel = false;
            } else if let Some(idx) = target {
                let entry = app.reasoning_expanded.entry(idx).or_insert(true);
                *entry = !*entry;
            } else if !app.engine.diagnostics.is_empty() {
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
                for entry in &app.engine.diagnostics {
                    app.delivered_diagnostics
                        .insert(jfc_engine::diagnostics::entry_key(entry));
                }
            }
            Some(Ok(false))
        }
        (KeyModifiers::ALT, KeyCode::Char('.')) => {
            step_reasoning_effort(app, true);
            Some(Ok(false))
        }
        (KeyModifiers::ALT, KeyCode::Char(',')) => {
            step_reasoning_effort(app, false);
            Some(Ok(false))
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
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::Up | KeyCode::Char('k')) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = app.diagnostic_panel_scroll.saturating_sub(1);
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::PageDown) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = app.diagnostic_panel_scroll.saturating_add(10);
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::PageUp) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = app.diagnostic_panel_scroll.saturating_sub(10);
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::Home | KeyCode::Char('g')) if app.show_diagnostic_panel => {
            app.diagnostic_panel_scroll = 0;
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::End | KeyCode::Char('G')) if app.show_diagnostic_panel => {
            // The renderer clamps overflow each frame, so passing a
            // large value lands at the bottom regardless of the
            // current diagnostic-set size.
            app.diagnostic_panel_scroll = usize::MAX / 2;
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::Esc) if app.show_diagnostic_panel => {
            app.show_diagnostic_panel = false;
            Some(Ok(false))
        }
        // ─── Task view: sticky arrow navigation ──────────────────────────
        // Once you're inside the task view (Ctrl+X then ↓ to enter, or you
        // typed something equivalent) plain ←/→ cycle through running
        // tasks, ↑ leaves the view, ↓ jumps to the most recent. No
        // leader-key chord required for each step — the leader is only
        // needed to *enter* the view. Without this the user had to type
        // Ctrl+X → → → → → to walk through five running agents.
        (KeyModifiers::NONE, KeyCode::Right) | (KeyModifiers::NONE, KeyCode::Left)
            if app.task_panel.viewing_task_id.is_some() && !input_has_text(app) =>
        {
            let task_ids: Vec<String> = crate::render::fleet_ordered_task_ids(app);
            if task_ids.is_empty() {
                return Some(Ok(false));
            }
            let pos = app
                .task_panel
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
                app.task_panel.viewing_task_id = Some(task_ids[next].clone());
                app.scroll_to_bottom();
            }
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::Up)
            if app.task_panel.viewing_task_id.is_some() && !input_has_text(app) =>
        {
            // Up exits the task view back to the main transcript —
            // matches the leader-mode `k` behavior so muscle memory is
            // consistent across modes. Per-task expansion state stays
            // in `app.task_panel.viewing_expanded` so re-entering the same
            // task restores what was expanded.
            app.task_panel.viewing_task_id = None;
            app.scroll_to_bottom();
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::Down)
            if app.task_panel.viewing_task_id.is_some() && !input_has_text(app) =>
        {
            // Down jumps to the most recently spawned task — useful
            // when several agents are running and you want the one
            // that just kicked off.
            let task_ids: Vec<String> = crate::render::fleet_ordered_task_ids(app);
            if let Some(last) = task_ids.last() {
                app.task_panel.viewing_task_id = Some(last.clone());
                app.scroll_to_bottom();
            }
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::Esc) => cmd_handle_escape(app, key, tx),
        (KeyModifiers::SHIFT, KeyCode::BackTab) | (KeyModifiers::NONE, KeyCode::BackTab) => {
            cycle_permission_mode(app);
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::PageUp) => {
            app.scroll_page_up();
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::PageDown) => {
            app.scroll_page_down();
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Home) => {
            app.scroll_to_top();
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::End) => {
            app.scroll_to_bottom();
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::Home) => {
            app.textarea.move_cursor(CursorMove::Head);
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::End) => {
            app.textarea.move_cursor(CursorMove::End);
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('a')) => {
            app.textarea.move_cursor(CursorMove::Head);
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('e')) => {
            app.textarea.move_cursor(CursorMove::End);
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('u')) => {
            app.textarea.delete_line_by_head();
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('k')) => {
            app.textarea.delete_line_by_end();
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('w')) => {
            app.textarea.delete_word();
            Some(Ok(false))
        }
        (KeyModifiers::CONTROL, KeyCode::Char('d')) => {
            if input_has_text(app) {
                app.textarea.delete_next_char();
                return Some(Ok(false));
            }
            Some(Ok(true))
        }
        (KeyModifiers::ALT, KeyCode::Char('d')) => {
            app.textarea.delete_next_word();
            Some(Ok(false))
        }
        (KeyModifiers::ALT, KeyCode::Char('b')) => {
            app.textarea.move_cursor(CursorMove::WordBack);
            Some(Ok(false))
        }
        (KeyModifiers::ALT, KeyCode::Char('f')) => {
            app.textarea.move_cursor(CursorMove::WordForward);
            Some(Ok(false))
        }
        // Ctrl+B is sidebar toggle (defined above). Ctrl+F is full-page-down.
        (KeyModifiers::CONTROL, KeyCode::Char('f')) => {
            let full = app.viewport_height.max(1);
            app.scroll_down(full);
            Some(Ok(false))
        }
        _ => None,
    }
}

// ─── Extracted command-key handlers ──────────────────────────────────────
// These are the formerly-inline bodies of the larger `handle_command_keys`
// match arms, lifted out verbatim so the dispatch match stays scannable.
// Each returns the same `Option<Result<bool>>` contract: `Some(Ok(false))`
// = handled, keep running; `Some(Ok(true))` = quit; `None` = not handled.

/// Ctrl+E — edit the most recent user message in place.
fn cmd_edit_last_user_message(app: &mut App) -> Option<anyhow::Result<bool>> {
    if app.engine.is_streaming
        || !app.engine.pending_tool_calls.is_empty()
        || app.engine.pending_approval.is_some()
    {
        jfc_engine::toast::push_with_cap(
            &mut app.engine.toasts,
            jfc_engine::toast::Toast::new(
                jfc_engine::toast::ToastKind::Warning,
                "edit: still in flight, finish or interrupt first".to_string(),
            ),
        );
        return Some(Ok(false));
    }
    let last_user: Option<(usize, String)> =
        app.engine
            .messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, m)| {
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
        jfc_engine::toast::push_with_cap(
            &mut app.engine.toasts,
            jfc_engine::toast::Toast::new(
                jfc_engine::toast::ToastKind::Info,
                "editing previous message — Esc cancels, Enter resubmits".to_string(),
            ),
        );
    } else {
        jfc_engine::toast::push_with_cap(
            &mut app.engine.toasts,
            jfc_engine::toast::Toast::new(
                jfc_engine::toast::ToastKind::Info,
                "no previous user message to edit".to_string(),
            ),
        );
    }
    Some(Ok(false))
}

/// Ctrl+L — yank a `path:line(:col)?` reference from recent tool output,
/// cycling through matches on repeated presses.
fn cmd_yank_path_ref(app: &mut App) -> Option<anyhow::Result<bool>> {
    let paths = collect_recent_paths(&app.engine.messages);
    if paths.is_empty() {
        jfc_engine::toast::push_with_cap(
            &mut app.engine.toasts,
            jfc_engine::toast::Toast::new(
                jfc_engine::toast::ToastKind::Info,
                "no path:line refs found in recent output".to_string(),
            ),
        );
        return Some(Ok(false));
    }
    let idx = app.path_yank_cursor % paths.len();
    let target = paths[idx].clone();
    crate::runtime::copy_to_clipboard(&target, "path-yank");
    app.path_yank_cursor = app.path_yank_cursor.wrapping_add(1);
    Some(Ok(false))
}

/// Alt+Y — copy the current editable prompt buffer.
fn cmd_yank_current_input(app: &mut App) -> Option<anyhow::Result<bool>> {
    let text = app.textarea.lines().join("\n");
    if text.trim().is_empty() {
        return Some(Ok(false));
    }

    crate::runtime::copy_to_clipboard(&text, "input-yank");
    Some(Ok(false))
}

/// Ctrl+R — open reverse-history search over past user prompts.
fn cmd_open_prompt_search(app: &mut App) -> Option<anyhow::Result<bool>> {
    let mut all: Vec<String> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for m in app.engine.messages.iter().rev() {
        if !m.role_is_user() || m.is_compact_boundary() {
            continue;
        }
        if let Some(text) = m.parts.iter().find_map(|p| match p {
            MessagePart::Text(s) if !s.is_empty() && !s.starts_with('/') => Some(s.clone()),
            _ => None,
        }) && seen.insert(text.clone())
        {
            all.push(text);
        }
    }
    // Append cross-session prompts (already de-duplicated against current session
    // in `user_prompts`, but PromptSearch.all is built independently here, so
    // de-dup manually using the same `seen` set).
    for p in app.prior_session_prompts.iter().rev() {
        if seen.insert(p.clone()) {
            all.push(p.clone());
        }
    }
    if all.is_empty() {
        jfc_engine::toast::push_with_cap(
            &mut app.engine.toasts,
            jfc_engine::toast::Toast::new(
                jfc_engine::toast::ToastKind::Info,
                "no prompt history to search".to_string(),
            ),
        );
    } else {
        let mut search = crate::app::PromptSearch {
            all,
            ..Default::default()
        };
        search.refilter();
        app.prompt_search = Some(search);
    }
    Some(Ok(false))
}

/// Ctrl+V — attach a clipboard image, falling back to text paste.
fn cmd_paste_clipboard_image(app: &mut App) -> Option<anyhow::Result<bool>> {
    match crate::attachments::read_clipboard_image() {
        Ok(Some((att, w, h))) => {
            tracing::debug!(
                target: "jfc::input::paste",
                width = w,
                height = h,
                bytes = att.bytes.len(),
                "attached clipboard image"
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
            Some(Ok(false))
        }
        Ok(None) => {
            if let Ok(mut cb) = arboard::Clipboard::new()
                && let Ok(text) = cb.get_text()
            {
                app.textarea.insert_str(&text);
            }
            Some(Ok(false))
        }
        Err(e) => {
            tracing::debug!(target: "jfc::input", error = %e, "Ctrl+V image paste failed");
            Some(Ok(false))
        }
    }
}

/// Esc (no input) — the dismiss/interrupt cascade: vim-mode exit, clear a
/// post-copy highlight, dismiss recap/help, cancel edit/task-view, then the
/// double-tap-to-interrupt flow, finally clearing the input.
fn cmd_handle_escape(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> Option<anyhow::Result<bool>> {
    if app
        .vim
        .as_ref()
        .is_some_and(|s| s.mode != crate::input::vim::VimMode::Normal)
    {
        app.textarea.cancel_selection();
        if let Some(s) = app.vim.as_mut() {
            s.mode = crate::input::vim::VimMode::Normal;
            s.pending = ratatui_textarea::Input::default();
        }
        return Some(Ok(false));
    }
    if app.text_selection.is_some() {
        app.text_selection = None;
        return Some(Ok(false));
    }
    if app.away_recap.is_some() {
        app.away_recap = None;
        return Some(Ok(false));
    }
    if app.show_help {
        app.show_help = false;
        return Some(Ok(false));
    }
    if app.editing_message_idx.is_some() {
        app.editing_message_idx = None;
        app.textarea.select_all();
        app.textarea.cut();
        jfc_engine::toast::push_with_cap(
            &mut app.engine.toasts,
            jfc_engine::toast::Toast::new(
                jfc_engine::toast::ToastKind::Info,
                "edit cancelled".to_string(),
            ),
        );
        return Some(Ok(false));
    }
    if app.task_panel.viewing_task_id.is_some() {
        app.task_panel.viewing_task_id = None;
        return Some(Ok(false));
    }

    // Double-tap ESC to instantly kill active work: 1st arms a 600ms timer +
    // hint, 2nd (within the window) fires the interrupt.
    const DOUBLE_TAP_MS: u128 = 600;
    let active = app.engine.has_interruptible_work();
    if active {
        if key.kind == event::KeyEventKind::Repeat {
            return Some(Ok(false));
        }
        let now = std::time::Instant::now();
        let armed = app
            .last_esc_at
            .map(|t| now.duration_since(t).as_millis() < DOUBLE_TAP_MS)
            .unwrap_or(false);
        if armed {
            request_user_interrupt(app, tx);
        } else {
            app.last_esc_at = Some(now);
            jfc_engine::toast::push_with_cap(
                &mut app.engine.toasts,
                jfc_engine::toast::Toast::new(
                    jfc_engine::toast::ToastKind::Info,
                    "Press ESC again to interrupt".to_owned(),
                ),
            );
        }
        return Some(Ok(false));
    }
    reset_input(app);
    Some(Ok(false))
}

/// Decide whether a fresh submit should *interrupt* the in-flight stream
/// (cancel + start the new turn now) or be *queued* behind it.
///
/// Plain Enter is queue-first: it should acknowledge the prompt and let the
/// active turn finish, not unexpectedly steer the conversation. Real-time
/// steering is still useful, but it must be explicit (`Alt+Enter`) and only
/// after the model has actually begun producing output.
///
/// `streaming_response_bytes` is the precise "output has begun" signal: it's
/// reset to 0 at every turn start and incremented on the first text/thinking/
/// tool-input delta (see `stream_chunk.rs`). `> 0` therefore means the
/// connection opened and the model started responding — the only state where
/// explicit interrupting is a coherent action.
///
/// When `message_queue_mode = true` in config, even explicit submit interrupts
/// are suppressed — every new prompt queues behind the current turn instead.
pub(crate) fn can_interrupt_on_submit(
    app: &App,
    compacting: bool,
    explicit_interrupt: bool,
    queue_mode: bool,
) -> bool {
    if queue_mode || !explicit_interrupt {
        return false;
    }
    app.engine.is_streaming
        && !compacting
        // Connection opened and the model has begun producing output. Before
        // the first byte there's nothing to steer — queue instead so the
        // first turn isn't silently dropped mid-connect.
        && app.engine.streaming_response_bytes > 0
        && app.engine.pending_approval.is_none()
        && app.engine.approval_queue.is_empty()
        && app.engine.pending_classifications == 0
        && app.engine.in_flight_eager_dispatches == 0
        && app.engine.in_flight_tool_batches == 0
        && app.engine.in_progress_tool_use_ids.is_empty()
        && app.engine
            .pending_tool_calls
            .iter()
            .all(|t| jfc_engine::scheduler::is_concurrency_safe(&t.kind))
}

/// `Enter` (without Shift) submission flow: trim textarea, route to the
/// streaming submit or to the queued-prompts list depending on busy-state.
/// Returns `Some(Ok(false))` on the non-empty path, `None` when the
/// textarea is empty (so the caller falls through to mention / textarea
/// handling). Extracted from `handle_key` for cohesion — the behavior is
/// identical to the prior inline block.
async fn handle_enter_submit(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> Option<anyhow::Result<bool>> {
    if key.code != KeyCode::Enter {
        return None;
    }

    let enter_sends = jfc_engine::config::load_arc().enter_sends_message;
    let ctrl_held = key.modifiers.contains(KeyModifiers::CONTROL);
    let shift_held = key.modifiers.contains(KeyModifiers::SHIFT);

    // Ctrl+Enter always inserts a literal newline (regardless of enter_sends_message).
    if ctrl_held && !shift_held {
        app.textarea.insert_newline();
        return Some(Ok(false));
    }

    // When enter_sends_message = false: bare Enter → newline, only Ctrl+Enter
    // submits. The Ctrl+Enter branch above already handled submit when the flag
    // is false — here we just let bare Enter fall through to newline insertion.
    // (The textarea widget handles Enter naturally when we return None.)
    if !enter_sends && !shift_held && !ctrl_held {
        app.textarea.insert_newline();
        return Some(Ok(false));
    }

    // Default (enter_sends_message = true): bare Enter (no Shift, no Ctrl) submits.
    if key.code == KeyCode::Enter && !key.modifiers.contains(KeyModifiers::SHIFT) {
        let text = app.textarea.lines().join("\n");
        let text = text.trim().to_string();
        if !text.is_empty() {
            // If a hold/tap voice recording is mid-flight, this Enter submits
            // what's already in the box — discard the recording so its eventual
            // Final transcript doesn't auto-submit a duplicate. (The earlier
            // double-send: Enter submitted, then stopping voice sent again.)
            // Leaves VAD's continuous loop alone.
            if app.voice_state != jfc_voice::VoiceState::Idle || app.voice_interim_chars > 0 {
                crate::voice::discard_recording().await;
                app.voice_interim = None;
                app.voice_interim_chars = 0;
                // Suppress late Interim/Final events from this in-flight
                // utterance so they don't re-hydrate the box or auto-submit a
                // duplicate after this manual submit. Cleared on the next
                // Recording onset (see handle_voice_event).
                app.voice_suppress_input = true;
            }
            // Enter-submit entry trace. `submitted_chars` is the source of
            // truth for the prompt-doubling bug: if it arrives already-doubled
            // here, the corruption happened upstream in a recall/insert path
            // (see jfc::input::recall), not in persistence/coalesce.
            tracing::debug!(
                target: "jfc::input::submit",
                submitted_chars = text.chars().count(),
                line_count = app.textarea.lines().len(),
                is_streaming = app.engine.is_streaming,
                streaming_response_bytes = app.engine.streaming_response_bytes,
                editing_idx = ?app.editing_message_idx,
                history_cursor = ?app.history_cursor,
                queued_depth = app.engine.queued_prompts.len(),
                preview = %text.chars().take(48).collect::<String>(),
                "enter_submit: textarea content captured"
            );
            reset_input(app);
            // Slash commands are view/config actions (`/voice off`, `/model`,
            // `/status`, …), not new conversation turns — they must NOT cancel
            // or queue behind an in-flight stream. Routing `/voice off` through
            // the interrupt-on-submit path below aborted the model's response
            // mid-stream (the reported bug). Handle them directly, leaving any
            // active stream untouched.
            if text.starts_with('/') {
                if let Err(e) = handle_submit(app, text, tx).await {
                    return Some(Err(e));
                }
                return Some(Ok(false));
            }
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
            let pipeline_busy = app.engine.pipeline_busy_for_submit();
            let compacting = app.engine.compacting_started_at.is_some();
            if app.engine.is_streaming || pipeline_busy || compacting {
                // Bare Enter queues. Alt+Enter is the explicit "interrupt and
                // steer now" path when the active turn is safe to cancel.
                let explicit_interrupt = key.modifiers.contains(KeyModifiers::ALT);
                let queue_mode = jfc_engine::config::load_arc().message_queue_mode;
                let can_interrupt =
                    can_interrupt_on_submit(app, compacting, explicit_interrupt, queue_mode);
                tracing::debug!(
                    target: "jfc::input",
                    "handle_enter_submit: is_streaming={} pipeline_busy={pipeline_busy} compacting={compacting} explicit={explicit_interrupt} queue_mode={queue_mode} can_interrupt={can_interrupt}",
                    app.engine.is_streaming
                );
                if can_interrupt {
                    tracing::info!(
                        target: "jfc::input::interrupt",
                        "interrupt-on-submit: aborting interruptible stream"
                    );
                    // Cancel the current stream
                    app.engine.cancel_token.cancel();
                    if let Some(handle) = app.engine.active_stream_handle.take() {
                        handle.abort();
                    }
                    app.engine.cancel_token = tokio_util::sync::CancellationToken::new();
                    app.engine
                        .interrupt_flag
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    app.engine.is_streaming = false;
                    app.engine.streaming_started_at = None;
                    app.engine.last_stream_event_at = None;
                    // Don't queue: submit the new turn immediately now that
                    // the interruptible stream has been cancelled.
                    if let Err(e) = handle_submit(app, text, tx).await {
                        return Some(Err(e));
                    }
                } else {
                    queue_prompt_for_later(app, text);
                } // end else (not can_interrupt)
            } else {
                if let Err(e) = handle_submit(app, text, tx).await {
                    return Some(Err(e));
                }
            }
        }
        return Some(Ok(false));
    }
    None
}

/// Queue a prompt behind in-flight work (streaming / busy pipeline /
/// compaction) when it can't be interrupted. Captures any referenced
/// `[Image #N]` attachments onto the queued entry so they re-stage
/// atomically on drain (unreferenced images are left for later prompts),
/// and inserts a `queued` user message so the transcript shows it landed
/// (build_provider_messages* skips queued messages so they don't inflate
/// the current turn).
pub(super) fn queue_prompt_for_later(app: &mut App, text: String) {
    // Expand any `[Pasted #N · …]` chips to their full text BEFORE queuing —
    // the non-queued submit path does this in `handle_submit`, but a queued
    // prompt bypasses that, so without this the placeholder (and the eventually
    // drained turn) would carry the literal chip token instead of what the user
    // pasted. Only consume chips referenced in *this* text; later prompts keep
    // theirs. Mirrors the expansion in `input/submit.rs`.
    let text = if app.pasted_texts.is_empty() {
        text
    } else {
        let mut expanded = text;
        let mut remaining = Vec::new();
        for (chip, content) in std::mem::take(&mut app.pasted_texts) {
            if expanded.contains(&chip) {
                expanded = expanded.replace(&chip, &content);
            } else {
                remaining.push((chip, content));
            }
        }
        app.pasted_texts = remaining;
        expanded
    };
    let is_meta = text.starts_with('/');
    tracing::info!(
        target: "jfc::ui::queue",
        depth = app.engine.queued_prompts.len() + 1,
        is_meta,
        "queued_prompt"
    );
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
    app.engine.queued_prompts.push(crate::app::QueuedPrompt {
        text: text.clone(),
        is_meta,
        priority: crate::app::QueuePriority::Later,
        attachments,
    });
    app.engine.messages.push(ChatMessage::user_queued(
        jfc_core::queued_prompt_placeholder(&text, is_meta),
    ));
    app.scroll_to_bottom();
}

/// Transcript-search (`Ctrl+F`) key handling. Active only while
/// `app.transcript_search` is set; returns `Some(Ok(false))` for every
/// key in that mode and `None` otherwise. Extracted from `handle_key`.
/// Ctrl+R reverse-history search modal. Consumes all keys while open.
/// Char/Backspace edit the query; Up/Down (and Ctrl+R) move the selection;
/// Enter loads the highlighted prompt into the input (the user can then edit
/// or submit); Esc cancels.
fn handle_prompt_search_keys(app: &mut App, key: event::KeyEvent) -> Option<anyhow::Result<bool>> {
    app.prompt_search.as_ref()?;
    match (key.modifiers, key.code) {
        (_, KeyCode::Esc) => {
            app.prompt_search = None;
        }
        (_, KeyCode::Enter) => {
            if let Some(s) = app.prompt_search.take()
                && let Some(text) = s.selected_text().map(str::to_owned)
            {
                app.textarea.select_all();
                app.textarea.cut();
                app.textarea.insert_str(&text);
            }
        }
        (_, KeyCode::Backspace) => {
            if let Some(s) = app.prompt_search.as_mut() {
                s.query.pop();
                s.refilter();
            }
        }
        // Ctrl+R again, or Down, steps to the next (older) match.
        (KeyModifiers::CONTROL, KeyCode::Char('r')) | (_, KeyCode::Down) => {
            if let Some(s) = app.prompt_search.as_mut()
                && !s.results.is_empty()
            {
                s.selected = (s.selected + 1) % s.results.len();
            }
        }
        (_, KeyCode::Up) => {
            if let Some(s) = app.prompt_search.as_mut()
                && !s.results.is_empty()
            {
                s.selected = if s.selected == 0 {
                    s.results.len() - 1
                } else {
                    s.selected - 1
                };
            }
        }
        (m, KeyCode::Char(c))
            if !m.contains(KeyModifiers::CONTROL) && !m.contains(KeyModifiers::ALT) =>
        {
            if let Some(s) = app.prompt_search.as_mut() {
                s.query.push(c);
                s.refilter();
            }
        }
        _ => {}
    }
    Some(Ok(false))
}

fn handle_transcript_search_keys(
    app: &mut App,
    key: event::KeyEvent,
) -> Option<anyhow::Result<bool>> {
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
                if let Some(s) = app.transcript_search.take()
                    && let Some(&idx) = s.matches.get(s.cursor)
                {
                    scroll_to_message(app, idx);
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
                if let Some(s) = app.transcript_search.as_mut()
                    && !s.matches.is_empty()
                {
                    s.cursor = (s.cursor + 1) % s.matches.len();
                    let target = s.matches[s.cursor];
                    scroll_to_message(app, target);
                }
            }
            KeyCode::Up => {
                if let Some(s) = app.transcript_search.as_mut()
                    && !s.matches.is_empty()
                {
                    s.cursor = if s.cursor == 0 {
                        s.matches.len() - 1
                    } else {
                        s.cursor - 1
                    };
                    let target = s.matches[s.cursor];
                    scroll_to_message(app, target);
                }
            }
            _ => {}
        }
        return Some(Ok(false));
    }
    None
}

/// Up/Down arrow history-recall match. Arms always `return`, so this
/// returns `Some(result)` when a binding fired and `None` to fall
/// through. Extracted from `handle_key` for cohesion.
fn handle_arrow_history_keys(app: &mut App, key: event::KeyEvent) -> Option<anyhow::Result<bool>> {
    match (key.modifiers, key.code) {
        (KeyModifiers::NONE, KeyCode::Up) => {
            // Up at empty input → recall previous user prompt. Multiple
            // presses cycle backwards through history. Mirrors v126's
            // `useArrowKeyHistory` (cli.js) — quality-of-life win for
            // resending or editing recent submissions.
            if !input_has_text(app)
                && let Some(prompt) = recall_previous_prompt(app)
            {
                let before_chars = textarea_char_len(app);
                app.textarea =
                    TextArea::from(prompt.lines().map(str::to_string).collect::<Vec<_>>());
                app.textarea.set_cursor_line_style(Style::default());
                app.textarea.set_placeholder_text("");
                app.textarea.move_cursor(CursorMove::End);
                tracing::debug!(
                    target: "jfc::input::recall",
                    path = "arrow_history_prev",
                    before_chars,
                    recalled_chars = prompt.chars().count(),
                    after_chars = textarea_char_len(app),
                    history_cursor = ?app.history_cursor,
                    "recall previous prompt (TextArea::from replace)"
                );
                return Some(Ok(false));
            }
            move_input_cursor_visual_up(app);
            Some(Ok(false))
        }
        (KeyModifiers::NONE, KeyCode::Down) => {
            // Symmetric to Up — cycle forward through history when the
            // user has recalled a past prompt. When `history_cursor` is
            // None or at the live edit, falls through to cursor move.
            if app.history_cursor.is_some() {
                if let Some(prompt) = recall_next_prompt(app) {
                    let before_chars = textarea_char_len(app);
                    app.textarea =
                        TextArea::from(prompt.lines().map(str::to_string).collect::<Vec<_>>());
                    app.textarea.set_cursor_line_style(Style::default());
                    app.textarea.set_placeholder_text("");
                    app.textarea.move_cursor(CursorMove::End);
                    tracing::debug!(
                        target: "jfc::input::recall",
                        path = "arrow_history_next",
                        before_chars,
                        recalled_chars = prompt.chars().count(),
                        after_chars = textarea_char_len(app),
                        history_cursor = ?app.history_cursor,
                        "recall next prompt (TextArea::from replace)"
                    );
                    return Some(Ok(false));
                } else {
                    // Cycled past the most recent — return to empty input.
                    app.history_cursor = None;
                    reset_input(app);
                    return Some(Ok(false));
                }
            }
            // ↓ at empty input with alive sub-agents → enter agent
            // select. Matches Claude Code's "↓ to manage" hint at the
            // top of the agent card. The user no longer needs the
            // Ctrl+X leader chord to dive into the fan — same muscle
            // memory as VS Code's command-palette `↓` to reach the
            // results list.
            if !input_has_text(app)
                && app.task_panel.viewing_task_id.is_none()
                && app
                    .engine
                    .background_tasks
                    .values()
                    .any(|bt| bt.status.is_alive())
            {
                // Pick the most-recent alive agent (matches the
                // existing `↓ jump to latest` semantics inside the
                // task view).
                let mut alive_ids: Vec<String> = app
                    .engine
                    .background_tasks
                    .iter()
                    .filter(|(_, bt)| bt.status.is_alive())
                    .map(|(id, _)| id.clone())
                    .collect();
                alive_ids.sort();
                if let Some(latest) = alive_ids.last().cloned() {
                    app.task_panel.viewing_task_id = Some(latest);
                    app.scroll_to_bottom();
                }
                return Some(Ok(false));
            }
            move_input_cursor_visual_down(app);
            Some(Ok(false))
        }
        _ => None,
    }
}

/// leader-key (`,` prefix) command keys; active only while `app.leader_key_active`. Returns `Some(result)` when handled, `None` to fall through.
fn handle_leader_key_keys(
    app: &mut App,
    key: event::KeyEvent,
    tx: &mpsc::Sender<crate::runtime::EngineEvent>,
) -> Option<anyhow::Result<bool>> {
    if app.leader_key_active {
        app.leader_key_active = false;
        app.leader_key_timeout = None;

        let task_ids: Vec<String> = crate::render::fleet_ordered_task_ids(app);
        let task_count = task_ids.len();

        match key.code {
            KeyCode::Esc => {}
            KeyCode::Down | KeyCode::Char('j') => {
                if task_count > 0 {
                    let current_pos = app
                        .task_panel
                        .viewing_task_id
                        .as_ref()
                        .and_then(|id| task_ids.iter().position(|t| t == id));
                    let next = match current_pos {
                        None => 0,
                        Some(i) => (i + 1).min(task_count - 1),
                    };
                    app.task_panel.viewing_task_id = task_ids.into_iter().nth(next);
                    app.scroll_to_bottom();
                }
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.task_panel.viewing_task_id = None;
                app.scroll_to_bottom();
            }
            KeyCode::Left | KeyCode::Char('h') => {
                if let Some(ref id) = app.task_panel.viewing_task_id.clone() {
                    let pos = task_ids.iter().position(|t| t == id).unwrap_or(0);
                    if pos > 0 {
                        app.task_panel.viewing_task_id = task_ids.into_iter().nth(pos - 1);
                    }
                }
            }
            KeyCode::Right | KeyCode::Char('l') => {
                if let Some(ref id) = app.task_panel.viewing_task_id.clone() {
                    let pos = task_ids.iter().position(|t| t == id).unwrap_or(0);
                    if pos + 1 < task_count {
                        app.task_panel.viewing_task_id = task_ids.into_iter().nth(pos + 1);
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
                if let Some(id) = app.task_panel.viewing_task_id.clone()
                    && let Some(bt) = app.engine.background_tasks.get_mut(&id)
                    && matches!(
                        bt.status,
                        jfc_core::TaskLifecycle::Running | jfc_core::TaskLifecycle::Idle
                    )
                {
                    bt.status = jfc_core::TaskLifecycle::Failed;
                    bt.error = Some("cancelled by user".into());
                    jfc_engine::toast::push_with_cap(
                        &mut app.engine.toasts,
                        jfc_engine::toast::Toast::new(
                            jfc_engine::toast::ToastKind::Warning,
                            format!("Cancelled task {id}"),
                        ),
                    );
                }
            }
            // `r` retries: re-queue the original task description as a
            // fresh user prompt so the leader dispatches a new agent.
            KeyCode::Char('r') => {
                if let Some(id) = app.task_panel.viewing_task_id.clone()
                    && let Some(bt) = app.engine.background_tasks.get(&id)
                {
                    let prompt = bt.description.clone();
                    let tx_clone = tx.clone();
                    tokio::spawn(async move {
                        let _ = tx_clone
                            .send(crate::runtime::EngineEvent::Control(
                                crate::runtime::ControlEvent::SubmitPrompt(prompt),
                            ))
                            .await;
                    });
                    jfc_engine::toast::push_with_cap(
                        &mut app.engine.toasts,
                        jfc_engine::toast::Toast::new(
                            jfc_engine::toast::ToastKind::Info,
                            format!("Retrying task {id}"),
                        ),
                    );
                }
            }
            // `z` detaches the in-flight FOREGROUND bash command to the
            // background (Claude Code's Ctrl+B). The engine flips the running
            // tool to a background task and toasts the new id. No-op if nothing
            // is running. (`b` opens the shell picker; both live under Ctrl+X.)
            KeyCode::Char('z') => {
                let tx_clone = tx.clone();
                tokio::spawn(async move {
                    let _ = tx_clone
                        .send(crate::runtime::EngineEvent::Control(
                            crate::runtime::ControlEvent::BackgroundForegroundBash,
                        ))
                        .await;
                });
            }
            _ => {}
        }
        return Some(Ok(false));
    }
    None
}

/// Up-arrow queued-prompt recall when the textarea is empty. Returns `Some(result)` when handled, `None` to fall through.
fn handle_up_recall_keys(app: &mut App, key: event::KeyEvent) -> Option<anyhow::Result<bool>> {
    if key.code == KeyCode::Up
        && key.modifiers == KeyModifiers::NONE
        && !app.engine.queued_prompts.is_empty()
        && app.textarea.lines().iter().all(|l| l.is_empty())
        && let Some(qp) = app.engine.queued_prompts.pop_back()
    {
        let placeholder = jfc_core::queued_prompt_placeholder(&qp.text, qp.is_meta);
        // Remove the matching placeholder user message (last occurrence).
        for i in (0..app.engine.messages.len()).rev() {
            if app.engine.messages[i].role == Role::User
                && app.engine.messages[i]
                    .parts
                    .iter()
                    .any(|p| matches!(p, MessagePart::Text(t) if t == &placeholder))
            {
                let streaming_before = app.engine.streaming_assistant_idx;
                let editing_before = app.editing_message_idx;
                app.engine.messages.remove(i);
                // Removing a message shifts every subsequent index down
                // by one. `streaming_assistant_idx` would otherwise point
                // one slot past the live assistant if a fresh sub-stream
                // already staged a slot after the queued user (agentic
                // continuation, pause_turn resume). A stale index lets
                // `StreamEvent::Tool` push `MessagePart::Tool` into a
                // `Role::User` message → API 400 on the next request:
                // "tool_use blocks can only appear in assistant messages".
                // Reproduced as session ses_20260516_071052 msg[20]/msg[21].
                if let Some(streaming_idx) = app.engine.streaming_assistant_idx
                    && i < streaming_idx
                {
                    app.engine.streaming_assistant_idx = Some(streaming_idx - 1);
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
                    message_count = app.engine.messages.len(),
                    streaming_before = ?streaming_before,
                    streaming_after = ?app.engine.streaming_assistant_idx,
                    editing_before = ?editing_before,
                    editing_after = ?app.editing_message_idx,
                    is_streaming = app.engine.is_streaming,
                    "up_recall: removed queued placeholder, adjusted indices"
                );
                break;
            }
        }
        // Recall into the textarea. CLEAR FIRST: the entry guard only checks
        // that every *line* is empty, which a single blank line satisfies —
        // but `insert_str` writes at the cursor, so any residual content (or
        // a prior un-submitted recall) would be APPENDED to, doubling the
        // text. This was the prompt-doubling regression: recall → recall →
        // submit produced `phasesalright…` (two copies, no separator) and
        // compounded each cycle (56 → 112 → 224 chars). Replace, don't append.
        let before_chars = textarea_char_len(app);
        reset_input(app);
        for line in qp.text.split('\n') {
            app.textarea.insert_str(line);
            app.textarea.insert_newline();
        }
        // Drop the trailing newline added by the loop's last iteration.
        // tui-textarea's `delete_line_by_end` after a final newline
        // removes the empty trailing line cleanly.
        app.textarea.delete_line_by_end();
        let after_chars = textarea_char_len(app);
        tracing::debug!(
            target: "jfc::input::recall",
            path = "up_recall_queued",
            before_chars,
            recalled_chars = qp.text.chars().count(),
            after_chars,
            queued_remaining = app.engine.queued_prompts.len(),
            is_meta = qp.is_meta,
            "recall queued prompt into textarea (cleared before insert)"
        );
        if before_chars > 0 {
            tracing::warn!(
                target: "jfc::input::recall",
                before_chars,
                "up_recall fired with a non-empty textarea — guard let residual text through; \
                 reset prevented a double-insert"
            );
        }
        return Some(Ok(false));
    }
    None
}

/// `Ctrl+Y` yank-last-assistant-message-to-clipboard. Returns `Some(result)` when handled, `None` to fall through.
fn handle_yank_key(app: &mut App, key: event::KeyEvent) -> Option<anyhow::Result<bool>> {
    if key.modifiers == KeyModifiers::CONTROL && key.code == KeyCode::Char('y') {
        if let Some(text) = crate::runtime::last_assistant_text(app).filter(|s| !s.is_empty()) {
            // Single funnel: the owner thread keeps the clipboard handle
            // alive (survives X11/Wayland) and emits OSC 52 for SSH/tmux.
            crate::runtime::copy_to_clipboard(&text, "yank");
        }
        return Some(Ok(false));
    }
    None
}

/// Slash-command autocomplete popup keys (Tab/Enter/Up/Down/Esc).
/// Returns `Some(Ok(false))` for arms that consume the key; `None` for
/// the deliberate fall-through arms (Enter-on-exact-match, Esc, other
/// keys) which still mutate `app.slash_popup_selected` before yielding.
fn handle_slash_popup_keys(app: &mut App, key: event::KeyEvent) -> Option<anyhow::Result<bool>> {
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
                    return Some(Ok(false));
                }
                KeyCode::Down => {
                    let idx = app.slash_popup_selected.unwrap_or(0);
                    app.slash_popup_selected = Some((idx + 1) % matches.len());
                    return Some(Ok(false));
                }
                KeyCode::Up => {
                    let idx = app.slash_popup_selected.unwrap_or(0);
                    app.slash_popup_selected =
                        Some(if idx == 0 { matches.len() - 1 } else { idx - 1 });
                    return Some(Ok(false));
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
    None
}
