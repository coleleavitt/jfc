use crate::app::App;

pub(super) async fn refresh_widget_snapshot(
    app: &mut App,
    widget: &jfc_plugin_sdk::UiWidgetDescriptor,
) -> bool {
    match app.refresh_ui_widget_snapshot(widget).await {
        Ok(refreshed) => refreshed,
        Err(error) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = widget.plugin_id.as_str(),
                widget = widget.id.as_str(),
                error = %error,
                "failed to refresh plugin widget snapshot"
            );
            false
        }
    }
}

pub(super) async fn refresh_panel_snapshot(
    app: &mut App,
    panel: &jfc_plugin_sdk::UiPanelDescriptor,
) -> bool {
    match app.refresh_ui_panel_snapshot(panel).await {
        Ok(refreshed) => refreshed,
        Err(error) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = panel.plugin_id.as_str(),
                panel = panel.id.as_str(),
                error = %error,
                "failed to refresh plugin panel snapshot"
            );
            false
        }
    }
}

pub(super) async fn refresh_focused_widget_snapshot(app: &mut App) -> bool {
    match app.refresh_focused_info_sidebar_widget_snapshot().await {
        Ok(refreshed) => refreshed,
        Err(error) => {
            tracing::warn!(
                target: "jfc::palette",
                error = %error,
                "failed to refresh focused plugin widget snapshot"
            );
            false
        }
    }
}

pub(super) async fn refresh_focused_panel_snapshot(app: &mut App) -> bool {
    match app.refresh_focused_info_sidebar_panel_snapshot().await {
        Ok(refreshed) => refreshed,
        Err(error) => {
            tracing::warn!(
                target: "jfc::palette",
                error = %error,
                "failed to refresh focused plugin panel snapshot"
            );
            false
        }
    }
}
