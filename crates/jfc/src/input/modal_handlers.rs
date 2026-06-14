use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::App;
use crate::runtime::EngineEvent;

use super::palette::{execute_palette_action, palette_items};
use super::theme_picker::{apply_theme, close_theme_picker, filtered_theme_choices, preview_theme};

pub(super) async fn handle_modal_key(
    app: &mut App,
    key: event::KeyEvent,
    _tx: &tokio::sync::mpsc::Sender<EngineEvent>,
) -> bool {
    if handle_task_panel_key(app, key) {
        return true;
    }
    if handle_teammates_panel_key(app, key) {
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
    if is_ctrl_t(key) {
        cycle_expanded_view(app);
        return true;
    }
    let total = app
        .engine
        .task_store
        .list(jfc_session::DeletedFilter::Exclude)
        .len();
    match key.code {
        KeyCode::Esc => {
            if app.task_panel_detail {
                app.task_panel_detail = false;
            } else {
                app.show_task_panel = false;
                app.expanded_view = crate::app::ExpandedView::None;
            }
        }
        KeyCode::Enter => {
            app.task_panel_detail = !app.task_panel_detail;
        }
        KeyCode::Up if app.task_panel_selected > 0 => {
            app.task_panel_selected -= 1;
            app.task_panel_state.select(Some(app.task_panel_selected));
            app.task_panel_detail = false;
        }
        KeyCode::Down => {
            let max = total.saturating_sub(1);
            if app.task_panel_selected < max {
                app.task_panel_selected += 1;
                app.task_panel_state.select(Some(app.task_panel_selected));
                app.task_panel_detail = false;
            }
        }
        _ => {}
    }
    true
}

fn handle_teammates_panel_key(app: &mut App, key: event::KeyEvent) -> bool {
    use crate::app::ExpandedView;
    if app.expanded_view != ExpandedView::Teammates {
        return false;
    }
    match key.code {
        KeyCode::Esc => {
            app.expanded_view = ExpandedView::None;
        }
        KeyCode::Char('t') if key.modifiers == KeyModifiers::CONTROL => {
            cycle_expanded_view(app);
        }
        KeyCode::Down | KeyCode::Char('j') => {
            move_agent_selection(app, 1);
        }
        KeyCode::Up | KeyCode::Char('k') => {
            move_agent_selection(app, -1);
        }
        KeyCode::Enter if app.viewing_task_id.is_some() => {
            app.expanded_view = ExpandedView::None;
            app.scroll_to_bottom();
        }
        _ => {}
    }
    true
}

fn is_ctrl_t(key: event::KeyEvent) -> bool {
    matches!(
        (key.modifiers, key.code),
        (KeyModifiers::CONTROL, KeyCode::Char('t'))
    )
}

fn cycle_expanded_view(app: &mut App) {
    use crate::app::ExpandedView;
    let has_teammates = app.engine.team_context.is_active()
        || app
            .engine
            .background_tasks
            .values()
            .any(|bt| bt.status.is_alive());
    app.expanded_view = match app.expanded_view {
        ExpandedView::None => ExpandedView::Tasks,
        ExpandedView::Tasks if has_teammates => ExpandedView::Teammates,
        ExpandedView::Tasks => ExpandedView::None,
        ExpandedView::Teammates => ExpandedView::None,
    };
    app.show_task_panel = app.expanded_view == ExpandedView::Tasks;
}

fn sorted_agent_task_ids(app: &App) -> Vec<String> {
    // Single source of truth: the same fleet order the fan renders and
    // the tab strip / leader-key navigation use (failed → active →
    // running → idle → done). Keeps every way of stepping through agents
    // consistent so the user's position never jumps unexpectedly.
    crate::render::fleet_ordered_task_ids(app)
}

fn move_agent_selection(app: &mut App, delta: isize) {
    let task_ids = sorted_agent_task_ids(app);
    if task_ids.is_empty() {
        app.viewing_task_id = None;
        return;
    }

    let current = app
        .viewing_task_id
        .as_ref()
        .and_then(|id| task_ids.iter().position(|task_id| task_id == id));
    let next = match (current, delta.cmp(&0)) {
        (Some(i), std::cmp::Ordering::Less) => i.saturating_sub(1),
        (Some(i), std::cmp::Ordering::Greater) => (i + 1).min(task_ids.len() - 1),
        (Some(i), std::cmp::Ordering::Equal) => i,
        (None, std::cmp::Ordering::Less) => task_ids.len() - 1,
        (None, _) => 0,
    };
    app.viewing_task_id = Some(task_ids[next].clone());
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
            if let Some(id) = ordered.get(app.session_selected).cloned()
                && let Some(messages) = jfc_engine::session::load_session(&id).await
            {
                app.engine.messages = messages;
                app.switch_session(Some(id));
                app.engine.streaming_text.clear();
                app.engine.streaming_reasoning.clear();
                app.engine.streaming_response_bytes = 0;
                app.engine.streaming_assistant_idx = None;
                app.scroll_to_bottom();
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
        // Jump navigation, parity with the theme/model/session pickers.
        KeyCode::Home => app.palette_selected = 0,
        KeyCode::End => app.palette_selected = palette_items(app).len().saturating_sub(1),
        KeyCode::PageUp => app.palette_selected = app.palette_selected.saturating_sub(5),
        KeyCode::PageDown => {
            let max = palette_items(app).len().saturating_sub(1);
            app.palette_selected = (app.palette_selected + 5).min(max);
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
            // Cancel: revert to the theme that was active before previewing.
            if let Some(orig) = app.theme_preview_original.take() {
                app.theme = orig;
                app.render_cache.borrow_mut().clear();
                app.height_index.borrow_mut().clear();
                crate::markdown::clear_highlight_cache();
            }
            close_theme_picker(app);
            return true;
        }
        KeyCode::Enter => {
            // Commit: persist the highlighted theme (apply_theme toasts + saves).
            let name = filtered_theme_choices(app)
                .get(app.theme_picker_selected)
                .map(|choice| choice.name);
            if let Some(name) = name {
                apply_theme(app, name);
            }
            close_theme_picker(app);
            return true;
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
    // Live preview: apply whatever is now highlighted (no persist, no toast),
    // so the whole UI re-themes as the user moves through the list. Esc reverts.
    if let Some(name) = filtered_theme_choices(app)
        .get(app.theme_picker_selected)
        .map(|choice| choice.name)
    {
        preview_theme(app, name);
    }
    true
}
