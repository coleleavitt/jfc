//! Terminal input event handler — Key, Paste, Mouse, and catch-all.
//!
//! `handle_term_event` returns `Ok(true)` when the user requested quit.

use crossterm::event::{Event, KeyEventKind};

use crate::app::{App, SelectKind, SelectRequest, TextSelection};
use crate::runtime::EventSender;
use crate::{attachments, input, message_view};
use jfc_core::*;

/// Max gap between clicks to count as a multi-click (word/line select).
const MULTI_CLICK_MS: u128 = 500;
/// Max cell distance between clicks to count as a multi-click.
const MULTI_CLICK_DIST: u16 = 1;

/// Dispatch a single crossterm `Event` into the appropriate handler.
/// Returns `Ok(true)` when the app should quit.
pub(crate) async fn handle_term_event(
    app: &mut App,
    ev: Event,
    tx: &EventSender,
) -> anyhow::Result<bool> {
    match ev {
        Event::Key(k) if matches!(k.kind, KeyEventKind::Press | KeyEventKind::Repeat) => {
            if input::handle_key(app, k, tx).await? {
                return Ok(true);
            }
        }
        // Push-to-talk key-up: end a hold-mode recording. Relies on the Kitty
        // keyboard protocol's release reports (enabled in `cli::terminal`); only
        // the bare Space key drives PTT, and the recorder ignores releases that
        // don't apply (tap/VAD modes), so this is safe to call unconditionally.
        Event::Key(k)
            if k.kind == KeyEventKind::Release
                && app.voice_enabled
                && crate::voice::is_initialized()
                && k.code == crossterm::event::KeyCode::Char(' ')
                && k.modifiers == crossterm::event::KeyModifiers::NONE =>
        {
            crate::voice::activate(false).await;
        }
        Event::Paste(text) => {
            // Try image clipboard first — when the user pastes a
            // screenshot the OS sends a bracketed-paste *event*
            // with empty/garbage text, but the actual image is
            // available via arboard's `get_image()`. If that
            // succeeds we attach it; otherwise fall through to
            // the text path. Mirrors v126's clipboard-image flow.
            let attached_image = match attachments::read_clipboard_image() {
                Ok(Some((att, w, h))) => {
                    app.image_counter += 1;
                    let id = app.image_counter;
                    app.pasted_images.push(crate::attachments::PastedContent {
                        id,
                        attachment: att,
                        width: w,
                        height: h,
                    });
                    app.textarea.insert_str(format!("[Image #{id}]"));
                    true
                }
                Ok(None) => false,
                Err(e) => {
                    tracing::debug!(target: "jfc::input", error = %e, "image paste check failed");
                    false
                }
            };
            // Pasted/dragged image *file paths*: terminals deliver a dragged
            // file as its path string, not the bytes. Detect image paths
            // (newline- or space-separated for multi-file drags), attach each
            // from disk like a clipboard image, and strip them from the text
            // so only the non-image remainder lands in the prompt. Mirrors
            // Claude Code's `usePasteHandler` multi-image handling.
            let image_paths = attachments::image_paths_in_paste(&text);
            let mut text = text;
            if !image_paths.is_empty() {
                let mut attached_any = false;
                for p in &image_paths {
                    match attachments::read_image_file(std::path::Path::new(p)) {
                        Ok((att, w, h)) => {
                            app.image_counter += 1;
                            let id = app.image_counter;
                            app.pasted_images.push(crate::attachments::PastedContent {
                                id,
                                attachment: att,
                                width: w,
                                height: h,
                            });
                            // Replace the path occurrence with the chip token.
                            text = text.replace(p, &format!("[Image #{id}]"));
                            attached_any = true;
                        }
                        Err(e) => {
                            tracing::debug!(target: "jfc::input", path = %p, error = %e, "image-path paste failed; leaving as text");
                        }
                    }
                }
                if attached_any {
                    tracing::debug!(
                        target: "jfc::input::paste",
                        attached = image_paths.len(),
                        "attached image path(s) from paste"
                    );
                }
            }
            // Always insert the text — it may be a path or
            // contextual prose alongside the image. Large pastes should
            // remain editable in the textarea instead of collapsing to an
            // opaque chip. Preserve the old chip behavior behind a config
            // gate for users who prefer it.
            if !attached_image || !text.is_empty() {
                const PASTE_LINE_THRESHOLD: usize = 8;
                const PASTE_CHAR_THRESHOLD: usize = 400;
                let n_lines = text.lines().count();
                let n_chars = text.chars().count();
                let collapse = jfc_engine::config::load_arc().collapse_large_pastes;
                if collapse && (n_lines > PASTE_LINE_THRESHOLD || n_chars > PASTE_CHAR_THRESHOLD) {
                    app.paste_counter += 1;
                    let id = app.paste_counter;
                    let label = if n_lines > 1 {
                        format!("{n_lines} lines")
                    } else {
                        format!("{n_chars} chars")
                    };
                    let chip = format!("[Pasted #{id} · {label}]");
                    app.pasted_texts.push((chip.clone(), text.clone()));
                    app.textarea.insert_str(&chip);
                } else {
                    // Insert full pasted text so it stays editable.
                    app.textarea.insert_str(&text);
                }
            }
        }
        Event::Mouse(mouse) => {
            handle_mouse(app, mouse, tx).await;
        }
        // A resize remaps every screen cell, so a persisted selection
        // highlight (absolute cells) would point at the wrong content — drop it.
        Event::Resize(..) => {
            app.text_selection = None;
        }
        Event::FocusGained => {
            handle_focus_gained(app, tx);
        }
        Event::FocusLost => {}
        _ => {}
    }
    Ok(false)
}

