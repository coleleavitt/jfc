use crate::app::App;

use super::palette::collect_all_models;
use super::theme_picker::{apply_theme, open_theme_picker};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum HostPaletteAction {
    ClearMessages,
    ToggleSessionsSidebar,
    ToggleInfoSidebar,
    OpenModelPicker,
    OpenThemePicker,
    ThemeCatppuccin,
    ThemeTokyoNight,
    ThemeGruvbox,
    ToggleThinking,
    RaiseReasoningEffort,
    LowerReasoningEffort,
}

impl HostPaletteAction {
    pub(super) fn parse(action: &str) -> Option<Self> {
        match action {
            "clear_messages" => Some(Self::ClearMessages),
            "toggle_sessions_sidebar" => Some(Self::ToggleSessionsSidebar),
            "toggle_info_sidebar" => Some(Self::ToggleInfoSidebar),
            "open_model_picker" => Some(Self::OpenModelPicker),
            "open_theme_picker" => Some(Self::OpenThemePicker),
            "theme_catppuccin" => Some(Self::ThemeCatppuccin),
            "theme_tokyo_night" => Some(Self::ThemeTokyoNight),
            "theme_gruvbox" => Some(Self::ThemeGruvbox),
            "toggle_thinking" => Some(Self::ToggleThinking),
            "raise_reasoning_effort" => Some(Self::RaiseReasoningEffort),
            "lower_reasoning_effort" => Some(Self::LowerReasoningEffort),
            _ => None,
        }
    }
}

pub(super) async fn execute_host_palette_action_name(app: &mut App, action: &str) {
    let Some(action) = HostPaletteAction::parse(action) else {
        tracing::warn!(
            target: "jfc::palette",
            action,
            "unknown command-palette host action"
        );
        return;
    };
    execute_host_palette_action(app, action).await;
}

pub(super) async fn execute_host_palette_action(app: &mut App, action: HostPaletteAction) {
    match action {
        HostPaletteAction::ClearMessages => clear_messages(app),
        HostPaletteAction::ToggleSessionsSidebar => toggle_sessions_sidebar(app).await,
        HostPaletteAction::ToggleInfoSidebar => {
            app.info_sidebar.visible = !app.info_sidebar.visible;
        }
        HostPaletteAction::OpenModelPicker => open_model_picker(app),
        HostPaletteAction::OpenThemePicker => open_theme_picker(app),
        HostPaletteAction::ThemeCatppuccin => apply_theme(app, "catppuccin"),
        HostPaletteAction::ThemeTokyoNight => apply_theme(app, "tokyo-night"),
        HostPaletteAction::ThemeGruvbox => apply_theme(app, "gruvbox"),
        HostPaletteAction::ToggleThinking => toggle_thinking(app),
        HostPaletteAction::RaiseReasoningEffort => super::step_reasoning_effort(app, true),
        HostPaletteAction::LowerReasoningEffort => super::step_reasoning_effort(app, false),
    }
}

fn clear_messages(app: &mut App) {
    app.engine.messages.clear();
    app.engine.streaming_text.clear();
    app.engine.streaming_reasoning.clear();
    app.engine.streaming_response_bytes = 0;
    app.engine.streaming_response_baseline = 0;
    app.engine.streaming_thinking_tokens = 0;
    app.engine.token_rate_samples.clear();
    app.engine.token_rate_sample_thinking = None;
    app.engine.streaming_assistant_idx = None;
    app.switch_session(None);
}

pub(super) async fn toggle_sessions_sidebar(app: &mut App) {
    app.session_sidebar.visible = !app.session_sidebar.visible;
    if app.session_sidebar.visible {
        app.session_sidebar.meta = jfc_session::list_sessions_with_metadata().await;
    }
}

fn open_model_picker(app: &mut App) {
    let models = collect_all_models(app);
    app.model_picker.open(models);
}

fn toggle_thinking(app: &mut App) {
    if let Some(idx) = app.engine.messages.len().checked_sub(1) {
        let entry = app.reasoning_expanded.entry(idx).or_insert(true);
        *entry = !*entry;
    }
}

#[cfg(test)]
mod tests {
    use super::HostPaletteAction;

    #[test]
    fn host_palette_action_parse_accepts_known_actions_normal() {
        assert_eq!(
            HostPaletteAction::parse("open_model_picker"),
            Some(HostPaletteAction::OpenModelPicker)
        );
        assert_eq!(
            HostPaletteAction::parse("theme_tokyo_night"),
            Some(HostPaletteAction::ThemeTokyoNight)
        );
    }

    #[test]
    fn host_palette_action_parse_rejects_unknown_actions_robust() {
        assert_eq!(HostPaletteAction::parse("shell_out_to_random_code"), None);
    }
}
