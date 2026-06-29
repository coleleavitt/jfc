use crossterm::event::{self, KeyCode, KeyModifiers};

use crate::app::App;
use crate::runtime::EngineEvent;

use super::palette::palette_items;
use super::palette_actions::execute_palette_action;
use super::theme_picker::{apply_theme, close_theme_picker, filtered_theme_choices, preview_theme};

pub(super) async fn handle_modal_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &tokio::sync::mpsc::Sender<EngineEvent>,
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
    if handle_palette_key(app, key, tx).await {
        return true;
    }
    if handle_theme_picker_key(app, key) {
        return true;
    }
    false
}

fn handle_task_panel_key(app: &mut App, key: event::KeyEvent) -> bool {
    if !app.task_panel.visible {
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
            if app.task_panel.detail {
                app.task_panel.detail = false;
            } else {
                app.task_panel.visible = false;
                app.task_panel.expanded_view = crate::app::ExpandedView::None;
            }
        }
        KeyCode::Enter => {
            app.task_panel.detail = !app.task_panel.detail;
        }
        KeyCode::Up if app.task_panel.selected > 0 => {
            app.task_panel.selected -= 1;
            app.task_panel.table.select(Some(app.task_panel.selected));
            app.task_panel.detail = false;
        }
        KeyCode::Down => {
            let max = total.saturating_sub(1);
            if app.task_panel.selected < max {
                app.task_panel.selected += 1;
                app.task_panel.table.select(Some(app.task_panel.selected));
                app.task_panel.detail = false;
            }
        }
        _ => {}
    }
    true
}

