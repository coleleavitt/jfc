use crate::{app::App, theme::Theme};

pub(crate) fn filtered_theme_choices(app: &App) -> Vec<&'static crate::theme::ThemeChoice> {
    let query = app.theme_picker_input.trim().to_ascii_lowercase();
    Theme::choices()
        .iter()
        .filter(|choice| {
            query.is_empty()
                || choice.name.contains(&query)
                || choice.label.to_ascii_lowercase().contains(&query)
                || choice.description.to_ascii_lowercase().contains(&query)
                || choice.aliases.iter().any(|alias| alias.contains(&query))
        })
        .collect()
}

/// Open the theme picker: snapshot the active theme for revert-on-cancel and
/// highlight the current theme so opening previews no change (mirrors Claude
/// Code's `usePreviewTheme` + opencode's live theme switch).
pub(super) fn open_theme_picker(app: &mut App) {
    app.theme_preview_original = Some(app.theme);
    app.theme_picker_input.clear();
    let current = jfc_engine::config::load_arc().theme.clone();
    app.theme_picker_selected = current
        .as_deref()
        .and_then(Theme::choice_by_name)
        .and_then(|choice| Theme::choices().iter().position(|c| c.name == choice.name))
        .unwrap_or(0);
    app.show_theme_picker = true;
}

/// Close the picker and drop the preview snapshot.
pub(super) fn close_theme_picker(app: &mut App) {
    app.show_theme_picker = false;
    app.theme_picker_input.clear();
    app.theme_picker_selected = 0;
    app.theme_preview_original = None;
}

/// Apply a theme for LIVE PREVIEW only — swap the active theme and bust the
/// style caches, but do NOT persist or toast. Committed on Enter via
/// [`apply_theme`] or reverted on Esc.
pub(super) fn preview_theme(app: &mut App, name: &str) {
    if let Some(choice) = Theme::choice_by_name(name)
        && let Some(theme) = Theme::by_name(choice.name)
    {
        app.theme = theme;
        app.render_cache.borrow_mut().clear();
        app.height_index.borrow_mut().clear();
        crate::markdown::clear_highlight_cache();
    }
}

pub(super) fn apply_theme(app: &mut App, name: &str) {
    if let Some(choice) = Theme::choice_by_name(name)
        && let Some(theme) = Theme::by_name(choice.name)
    {
        app.theme = theme;
        app.render_cache.borrow_mut().clear();
        app.height_index.borrow_mut().clear();
        crate::markdown::clear_highlight_cache();
        if let Err(err) = jfc_engine::config::save_theme(choice.name) {
            jfc_engine::toast::push_with_cap(
                &mut app.engine.toasts,
                jfc_engine::toast::Toast::new(
                    jfc_engine::toast::ToastKind::Warning,
                    format!("Theme: {} (not persisted: {err})", choice.label),
                ),
            );
        } else {
            jfc_engine::toast::push_with_cap(
                &mut app.engine.toasts,
                jfc_engine::toast::Toast::new(
                    jfc_engine::toast::ToastKind::Success,
                    format!("Theme: {}", choice.label),
                ),
            );
        }
    }
}
