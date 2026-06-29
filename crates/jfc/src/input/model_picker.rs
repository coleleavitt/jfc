use std::sync::Arc;

use crossterm::event::KeyCode;

use crate::app::App;

pub(super) fn open_model_picker(app: &mut App) {
    let models = super::collect_all_models(app);
    app.model_picker.open(models);
}

pub(super) fn handle_model_picker_key(app: &mut App, key: crossterm::event::KeyEvent) -> bool {
    if !app.model_picker.visible {
        return false;
    }

    let total = filtered_models(app).len();
    match key.code {
        KeyCode::Esc => {
            close_model_picker(app);
        }
        KeyCode::Enter => {
            let filtered = filtered_models(app);
            if let Some(model) = filtered.get(app.model_picker.selected) {
                let chosen_id = model.id.clone();
                let chosen_provider_name = model.provider.clone();
                let old_model = app.engine.model.clone();
                let old_max_ctx = app.engine.max_context_tokens;
                tracing::info!(
                    target: "jfc::input",
                    old_model = %old_model,
                    new_model = %chosen_id,
                    old_provider = %app.engine.provider.name(),
                    new_provider = %chosen_provider_name,
                    old_max_context_tokens = old_max_ctx,
                    "model switch initiated from picker"
                );
                if let Some(p) = app
                    .engine
                    .providers
                    .iter()
                    .find(|p| chosen_provider_name == p.name())
                {
                    app.engine.provider = Arc::clone(p);
                }
                app.engine.model = chosen_id.clone();
                let recent_model =
                    crate::qualified_model_id(app.engine.provider.as_ref(), &chosen_id);
                crate::app::push_recent_model(&mut app.engine.recent_models, &recent_model);
                app.engine.sync_selected_context_window();
                close_model_picker(app);
            }
        }
        KeyCode::Up if app.model_picker.selected > 0 => {
            app.model_picker.select(app.model_picker.selected - 1);
        }
        KeyCode::Down => {
            let max = total.saturating_sub(1);
            if app.model_picker.selected < max {
                app.model_picker.select(app.model_picker.selected + 1);
            }
        }
        KeyCode::Home => {
            app.model_picker.select(0);
        }
        KeyCode::End => {
            let max = total.saturating_sub(1);
            app.model_picker.select(max);
        }
        KeyCode::PageUp => {
            app.model_picker
                .select(app.model_picker.selected.saturating_sub(10));
        }
        KeyCode::PageDown => {
            let max = total.saturating_sub(1);
            app.model_picker
                .select((app.model_picker.selected + 10).min(max));
        }
        KeyCode::Char(c) => {
            app.model_picker.filter.push(c);
            app.model_picker.reset_selection();
        }
        KeyCode::Backspace => {
            app.model_picker.filter.pop();
            app.model_picker.reset_selection();
        }
        _ => {}
    }
    true
}

fn close_model_picker(app: &mut App) {
    app.model_picker.close();
}

pub fn filtered_models(app: &App) -> Vec<jfc_provider::ModelInfo> {
    if app.model_picker.filter.is_empty() {
        app.model_picker.models.clone()
    } else {
        let q = app.model_picker.filter.to_lowercase();
        app.model_picker
            .models
            .iter()
            .filter(|m| {
                m.display_name.to_lowercase().contains(&q) || m.id.to_lowercase().contains(&q)
            })
            .cloned()
            .collect()
    }
}
