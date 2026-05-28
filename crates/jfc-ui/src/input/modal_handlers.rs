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
    let has_teammates = app.team_context.is_active()
        || app.background_tasks.values().any(|bt| bt.status.is_alive());
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
                && let Some(messages) = crate::session::load_session(&id).await
            {
                app.messages = messages;
                app.switch_session(Some(id));
                app.streaming_text.clear();
                app.streaming_reasoning.clear();
                app.streaming_response_bytes = 0;
                app.streaming_assistant_idx = None;
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
