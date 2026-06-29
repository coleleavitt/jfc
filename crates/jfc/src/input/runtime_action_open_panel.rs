use crate::app::App;
use crate::runtime::EngineEvent;
use jfc_engine::runtime::{
    FrontendOpenPanelRequest, RuntimeActionBoundaryError, RuntimeActionFrontendDirective,
    RuntimeActionOutcome, resolve_runtime_action,
};
use jfc_plugin_sdk::{
    RuntimeActionDescriptor, RuntimeActionOpenPanelTarget, RuntimeActionPayloadError,
};
use tokio::sync::mpsc;

use super::runtime_action_panels::focus_info_sidebar_panel_request;
use super::runtime_action_refresh::{
    refresh_focused_panel_snapshot, refresh_focused_widget_snapshot,
};
use super::runtime_action_widgets::focus_info_sidebar_widget_request;

pub(super) async fn execute_open_panel_action(
    app: &mut App,
    action: &RuntimeActionDescriptor,
    tx: &mpsc::Sender<EngineEvent>,
) {
    let Some(open_panel) = open_panel_frontend_directive(action) else {
        return;
    };
    let navigation_action = apply_open_panel_directive(app, &open_panel).await;
    if let Some(navigation_action) = navigation_action.as_ref() {
        if open_panel.execute_panel_action || open_panel.execute_widget_action {
            super::runtime_action_router::execute_nested_runtime_action(app, navigation_action, tx)
                .await;
        }
    }
    if open_panel.execute_widget_action {
        let _ = refresh_focused_widget_snapshot(app).await;
    }
    if open_panel.execute_panel_action {
        let _ = refresh_focused_panel_snapshot(app).await;
    }
}

pub(super) async fn open_panel_navigation(
    app: &mut App,
    action: &RuntimeActionDescriptor,
) -> Option<RuntimeActionDescriptor> {
    let open_panel = open_panel_frontend_directive(action)?;
    apply_open_panel_directive(app, &open_panel).await
}

async fn apply_open_panel_directive(
    app: &mut App,
    open_panel: &FrontendOpenPanelRequest,
) -> Option<RuntimeActionDescriptor> {
    match open_panel.target {
        RuntimeActionOpenPanelTarget::InfoSidebar => {
            app.info_sidebar.visible = true;
            focus_info_sidebar_panel_request(app, open_panel)
                .or_else(|| focus_info_sidebar_widget_request(app, open_panel))
        }
        RuntimeActionOpenPanelTarget::SessionsSidebar => {
            if !app.session_sidebar.visible {
                super::host_palette_action::toggle_sessions_sidebar(app).await;
            }
            None
        }
        RuntimeActionOpenPanelTarget::ModelPicker => {
            super::host_palette_action::execute_host_palette_action(
                app,
                super::host_palette_action::HostPaletteAction::OpenModelPicker,
            )
            .await;
            None
        }
        RuntimeActionOpenPanelTarget::ThemePicker => {
            super::host_palette_action::execute_host_palette_action(
                app,
                super::host_palette_action::HostPaletteAction::OpenThemePicker,
            )
            .await;
            None
        }
    }
}

fn open_panel_frontend_directive(
    action: &RuntimeActionDescriptor,
) -> Option<FrontendOpenPanelRequest> {
    match resolve_runtime_action(action) {
        Ok(RuntimeActionOutcome::Frontend(RuntimeActionFrontendDirective::OpenPanel(
            open_panel,
        ))) => Some(open_panel),
        Ok(outcome) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                outcome = ?outcome,
                "runtime action did not resolve to OpenPanel frontend directive"
            );
            None
        }
        Err(RuntimeActionBoundaryError::Payload {
            payload_error: RuntimeActionPayloadError::MissingOpenPanel,
            ..
        }) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                key = "panel",
                "runtime action is missing required payload field"
            );
            None
        }
        Err(RuntimeActionBoundaryError::Payload {
            payload_error: RuntimeActionPayloadError::UnsupportedOpenPanelTarget,
            ..
        }) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                panel = action.payload_text("panel").unwrap_or_default(),
                "unknown runtime-action panel target"
            );
            None
        }
        Err(RuntimeActionBoundaryError::Payload { payload_error, .. }) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                reason = payload_error.as_manifest_reason(),
                "invalid runtime-action OpenPanel payload"
            );
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_plugin_sdk::{PluginId, RuntimeActionKind};

    #[test]
    fn open_panel_action_parse_accepts_info_alias_and_flags_normal() {
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "panel.open",
            "Open Panel",
            "Open an info panel",
            RuntimeActionKind::OpenPanel,
        )
        .with_payload(serde_json::json!({
            "panel": "right_sidebar",
            "execute_panel_action": true,
            "execute_widget_action": false
        }));

        let open_panel = open_panel_frontend_directive(&action).expect("known panel");

        assert_eq!(open_panel.target, RuntimeActionOpenPanelTarget::InfoSidebar);
        assert_eq!(open_panel.source.plugin_id, "plugin.palette");
        assert_eq!(open_panel.source.action_id, "panel.open");
        assert!(open_panel.execute_panel_action);
        assert!(!open_panel.execute_widget_action);
    }

    #[test]
    fn open_panel_action_parse_rejects_unknown_panel_robust() {
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "panel.open",
            "Open Panel",
            "Open an info panel",
            RuntimeActionKind::OpenPanel,
        )
        .with_payload(serde_json::json!({ "panel": "floating_debugger" }));

        assert_eq!(
            resolve_runtime_action(&action).map(|_| ()),
            Err(RuntimeActionBoundaryError::Payload {
                kind: RuntimeActionKind::OpenPanel,
                payload_error: RuntimeActionPayloadError::UnsupportedOpenPanelTarget,
            })
        );
    }

    #[test]
    fn open_panel_action_parse_rejects_missing_panel_robust() {
        let action = RuntimeActionDescriptor::new(
            PluginId::new("plugin.palette"),
            "panel.open",
            "Open Panel",
            "Open an info panel",
            RuntimeActionKind::OpenPanel,
        );

        assert_eq!(
            resolve_runtime_action(&action).map(|_| ()),
            Err(RuntimeActionBoundaryError::Payload {
                kind: RuntimeActionKind::OpenPanel,
                payload_error: RuntimeActionPayloadError::MissingOpenPanel,
            })
        );
    }
}