/// On terminal refocus, remember the moment so focus storms stay cheap.
///
/// Older builds probed the clipboard here and showed a passive image hint.
/// That was noisy and could block on clipboard backends; paste itself still
/// probes and attaches images on Ctrl+V.
fn handle_focus_gained(app: &mut App, _tx: &EventSender) {
    const FOCUS_HINT_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);
    let now = std::time::Instant::now();
    if app
        .last_focus_hint_at
        .is_some_and(|t| now.duration_since(t) < FOCUS_HINT_COOLDOWN)
    {
        return;
    }
    app.last_focus_hint_at = Some(now);
}

/// Handle mouse events: scroll, drag, click.
async fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent, _tx: &EventSender) {
    use crossterm::event::{MouseButton, MouseEventKind};
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.scroll_velocity = 0.0;
            app.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            app.scroll_velocity = 0.0;
            app.scroll_down(3);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // Drag inside the transcript extends a text selection (copy-on-
            // select). The renderer paints the highlight and, on button-up,
            // copies the covered content. Coordinates are (col, content line):
            // the row is translated through the current scroll offset so the
            // selection survives scrolling mid-drag and afterwards.
            let content_line = selection_content_line(app, mouse.row);
            if let Some(sel) = app.text_selection.as_mut() {
                sel.head = (mouse.column, content_line);
                // Only promote to a real selection once the cursor has moved
                // a meaningful distance. A one-cell jitter on an ordinary
                // click must NOT count as a drag — otherwise a shaky click
                // loses its click action (tool expand, group toggle) and
                // copies a stray character instead.
                if !sel.dragged && selection_started(sel.anchor, sel.head) {
                    sel.dragged = true;
                }
            }
            // Drag-edge autoscroll: when the cursor is dragged to/past the top
            // or bottom edge of the transcript, record how far beyond the edge
            // it is so the throttled tick can keep scrolling + extending the
            // selection. Without this a drag stalls at the viewport edge and
            // can never select content that scrolled offscreen (the original
            // "copy is limited to the visible area" report).
            app.drag_autoscroll = app
                .text_selection
                .filter(|s| s.dragged)
                .and_then(|_| drag_autoscroll_overrun(app, mouse.row));
        }
        // Only the *left* button drives selection. Matching `Up(_)` here let a
        // right/middle-button release finalize or drop a left-drag selection.
        MouseEventKind::Up(MouseButton::Left) => {
            app.drag_anchor_y = None;
            // The drag is over — stop any edge autoscroll.
            app.drag_autoscroll = None;
            match app.text_selection {
                // A real drag: hand off to the renderer to extract + copy the
                // covered cells (it has the buffer; this handler does not).
                Some(sel) if sel.dragged => {
                    app.text_selection = Some(TextSelection {
                        finalize: true,
                        ..sel
                    });
                }
                // No drag → it was a click. Run the click action now (on
                // release, standard click semantics) and drop the selection.
                Some(_) => {
                    app.text_selection = None;
                    handle_left_click(app, mouse).await;
                }
                None => {}
            }
        }
        // Press anchors a potential selection but defers the click action to
        // release, so starting a drag doesn't also fire a click (e.g. yank).
        MouseEventKind::Down(MouseButton::Left) => {
            // Fresh gesture — clear any leftover edge-autoscroll signal.
            app.drag_autoscroll = None;
            // `copy_on_select = false` disables the drag-to-select gesture
            // entirely: clicks fall straight through to the click handler and
            // the clipboard is never touched by a drag.
            let copy_on_select = jfc_engine::config::load_arc().copy_on_select;
            let in_messages = copy_on_select
                && app.messages_rect.borrow().as_ref().is_some_and(|r| {
                    mouse.column >= r.x
                        && mouse.column < r.x + r.width
                        && mouse.row >= r.y
                        && mouse.row < r.y + r.height
                });
            if in_messages {
                // Track click count for word (double) / line (triple) select.
                // A click on a tool block is excluded — it keeps its own
                // double-click-to-pin behavior (run on release in
                // handle_left_click), so word/line select only applies to
                // plain transcript text.
                let on_tool = message_view::find_tool_at(
                    &app.tool_hit_regions.borrow(),
                    mouse.column,
                    mouse.row,
                )
                .is_some();
                let now = std::time::Instant::now();
                let count = match app.last_click {
                    Some((c, r, n, t))
                        if now.duration_since(t).as_millis() < MULTI_CLICK_MS
                            && c.abs_diff(mouse.column) <= MULTI_CLICK_DIST
                            && r.abs_diff(mouse.row) <= MULTI_CLICK_DIST =>
                    {
                        n.saturating_add(1)
                    }
                    _ => 1,
                };
                app.last_click = Some((mouse.column, mouse.row, count, now));
                if !on_tool && count >= 2 {
                    // Renderer resolves the word/line span against the buffer.
                    let kind = if count >= 3 {
                        SelectKind::Line
                    } else {
                        SelectKind::Word
                    };
                    app.pending_select_request = Some(SelectRequest {
                        col: mouse.column,
                        row: mouse.row,
                        kind,
                    });
                    // The request owns this frame; don't also start a drag.
                    app.text_selection = None;
                } else {
                    let line = selection_content_line(app, mouse.row);
                    app.text_selection = Some(TextSelection {
                        anchor: (mouse.column, line),
                        head: (mouse.column, line),
                        area_width: app.messages_rect.borrow().map(|r| r.width).unwrap_or(0),
                        dragged: false,
                        finalize: false,
                        copied: false,
                    });
                }
            } else {
                // A click outside the transcript dismisses any persisted
                // post-copy highlight before running the click action.
                app.text_selection = None;
                handle_left_click(app, mouse).await;
            }
        }
        _ => {}
    }
}

