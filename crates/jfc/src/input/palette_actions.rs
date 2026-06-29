use crate::app::App;
use crate::runtime::EngineEvent;
use jfc_plugin_sdk::{UiSlotActionDescriptor, UiSlotDescriptor};
use tokio::sync::mpsc;

use super::palette::command_palette_slots;

pub(super) async fn execute_palette_action(
    app: &mut App,
    label: &str,
    tx: &mpsc::Sender<EngineEvent>,
) {
    if super::runtime_action_router::execute_runtime_action_for_label(app, label, tx).await {
        return;
    }
    let action = descriptor_action_for_label(&app.plugins.ui_slots, label);
    if let Some(action) = action {
        execute_palette_action_descriptor(app, action, tx).await;
    }
}

async fn execute_palette_action_descriptor(
    app: &mut App,
    action: UiSlotActionDescriptor,
    tx: &mpsc::Sender<EngineEvent>,
) {
    match action {
        UiSlotActionDescriptor::HostAction { action } => {
            super::host_palette_action::execute_host_palette_action_name(app, action.as_str())
                .await;
        }
        UiSlotActionDescriptor::SlashCommand { command } => {
            super::palette_slash_action::execute_palette_slash_command_name(
                app,
                command.as_str(),
                tx,
            )
            .await;
        }
    }
}

fn descriptor_action_for_label(
    slots: &[UiSlotDescriptor],
    label: &str,
) -> Option<UiSlotActionDescriptor> {
    command_palette_slots(slots)
        .into_iter()
        .find(|slot| slot.label == label)
        .and_then(|slot| slot.action.clone())
}
