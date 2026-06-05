use std::sync::Arc;

use crossterm::event::KeyCode;

use crate::app::App;

pub(super) fn open_model_picker(app: &mut App) {
    app.show_model_picker = true;
    app.model_picker_filter.clear();
    app.model_picker_selected = 0;
    app.model_picker_state.select(Some(0));
    app.model_picker_models = super::collect_all_models(app);
}

pub(super) fn handle_model_picker_key(app: &mut App, key: crossterm::event::KeyEvent) -> bool {
    if !app.show_model_picker {
        return false;
    }

    let total = filtered_models(app).len();
    match key.code {
        KeyCode::Esc => {
            close_model_picker(app);
        }
        KeyCode::Enter => {
            let filtered = filtered_models(app);
            if let Some(model) = filtered.get(app.model_picker_selected) {
                let chosen_id = model.id.clone();
                let chosen_provider_name = model.provider.clone();
                let old_model = app.model.clone();
                let old_max_ctx = app.max_context_tokens;
                tracing::info!(
                    target: "jfc::input",
                    old_model = %old_model,
                    new_model = %chosen_id,
                    old_provider = %app.provider.name(),
                    new_provider = %chosen_provider_name,
                    old_max_context_tokens = old_max_ctx,
                    "model switch initiated from picker"
                );
                if let Some(p) = app
                    .providers
                    .iter()
                    .find(|p| chosen_provider_name == p.name())
                {
                    app.provider = Arc::clone(p);
                }
                app.model = chosen_id.clone();
                let recent_model = crate::qualified_model_id(app.provider.as_ref(), &chosen_id);
                crate::app::push_recent_model(&mut app.recent_models, &recent_model);
                app.sync_selected_context_window();
                close_model_picker(app);
            }
        }
        KeyCode::Up if app.model_picker_selected > 0 => {
            app.model_picker_selected -= 1;
            app.model_picker_state
                .select(Some(app.model_picker_selected));
        }
        KeyCode::Down => {
            let max = total.saturating_sub(1);
            if app.model_picker_selected < max {
                app.model_picker_selected += 1;
                app.model_picker_state
                    .select(Some(app.model_picker_selected));
            }
        }
        KeyCode::Home => {
            app.model_picker_selected = 0;
            app.model_picker_state.select(Some(0));
        }
        KeyCode::End => {
            let max = total.saturating_sub(1);
            app.model_picker_selected = max;
            app.model_picker_state.select(Some(max));
        }
        KeyCode::PageUp => {
            app.model_picker_selected = app.model_picker_selected.saturating_sub(10);
            app.model_picker_state
                .select(Some(app.model_picker_selected));
        }
        KeyCode::PageDown => {
            let max = total.saturating_sub(1);
            app.model_picker_selected = (app.model_picker_selected + 10).min(max);
            app.model_picker_state
                .select(Some(app.model_picker_selected));
        }
        KeyCode::Char(c) => {
            app.model_picker_filter.push(c);
            app.model_picker_selected = 0;
            app.model_picker_state.select(Some(0));
        }
        KeyCode::Backspace => {
            app.model_picker_filter.pop();
            app.model_picker_selected = 0;
            app.model_picker_state.select(Some(0));
        }
        _ => {}
    }
    true
}

fn close_model_picker(app: &mut App) {
    app.show_model_picker = false;
    app.model_picker_filter.clear();
    app.model_picker_selected = 0;
    app.model_picker_state.select(Some(0));
}

pub fn filtered_models(app: &App) -> Vec<jfc_provider::ModelInfo> {
    if app.model_picker_filter.is_empty() {
        app.model_picker_models.clone()
    } else {
        let q = app.model_picker_filter.to_lowercase();
        app.model_picker_models
            .iter()
            .filter(|m| {
                m.display_name.to_lowercase().contains(&q) || m.id.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }
}
