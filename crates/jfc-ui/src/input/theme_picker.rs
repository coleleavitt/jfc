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

pub(super) fn apply_theme(app: &mut App, name: &str) {
    if let Some(choice) = Theme::choice_by_name(name)
        && let Some(theme) = Theme::by_name(choice.name)
    {
        app.theme = theme;
        app.render_cache.borrow_mut().clear();
        crate::markdown::clear_highlight_cache();
        if let Err(err) = crate::config::save_theme(choice.name) {
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Warning,
                    format!("theme: {} (not persisted: {err})", choice.label),
                ),
            );
        } else {
            crate::toast::push_with_cap(
                &mut app.toasts,
                crate::toast::Toast::new(
                    crate::toast::ToastKind::Success,
                    format!("theme: {}", choice.label),
                ),
            );
        }
    }
}
