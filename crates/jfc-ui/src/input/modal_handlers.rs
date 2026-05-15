use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::App;
use crate::runtime::AppEvent;

use super::palette::{execute_palette_action, palette_items};
use super::theme_picker::{apply_theme, filtered_theme_choices};

pub(super) async fn handle_modal_key(
    app: &mut App,
    key: event::KeyEvent,
    _tx: &tokio::sync::mpsc::Sender<AppEvent>,
) -> bool {
    if handle_task_panel_key(app, key) {
        return true;
    }
    if handle_sidebar_key(app, key).await {
        return true;
    }
    if handle_palette_key(app, key).await {
        return true;
    }
    if handle_theme_picker_key(app, key) {
        return true;
    }
    false
}

fn handle_task_panel_key(app: &mut App, key: event::KeyEvent) -> bool {
    if !app.show_task_panel {
        return false;
    }
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
    true
}

async fn handle_sidebar_key(app: &mut App, key: event::KeyEvent) -> bool {
    if !(app.show_sidebar
        && matches!(
            (key.modifiers, key.code),
            (KeyModifiers::NONE, KeyCode::Up)
                | (KeyModifiers::NONE, KeyCode::Down)
                | (KeyModifiers::NONE, KeyCode::Enter)
        ))
    {
        return false;
    }

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
    true
}

async fn handle_palette_key(app: &mut App, key: event::KeyEvent) -> bool {
    if !app.show_palette {
        return false;
    }
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
    true
}

fn handle_theme_picker_key(app: &mut App, key: event::KeyEvent) -> bool {
    if !app.show_theme_picker {
        return false;
    }
    let total = filtered_theme_choices(app).len();
    match key.code {
        KeyCode::Esc => {
            app.show_theme_picker = false;
            app.theme_picker_input.clear();
            app.theme_picker_selected = 0;
        }
        KeyCode::Enter => {
            let filtered = filtered_theme_choices(app);
            if let Some(choice) = filtered.get(app.theme_picker_selected) {
                let name = choice.name;
                apply_theme(app, name);
                app.show_theme_picker = false;
                app.theme_picker_input.clear();
                app.theme_picker_selected = 0;
            }
        }
        KeyCode::Up if app.theme_picker_selected > 0 => {
            app.theme_picker_selected -= 1;
        }
        KeyCode::Down => {
            let max = total.saturating_sub(1);
            if app.theme_picker_selected < max {
                app.theme_picker_selected += 1;
            }
        }
        KeyCode::Home => app.theme_picker_selected = 0,
        KeyCode::End => app.theme_picker_selected = total.saturating_sub(1),
        KeyCode::Char('j') if app.theme_picker_input.is_empty() => {
            let max = total.saturating_sub(1);
            if app.theme_picker_selected < max {
                app.theme_picker_selected += 1;
            }
        }
        KeyCode::Char('k')
            if app.theme_picker_input.is_empty() && app.theme_picker_selected > 0 =>
        {
            app.theme_picker_selected -= 1;
        }
        KeyCode::Char(c) => {
            app.theme_picker_input.push(c);
            app.theme_picker_selected = 0;
        }
        KeyCode::Backspace => {
            app.theme_picker_input.pop();
            app.theme_picker_selected = 0;
        }
        _ => {}
    }
    true
}
