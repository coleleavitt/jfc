//! Terminal input event handler — Key, Paste, Mouse, and catch-all.
//!
//! `handle_term_event` returns `Ok(true)` when the user requested quit.

use crossterm::event::{Event, KeyEventKind};

use crate::app::App;
use crate::runtime::EventSender;
use crate::types::*;
use crate::{attachments, input, message_view, toast};

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
        Event::Paste(text) => {
            // Try image clipboard first — when the user pastes a
            // screenshot the OS sends a bracketed-paste *event*
            // with empty/garbage text, but the actual image is
            // available via arboard's `get_image()`. If that
            // succeeds we attach it; otherwise fall through to
            // the text path. Mirrors v126's clipboard-image flow.
            let attached_image = match attachments::read_clipboard_image() {
                Ok(Some((att, w, h))) => {
                    toast::push_with_cap(
                        &mut app.toasts,
                        toast::Toast::new(
                            toast::ToastKind::Info,
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
                    true
                }
                Ok(None) => false,
                Err(e) => {
                    tracing::debug!(target: "jfc::input", error = %e, "image paste check failed");
                    false
                }
            };
            // Always insert the text — it may be a path or
            // contextual prose alongside the image.
            if !attached_image || !text.is_empty() {
                app.textarea.insert_str(&text);
            }
        }
        Event::Mouse(mouse) => {
            handle_mouse(app, mouse, tx).await;
        }
        _ => {}
    }
    Ok(false)
}

/// Handle mouse events: scroll, drag, click.
async fn handle_mouse(app: &mut App, mouse: crossterm::event::MouseEvent, _tx: &EventSender) {
    use crossterm::event::{MouseButton, MouseEventKind};
    match mouse.kind {
        MouseEventKind::ScrollUp => {
            app.scroll_velocity = (app.scroll_velocity - 12.0).max(-120.0);
            app.scroll_up(3);
        }
        MouseEventKind::ScrollDown => {
            app.scroll_velocity = (app.scroll_velocity + 12.0).min(120.0);
            app.scroll_down(3);
        }
        MouseEventKind::Drag(MouseButton::Left) => {
            // Drag-scroll: convert vertical delta into
            // scroll_offset adjustments. Anchor on the
            // first Drag event and re-anchor on every
            // subsequent one so the next delta is one
            // row's worth, not cumulative. Up-drag scrolls
            // up (look at older content); down-drag
            // scrolls down. Gated to the messages area —
            // dragging in the input bar still selects.
            let in_messages = app.messages_rect.borrow().as_ref().is_some_and(|r| {
                mouse.column >= r.x
                    && mouse.column < r.x + r.width
                    && mouse.row >= r.y
                    && mouse.row < r.y + r.height
            });
            if in_messages {
                if let Some(anchor) = app.drag_anchor_y {
                    let delta = mouse.row as i32 - anchor as i32;
                    if delta > 0 {
                        app.scroll_up(delta as usize);
                    } else if delta < 0 {
                        app.scroll_down((-delta) as usize);
                    }
                }
                app.drag_anchor_y = Some(mouse.row);
            }
        }
        MouseEventKind::Up(_) => {
            app.drag_anchor_y = None;
        }
        // Left-click on the message pane copies the assistant
        // message under the cursor to the clipboard. ratatui
        // doesn't expose hit-testing, so we approximate: any
        // click outside the input area + sidebar copies the
        // most recent assistant text. (Full message-by-position
        // hit detection would require tracking each message's
        // y-range during render, which is the next iteration.)
        MouseEventKind::Down(MouseButton::Left) => {
            handle_left_click(app, mouse).await;
        }
        _ => {}
    }
}

/// Handle left-click: tool toggle, toast dismiss, sidebar session load, yank.
async fn handle_left_click(app: &mut App, mouse: crossterm::event::MouseEvent) {
    use crate::runtime::yank_last_assistant;

    // First, see if the click landed on a tool block —
    // each visible tool is registered in
    // `app.tool_hit_regions` by the renderer. Toggling
    // `expanded` flips the body between preview and
    // full content. Mirrors v126's per-tool expand
    // affordance (cmd-click on iTerm2; we use a plain
    // click since non-iTerm terminals don't surface
    // the cmd modifier the same way).
    let hit = message_view::find_tool_at(&app.tool_hit_regions.borrow(), mouse.column, mouse.row)
        .map(str::to_owned);
    // Toast click → dismiss. Toasts render newest-
    // first; row 0 corresponds to the last entry
    // in `app.toasts`, row 1 to the second-to-last,
    // etc. (See `iter().rev().take(h)` in
    // `toast_overlay`.) Pop the matched toast.
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
        .map(|r| mouse.row.saturating_sub(r.y) as usize);
    if let Some(local_row) = toast_hit {
        if local_row < app.toasts.len() {
            let drop_idx = app.toasts.len() - 1 - local_row;
            app.toasts.remove(drop_idx);
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
        if !app.session_meta.is_empty() {
            let cwd = app.cwd.clone();
            let (this_project, other) =
                jfc_session::group_by_cwd(app.session_meta.clone(), Some(cwd.as_str()));
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
                let ordered: Vec<crate::ids::SessionId> = this_project
                    .into_iter()
                    .chain(other)
                    .map(|s| s.id)
                    .collect();
                if let Some(id) = ordered.get(idx).cloned() {
                    if let Some(messages) = crate::session::load_session(&id).await {
                        app.messages = messages;
                        app.switch_session(Some(id));
                        app.streaming_text = String::new();
                        app.streaming_reasoning = String::new();
                        app.streaming_response_bytes = 0;
                        app.streaming_assistant_idx = None;
                        app.session_selected = idx;
                        app.session_list_state.select(Some(idx));
                        app.scroll_to_bottom();
                        handled_in_sidebar = true;
                    }
                }
            }
        }
    }
    if handled_in_sidebar {
        // Sidebar consumed the click; skip the
        // tool/yank fallthrough.
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
        let is_double_click = match &app.last_tool_click {
            Some((prev_id, prev_at))
                if prev_id == &tool_id
                    && now.duration_since(*prev_at).as_millis() < DOUBLE_CLICK_MS =>
            {
                true
            }
            _ => false,
        };
        for msg in &mut app.messages {
            for part in &mut msg.parts {
                if let MessagePart::Tool(tc) = part {
                    if tc.id == tool_id {
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
        }
        app.last_tool_click = Some((tool_id, now));
    } else {
        let in_input = mouse.row as usize
            >= app
                .viewport_height
                .saturating_add(app.scroll_offset)
                .saturating_sub(2);
        if !in_input {
            yank_last_assistant(app);
        }
    }
}
