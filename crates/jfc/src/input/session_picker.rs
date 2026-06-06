//! Session-picker input handler — same shape as `model_picker.rs`.
//! Type to filter, ↑↓ to move, Enter to load the selected session,
//! Esc to close. The picker is opened by Ctrl+P (popular muscle memory
//! from VS Code's quick-open) and runs alongside the legacy Ctrl+B
//! sidebar so users can choose whichever flow they prefer.

use crossterm::event::KeyCode;
use tokio::sync::mpsc;

use crate::app::App;
use crate::render::session_picker::filtered_sessions;
use crate::runtime::{AppEvent, UiEvent, send_critical};

pub(super) fn open_session_picker(app: &mut App) {
    app.show_session_picker = true;
    app.session_picker_filter.clear();
    app.session_picker_state.select(Some(0));
    // Session metadata refresh is handled by the existing Ctrl+B sidebar
    // path (`jfc_session::list_sessions_with_metadata()` is async); the
    // picker reuses the already-cached `app.session_meta`. If the user
    // wants a freshly-rescanned list they hit Ctrl+B once first.
}

pub(super) fn handle_session_picker_key(
    app: &mut App,
    key: crossterm::event::KeyEvent,
    tx: &mpsc::Sender<AppEvent>,
) -> bool {
    if !app.show_session_picker {
        return false;
    }
    let total = filtered_sessions(app).len();
    let current = app.session_picker_state.selected().unwrap_or(0);
    match key.code {
        KeyCode::Esc => {
            close_session_picker(app);
        }
        KeyCode::Enter => {
            let visible = filtered_sessions(app);
            if let Some(meta) = visible.get(current) {
                let chosen = meta.id.clone();
                tracing::info!(
                    target: "jfc::session_picker",
                    session_id = %chosen,
                    "session_picker selected, dispatching async load"
                );
                close_session_picker(app);
                send_critical(tx, AppEvent::Ui(UiEvent::LoadSession(chosen)));
            }
        }
        KeyCode::Up if current > 0 => {
            app.session_picker_state.select(Some(current - 1));
        }
        KeyCode::Down => {
            let max = total.saturating_sub(1);
            if current < max {
                app.session_picker_state.select(Some(current + 1));
            }
        }
        KeyCode::Home => {
            app.session_picker_state.select(Some(0));
        }
        KeyCode::End => {
            let max = total.saturating_sub(1);
            app.session_picker_state.select(Some(max));
        }
        KeyCode::PageUp => {
            app.session_picker_state
                .select(Some(current.saturating_sub(10)));
        }
        KeyCode::PageDown => {
            let max = total.saturating_sub(1);
            app.session_picker_state
                .select(Some((current + 10).min(max)));
        }
        KeyCode::Char(c) => {
            app.session_picker_filter.push(c);
            app.session_picker_state.select(Some(0));
        }
        KeyCode::Backspace => {
            app.session_picker_filter.pop();
            app.session_picker_state.select(Some(0));
        }
        _ => {}
    }
    true
}

fn close_session_picker(app: &mut App) {
    app.show_session_picker = false;
    app.session_picker_filter.clear();
    app.session_picker_state.select(Some(0));
}