/// Handle left-click: tool toggle, toast dismiss, sidebar session load.
async fn handle_left_click(app: &mut App, mouse: crossterm::event::MouseEvent) {
    // First, see if the click landed on a tool block —
    // each visible tool is registered in
    // `app.tool_hit_regions` by the renderer. Toggling
    // `expanded` flips the body between preview and
    // full content. Mirrors v126's per-tool expand
    // affordance (cmd-click on iTerm2; we use a plain
    // click since non-iTerm terminals don't surface
    // the cmd modifier the same way).
    let copy_hit =
        message_view::find_tool_at(&app.tool_copy_regions.borrow(), mouse.column, mouse.row)
            .map(str::to_owned);
    let hit = message_view::find_tool_at(&app.tool_hit_regions.borrow(), mouse.column, mouse.row)
        .map(str::to_owned);
    // Toast click → dismiss. The overlay renders newest first inside a
    // bordered strip, so translate the clicked body row back to the matching entry
    // in the full toast queue.
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
        .and_then(|r| {
            let local = mouse.row.saturating_sub(r.y);
            (local > 0 && local + 1 < r.height).then(|| (local - 1) as usize)
        });
    if let Some(local_row) = toast_hit {
        let visible_indices = app
            .engine
            .toasts
            .iter()
            .enumerate()
            .rev()
            .map(|(idx, _)| idx)
            .collect::<Vec<_>>();
        if let Some(drop_idx) = visible_indices.get(local_row).copied() {
            app.engine.toasts.remove(drop_idx);
        }
        return;
    }

    let input_hit = app.input_rect.borrow().as_ref().is_some_and(|r| {
        mouse.column >= r.x
            && mouse.column < r.x + r.width
            && mouse.row >= r.y
            && mouse.row < r.y + r.height
    });
    if input_hit {
        let text = app.textarea.lines().join("\n");
        if !text.trim().is_empty() {
            crate::runtime::copy_to_clipboard(&text, "current-input");
        }
        return;
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
        if !app.session_sidebar.meta.is_empty() {
            let cwd = app.engine.cwd.clone();
            let (this_project, other) =
                jfc_session::group_by_cwd(app.session_sidebar.meta.clone(), Some(cwd.as_str()));
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
                let ordered: Vec<jfc_engine::ids::SessionId> = this_project
                    .into_iter()
                    .chain(other)
                    .map(|s| s.id)
                    .collect();
                if let Some(id) = ordered.get(idx).cloned()
                    && let Some(messages) = jfc_engine::session::load_session(&id).await
                {
                    app.engine.messages = messages;
                    app.switch_session(Some(id));
                    app.engine.streaming_text = String::new();
                    app.engine.streaming_reasoning = String::new();
                    app.engine.streaming_response_bytes = 0;
                    app.engine.streaming_response_baseline = 0;
                    app.engine.streaming_thinking_tokens = 0;
                    app.engine.token_rate_samples.clear();
                    app.engine.token_rate_sample_thinking = None;
                    app.engine.streaming_assistant_idx = None;
                    app.session_sidebar.selected = idx;
                    app.session_sidebar.list.select(Some(idx));
                    app.scroll_to_bottom();
                    handled_in_sidebar = true;
                }
            }
        }
    }
    if handled_in_sidebar {
        // Sidebar consumed the click; skip the
        // tool/yank fallthrough.
    } else if let Some(tool_id) = copy_hit {
        if let Some(command) = bash_command_for_tool(app, &tool_id).map(str::to_owned) {
            crate::runtime::copy_to_clipboard(&command, "bash-command");
            app.last_tool_click = None;
        }
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
        let is_double_click = matches!(
            &app.last_tool_click,
            Some((prev_id, prev_at))
                if prev_id == &tool_id
                    && now.duration_since(*prev_at).as_millis() < DOUBLE_CLICK_MS
        );
        for msg in &mut app.engine.messages {
            for part in &mut msg.parts {
                if let MessagePart::Tool(tc) = part
                    && tc.id == tool_id
                {
                    if is_double_click {
                        // Toggle pin. Pinning
                        // forces expanded; unpinning
                        // leaves cap state as-is so
                        // the user can collapse with
                        // a subsequent single click.
                        tc.display.toggle_pinned();
                    } else {
                        tc.display.toggle_expanded();
                    }
                }
            }
        }
        app.last_tool_click = Some((tool_id, now));
    }
    // A plain click on empty transcript space does nothing. It used to yank
    // the whole last assistant message to the clipboard, which was surprising
    // (any stray click silently overwrote the clipboard). Explicit copy lives
    // on Ctrl+Y and drag-to-select.
}

