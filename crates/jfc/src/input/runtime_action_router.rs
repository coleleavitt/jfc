use crate::app::App;
use crate::runtime::EngineEvent;
use jfc_plugin_sdk::{
    RuntimeActionDescriptor, RuntimeActionKind, RuntimeActionPayloadError, UiSlotDescriptor,
};
use tokio::sync::mpsc;

use super::palette::command_palette_slots;
use super::runtime_action_panels::{
    focused_info_sidebar_panel_descriptor, runtime_action_for_panel_descriptor,
};
use super::runtime_action_refresh::{refresh_panel_snapshot, refresh_widget_snapshot};
use super::runtime_action_widgets::{
    focused_info_sidebar_widget_descriptor, runtime_action_for_widget_descriptor,
};

pub(super) async fn execute_runtime_action_for_label(
    app: &mut App,
    label: &str,
    tx: &mpsc::Sender<EngineEvent>,
) -> bool {
    let Some(action) = runtime_action_for_label(
        &app.plugins.runtime_action_descriptors,
        &app.plugins.ui_slots,
        label,
    ) else {
        return false;
    };
    execute_runtime_action_descriptor(app, &action, tx).await;
    true
}

pub(super) async fn execute_focused_info_sidebar_widget_action(
    app: &mut App,
    tx: &mpsc::Sender<EngineEvent>,
) -> bool {
    let Some(widget) = focused_info_sidebar_widget_descriptor(app) else {
        return false;
    };
    let action = runtime_action_for_widget_descriptor(app, &widget);
    if let Some(action) = action.as_ref() {
        execute_nested_runtime_action(app, action, tx).await;
    }
    let refreshed = refresh_widget_snapshot(app, &widget).await;
    action.is_some() || refreshed
}

pub(super) async fn execute_focused_info_sidebar_panel_action(
    app: &mut App,
    tx: &mpsc::Sender<EngineEvent>,
) -> bool {
    let Some(panel) = focused_info_sidebar_panel_descriptor(app) else {
        return false;
    };
    let action = runtime_action_for_panel_descriptor(app, &panel);
    if let Some(action) = action.as_ref() {
        execute_nested_runtime_action(app, action, tx).await;
    }
    let refreshed = refresh_panel_snapshot(app, &panel).await;
    action.is_some() || refreshed
}

async fn execute_runtime_action_descriptor(
    app: &mut App,
    action: &RuntimeActionDescriptor,
    tx: &mpsc::Sender<EngineEvent>,
) {
    match action.kind {
        RuntimeActionKind::HostAction => execute_host_action(app, action).await,
        RuntimeActionKind::SlashCommand => execute_slash_command(app, action, tx).await,
        RuntimeActionKind::RefreshMetrics => {
            super::runtime_action_metrics::execute_refresh_metrics_action(app, action);
        }
        RuntimeActionKind::OpenPanel => {
            super::runtime_action_open_panel::execute_open_panel_action(app, action, tx).await;
        }
        RuntimeActionKind::SendTeammateMessage => {
            super::runtime_action_teammate::execute_teammate_message_action(action).await;
        }
        RuntimeActionKind::RefreshPromptContext => {
            super::runtime_action_prompt_context::execute_refresh_prompt_context_action(
                app, action,
            );
        }
        RuntimeActionKind::PluginSmoke => {
            super::runtime_action_smoke::execute_plugin_smoke_action(app, action).await;
        }
        RuntimeActionKind::PluginDiagnostics => {
            super::runtime_action_plugin_diagnostics::execute_plugin_diagnostics_action(
                app, action, tx,
            )
            .await;
        }
    }
}

async fn execute_host_action(app: &mut App, action: &RuntimeActionDescriptor) {
    super::runtime_action_host::execute_host_action(app, action).await;
}

async fn execute_slash_command(
    app: &mut App,
    action: &RuntimeActionDescriptor,
    tx: &mpsc::Sender<EngineEvent>,
) {
    let Ok(command) =
        required_action_payload(action, RuntimeActionDescriptor::slash_command_payload)
    else {
        return;
    };
    super::palette_slash_action::execute_palette_slash_command_name(app, command, tx).await;
}

pub(super) async fn execute_nested_runtime_action(
    app: &mut App,
    action: &RuntimeActionDescriptor,
    tx: &mpsc::Sender<EngineEvent>,
) {
    match action.kind {
        RuntimeActionKind::HostAction => execute_host_action(app, action).await,
        RuntimeActionKind::SlashCommand => execute_slash_command(app, action, tx).await,
        RuntimeActionKind::RefreshMetrics => {
            super::runtime_action_metrics::execute_refresh_metrics_action(app, action);
        }
        RuntimeActionKind::OpenPanel => {
            let _ = super::runtime_action_open_panel::open_panel_navigation(app, action).await;
        }
        RuntimeActionKind::SendTeammateMessage => {
            super::runtime_action_teammate::execute_teammate_message_action(action).await;
        }
        RuntimeActionKind::RefreshPromptContext => {
            super::runtime_action_prompt_context::execute_refresh_prompt_context_action(
                app, action,
            );
        }
        RuntimeActionKind::PluginSmoke => {
            super::runtime_action_smoke::execute_plugin_smoke_action(app, action).await;
        }
        RuntimeActionKind::PluginDiagnostics => {
            super::runtime_action_plugin_diagnostics::execute_plugin_diagnostics_action(
                app, action, tx,
            )
            .await;
        }
    }
}

fn runtime_action_for_label(
    actions: &[RuntimeActionDescriptor],
    slots: &[UiSlotDescriptor],
    label: &str,
) -> Option<RuntimeActionDescriptor> {
    if let Some(slot) = command_palette_slots(slots)
        .into_iter()
        .find(|slot| slot.label == label)
        && let Some(action) = actions
            .iter()
            .find(|action| action.plugin_id == slot.plugin_id && action.id == slot.id)
    {
        return Some(action.clone());
    }
    actions.iter().find(|action| action.label == label).cloned()
}

fn required_action_payload<'a>(
    action: &'a RuntimeActionDescriptor,
    parser: fn(&'a RuntimeActionDescriptor) -> Result<&'a str, RuntimeActionPayloadError>,
) -> Result<&'a str, RuntimeActionPayloadError> {
    match parser(action) {
        Ok(payload) => Ok(payload),
        Err(RuntimeActionPayloadError::MissingHostAction) => {
            warn_missing_payload(action, "action");
            Err(RuntimeActionPayloadError::MissingHostAction)
        }
        Err(RuntimeActionPayloadError::MissingSlashCommand) => {
            warn_missing_payload(action, "command");
            Err(RuntimeActionPayloadError::MissingSlashCommand)
        }
        Err(error) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                reason = error.as_manifest_reason(),
                "invalid runtime-action payload"
            );
            Err(error)
        }
    }
}

fn warn_missing_payload(action: &RuntimeActionDescriptor, key: &str) {
    tracing::warn!(
        target: "jfc::palette",
        plugin = action.plugin_id.as_str(),
        action = action.id.as_str(),
        key,
        "runtime action is missing required payload field"
    );
}