fn handle_teammates_panel_key(app: &mut App, key: event::KeyEvent) -> bool {
    use crate::app::ExpandedView;
    if app.task_panel.expanded_view != ExpandedView::Teammates {
        return false;
    }
    match key.code {
        KeyCode::Esc => {
            app.task_panel.expanded_view = ExpandedView::None;
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
        KeyCode::Enter if app.task_panel.viewing_task_id.is_some() => {
            app.task_panel.expanded_view = ExpandedView::None;
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
    app.task_panel.expanded_view = match app.task_panel.expanded_view {
        ExpandedView::None => ExpandedView::Tasks,
        ExpandedView::Tasks if has_teammates => ExpandedView::Teammates,
        ExpandedView::Tasks => ExpandedView::None,
        ExpandedView::Teammates => ExpandedView::None,
    };
    app.task_panel.visible = app.task_panel.expanded_view == ExpandedView::Tasks;
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
        app.task_panel.viewing_task_id = None;
        return;
    }

    let current = app
        .task_panel
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
    app.task_panel.viewing_task_id = Some(task_ids[next].clone());
}

async fn handle_sidebar_key(app: &mut App, key: event::KeyEvent) -> bool {
    if !(app.session_sidebar.visible
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
    // below) but `session_sidebar.meta` itself stays in recency order. Build a
    // resolved order each navigation tick so Up/Down/Enter walk the
    // user-visible list, not the underlying vec.
    let ordered = crate::render::ordered_sidebar_sessions(app);
    let total = ordered.len();
    match key.code {
        KeyCode::Up if app.session_sidebar.selected > 0 => {
            app.session_sidebar.selected -= 1;
            app.session_sidebar
                .list
                .select(Some(app.session_sidebar.selected));
        }
        KeyCode::Down => {
            let max = total.saturating_sub(1);
            if app.session_sidebar.selected < max {
                app.session_sidebar.selected += 1;
                app.session_sidebar
                    .list
                    .select(Some(app.session_sidebar.selected));
            }
        }
        KeyCode::Enter => {
            if let Some(id) = ordered.get(app.session_sidebar.selected).cloned()
                && let Some(messages) = jfc_engine::session::load_session(&id).await
            {
                app.engine.messages = messages;
                app.switch_session(Some(id));
                app.engine.streaming_text.clear();
                app.engine.streaming_reasoning.clear();
                app.engine.streaming_response_bytes = 0;
                app.engine.streaming_response_baseline = 0;
                app.engine.streaming_thinking_tokens = 0;
                app.engine.token_rate_samples.clear();
                app.engine.token_rate_sample_thinking = None;
                app.engine.streaming_assistant_idx = None;
                app.scroll_to_bottom();
            }
        }
        _ => {}
    }
    true
}

async fn handle_palette_key(
    app: &mut App,
    key: event::KeyEvent,
    tx: &tokio::sync::mpsc::Sender<EngineEvent>,
) -> bool {
    if !app.palette.visible {
        return false;
    }
    match key.code {
        KeyCode::Esc => {
            app.palette.close();
        }
        KeyCode::Enter => {
            let items = palette_items(app);
            if let Some(label) = items.get(app.palette.selected) {
                let label = label.to_string();
                app.palette.close();
                execute_palette_action(app, &label, tx).await;
            }
        }
        KeyCode::Up if app.palette.selected > 0 => {
            app.palette.selected -= 1;
        }
        KeyCode::Down => {
            let max = palette_items(app).len().saturating_sub(1);
            if app.palette.selected < max {
                app.palette.selected += 1;
            }
        }
        // Jump navigation, parity with the theme/model/session pickers.
        KeyCode::Home => app.palette.selected = 0,
        KeyCode::End => app.palette.selected = palette_items(app).len().saturating_sub(1),
        KeyCode::PageUp => app.palette.selected = app.palette.selected.saturating_sub(5),
        KeyCode::PageDown => {
            let max = palette_items(app).len().saturating_sub(1);
            app.palette.selected = (app.palette.selected + 5).min(max);
        }
        KeyCode::Char(c) => {
            app.palette.input.push(c);
            app.palette.reset_selection();
        }
        KeyCode::Backspace => {
            app.palette.input.pop();
            app.palette.reset_selection();
        }
        _ => {}
    }
    true
}

fn handle_theme_picker_key(app: &mut App, key: event::KeyEvent) -> bool {
    if !app.theme_picker.visible {
        return false;
    }
    let total = filtered_theme_choices(app).len();
    match key.code {
        KeyCode::Esc => {
            // Cancel: revert to the theme that was active before previewing.
            if let Some(orig) = app.theme_picker.preview_original.take() {
                app.theme = orig;
                if let Some(name) = app.theme_picker.preview_original_name.take() {
                    app.active_theme_name = name;
                }
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
                .get(app.theme_picker.selected)
                .map(|choice| choice.name);
            if let Some(name) = name {
                apply_theme(app, name);
            }
            close_theme_picker(app);
            return true;
        }
        KeyCode::Up if app.theme_picker.selected > 0 => {
            app.theme_picker.selected -= 1;
        }
        KeyCode::Down => {
            let max = total.saturating_sub(1);
            if app.theme_picker.selected < max {
                app.theme_picker.selected += 1;
            }
        }
        KeyCode::Home => app.theme_picker.selected = 0,
        KeyCode::End => app.theme_picker.selected = total.saturating_sub(1),
        KeyCode::Char('j') if app.theme_picker.input.is_empty() => {
            let max = total.saturating_sub(1);
            if app.theme_picker.selected < max {
                app.theme_picker.selected += 1;
            }
        }
        KeyCode::Char('k')
            if app.theme_picker.input.is_empty() && app.theme_picker.selected > 0 =>
        {
            app.theme_picker.selected -= 1;
        }
        KeyCode::Char(c) => {
            app.theme_picker.input.push(c);
            app.theme_picker.reset_selection();
        }
        KeyCode::Backspace => {
            app.theme_picker.input.pop();
            app.theme_picker.reset_selection();
        }
        _ => {}
    }
    // Live preview: apply whatever is now highlighted (no persist, no toast),
    // so the whole UI re-themes as the user moves through the list. Esc reverts.
    if let Some(name) = filtered_theme_choices(app)
        .get(app.theme_picker.selected)
        .map(|choice| choice.name)
    {
        preview_theme(app, name);
    }
    true
}