fn bash_command_for_tool<'a>(app: &'a App, tool_id: &str) -> Option<&'a str> {
    app.engine.messages.iter().find_map(|msg| {
        msg.parts.iter().find_map(|part| {
            let MessagePart::Tool(tc) = part else {
                return None;
            };
            if tc.id != tool_id {
                return None;
            }
            let ToolInput::Bash { command, .. } = &tc.input else {
                return None;
            };
            Some(command.as_str())
        })
    })
}

/// Whether a drag has moved far enough from its anchor to count as a real
/// text selection rather than click jitter. Any row change, or ≥2 columns
/// horizontally on the same row, qualifies.
fn selection_started(anchor: (u16, usize), head: (u16, usize)) -> bool {
    const MIN_DRAG_COLS: u16 = 2;
    anchor.1 != head.1 || anchor.0.abs_diff(head.0) >= MIN_DRAG_COLS
}

/// Translate a screen row inside the transcript into a scroll-invariant
/// absolute content line: `scroll_offset + (row − area.top)`. Selections are
/// stored in these coordinates so they survive scrolling.
fn selection_content_line(app: &App, row: u16) -> usize {
    let top = app.messages_rect.borrow().map(|r| r.y).unwrap_or(0);
    app.scroll_offset + row.saturating_sub(top) as usize
}

