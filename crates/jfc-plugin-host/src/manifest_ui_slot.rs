use jfc_plugin_sdk::{
    DescriptorVisibility, ExtensionSlot, PluginId, UiSlotActionDescriptor, UiSlotDescriptor,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestUiSlotDescriptor {
    slot: ExtensionSlot,
    id: String,
    label: String,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
    #[serde(default)]
    action: Option<UiSlotActionDescriptor>,
}

impl ManifestUiSlotDescriptor {
    pub(crate) fn to_ui_slot_descriptor(&self, plugin_id: &PluginId) -> UiSlotDescriptor {
        let mut descriptor = UiSlotDescriptor::new(
            plugin_id.clone(),
            self.slot,
            self.id.clone(),
            self.label.clone(),
        )
        .with_priority(self.priority.unwrap_or_default())
        .with_visibility(self.visibility.unwrap_or(DescriptorVisibility::HostVisible));
        if let Some(action) = self.action.clone() {
            descriptor = descriptor.with_action(action);
        }
        descriptor
    }
}