/// How far (in rows) a drag cursor has overrun the transcript's top/bottom
/// edge, or `None` when it's inside the viewport. Negative = above the top
/// edge (autoscroll up); positive = below the bottom edge (autoscroll down).
/// Drives [`crate::app::App::drag_autoscroll`] so the tick keeps scrolling
/// while the cursor is pinned past an edge — letting a drag select content
/// beyond the visible area.
fn drag_autoscroll_overrun(app: &App, row: u16) -> Option<i32> {
    let area = (*app.messages_rect.borrow())?;
    let top = area.y;
    let bottom = area.y + area.height; // exclusive (last content row = bottom-1)
    if row < top || (row == top && app.scroll_offset > 0) {
        Some((i32::from(row) - i32::from(top)).min(-1)) // negative
    } else if row >= bottom {
        Some(i32::from(row) - i32::from(bottom) + 1) // positive (>=1)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::{drag_autoscroll_overrun, handle_mouse, selection_started};
    use crate::app::App;
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use std::sync::Arc;

    #[test]
    fn one_cell_jitter_is_not_a_selection_normal() {
        // A click that wobbles one column must stay a click.
        assert!(!selection_started((10, 5), (10, 5)));
        assert!(!selection_started((10, 5), (11, 5)));
        assert!(!selection_started((10, 5), (9, 5)));
    }

    #[test]
    fn real_drag_starts_selection_normal() {
        // Two+ columns on the same row, or any row change, counts.
        assert!(selection_started((10, 5), (12, 5)));
        assert!(selection_started((10, 5), (8, 5)));
        assert!(selection_started((10, 5), (10, 6)));
        assert!(selection_started((10, 5), (10, 4)));
    }

    struct StubProvider;
    #[async_trait::async_trait]
    impl Provider for StubProvider {
        fn name(&self) -> &str {
            "stub"
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
    impl jfc_provider::seal::Sealed for StubProvider {}

    #[tokio::test]
    async fn mouse_wheel_scroll_is_direct_not_kinetic_regression() {
        let mut app = App::new(Arc::new(StubProvider), "test-model");
        app.total_lines = 100;
        app.viewport_height = 20;
        app.scroll_offset = 8;
        app.scroll_velocity = 42.0;
        let (tx, _rx) = tokio::sync::mpsc::channel(1);

        handle_mouse(
            &mut app,
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::ScrollUp,
                column: 0,
                row: 0,
                modifiers: crossterm::event::KeyModifiers::empty(),
            },
            &tx,
        )
        .await;
        assert_eq!(app.scroll_offset, 5);
        assert_eq!(app.scroll_velocity, 0.0);

        app.scroll_velocity = -42.0;
        handle_mouse(
            &mut app,
            crossterm::event::MouseEvent {
                kind: crossterm::event::MouseEventKind::ScrollDown,
                column: 0,
                row: 0,
                modifiers: crossterm::event::KeyModifiers::empty(),
            },
            &tx,
        )
        .await;
        assert_eq!(app.scroll_offset, 8);
        assert_eq!(app.scroll_velocity, 0.0);
    }

    // Regression: clicking a toast's top border row maps to `local == 0`. The
    // old click handler computed `local - 1` on an unsigned row, so a click on
    // that border underflowed and panicked ("attempt to subtract with
    // overflow"). The guarded handler must treat border rows as no-ops (toast
    // stays) and only dismiss when a body row is clicked.
    #[tokio::test]
    async fn toast_border_click_does_not_underflow_regression() {
        let mut app = App::new(Arc::new(StubProvider), "test-model");
        let mut toast = jfc_engine::toast::Toast::new(jfc_engine::toast::ToastKind::Error, "boom");
        toast.created_at -= std::time::Duration::from_secs(1);
        jfc_engine::toast::push_with_cap(&mut app.engine.toasts, toast);
        // Bordered strip at (40,10) sized 30x4: row 10 = top border (local 0),
        // rows 11-12 = body, row 13 = bottom border.
        *app.toasts_rect.borrow_mut() = Some(ratatui::layout::Rect::new(40, 10, 30, 4));
        let (tx, _rx) = tokio::sync::mpsc::channel(1);

        let click = |row: u16| crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: 45,
            row,
            modifiers: crossterm::event::KeyModifiers::empty(),
        };

        // Top border: previously panicked; now a no-op that keeps the toast.
        handle_mouse(&mut app, click(10), &tx).await;
        assert_eq!(
            app.engine.toasts.len(),
            1,
            "clicking the toast border must not dismiss or panic"
        );

        // Body row dismisses the toast.
        handle_mouse(&mut app, click(11), &tx).await;
        assert!(
            app.engine.toasts.is_empty(),
            "clicking a toast body row should dismiss it"
        );
    }

    // Drag-edge autoscroll overrun: rows inside the transcript yield None; rows
    // above the top edge yield a negative overrun (scroll up); rows at/below
    // the bottom edge yield a positive overrun (scroll down). This is the core
    // signal that lets a drag select content past the viewport.
    #[test]
    fn drag_autoscroll_overrun_detects_edges_normal() {
        let app = App::new(Arc::new(StubProvider), "test-model");
        // Messages area: y=2, height=10 → rows [2, 12), last content row = 11.
        *app.messages_rect.borrow_mut() = Some(ratatui::layout::Rect::new(0, 2, 40, 10));

        // Inside the viewport → no autoscroll.
        assert_eq!(drag_autoscroll_overrun(&app, 2), None);
        assert_eq!(drag_autoscroll_overrun(&app, 6), None);
        assert_eq!(drag_autoscroll_overrun(&app, 11), None);

        // Above the top edge → negative (scroll up), magnitude = rows past edge.
        assert_eq!(drag_autoscroll_overrun(&app, 1), Some(-1));
        assert_eq!(drag_autoscroll_overrun(&app, 0), Some(-2));

        // At/below the bottom edge → positive (scroll down), magnitude ≥ 1.
        assert_eq!(drag_autoscroll_overrun(&app, 12), Some(1));
        assert_eq!(drag_autoscroll_overrun(&app, 14), Some(3));
    }

    // Regression: in the normal chat layout the messages area can start at
    // terminal row 0, and mouse rows are unsigned, so a cursor can never be
    // reported as "row < top". A promoted drag pinned to that top edge still
    // has to arm upward autoscroll when scrollback exists; otherwise copying
    // upward stalls while copying downward works.
    #[test]
    fn drag_autoscroll_overrun_top_edge_scrolls_when_transcript_starts_at_zero_regression() {
        let mut app = App::new(Arc::new(StubProvider), "test-model");
        *app.messages_rect.borrow_mut() = Some(ratatui::layout::Rect::new(0, 0, 40, 10));

        assert_eq!(drag_autoscroll_overrun(&app, 0), None);

        app.scroll_offset = 8;
        assert_eq!(drag_autoscroll_overrun(&app, 0), Some(-1));
    }

    // A `Drag` whose cursor is back INSIDE the viewport must clear the
    // autoscroll signal — otherwise the tick would keep scrolling after the
    // user dragged back in. This mirrors the Drag arm's assignment:
    // `drag_autoscroll = selection(dragged).and_then(overrun)`.
    #[test]
    fn drag_autoscroll_clears_when_cursor_reenters_viewport_normal() {
        let app = build_drag_app(true);
        // Cursor inside the viewport → overrun is None regardless of a prior
        // edge overrun, so the Drag arm stores None.
        let reentered = app
            .text_selection
            .filter(|s| s.dragged)
            .and_then(|_| drag_autoscroll_overrun(&app, 6));
        assert_eq!(reentered, None, "in-viewport drag must clear autoscroll");
    }

    // Click jitter past the edge (selection exists but `dragged == false`) must
    // NOT start autoscroll — only a promoted drag-selection does. Guards the
    // `filter(|s| s.dragged)` in the Drag arm.
    #[test]
    fn drag_autoscroll_not_armed_for_undragged_selection_normal() {
        let app = build_drag_app(false); // dragged = false
        let armed = app
            .text_selection
            .filter(|s| s.dragged)
            .and_then(|_| drag_autoscroll_overrun(&app, 0)); // row above top edge
        assert_eq!(armed, None, "an un-dragged selection must not autoscroll");
    }

    /// App with a transcript selection over a recorded messages_rect; `dragged`
    /// controls whether the selection has been promoted to a real drag.
    fn build_drag_app(dragged: bool) -> App {
        let app = App::new(Arc::new(StubProvider), "test-model");
        *app.messages_rect.borrow_mut() = Some(ratatui::layout::Rect::new(0, 2, 40, 10));
        let mut app = app;
        app.text_selection = Some(crate::app::TextSelection {
            anchor: (1, 5),
            head: (5, 5),
            area_width: 40,
            dragged,
            finalize: false,
            copied: false,
        });
        app
    }
}
