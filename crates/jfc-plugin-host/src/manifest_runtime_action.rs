use jfc_plugin_sdk::{DescriptorVisibility, PluginId, RuntimeActionDescriptor, RuntimeActionKind};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestRuntimeActionDescriptor {
    id: String,
    label: String,
    description: String,
    kind: RuntimeActionKind,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
    #[serde(default)]
    payload: Option<serde_json::Value>,
}

impl ManifestRuntimeActionDescriptor {
    pub(crate) fn to_runtime_action_descriptor(
        &self,
        plugin_id: &PluginId,
    ) -> Option<RuntimeActionDescriptor> {
        let mut descriptor = RuntimeActionDescriptor::new(
            plugin_id.clone(),
            self.id.clone(),
            self.label.clone(),
            self.description.clone(),
            self.kind,
        )
        .with_priority(self.priority.unwrap_or_default())
        .with_visibility(self.visibility.unwrap_or(DescriptorVisibility::HostVisible));
        if let Some(payload) = self.payload.clone() {
            descriptor = descriptor.with_payload(payload);
        }
        if let Err(error) = descriptor.validate_payload() {
            let reason = error.as_manifest_reason();
            tracing::warn!(
                target: "jfc::plugin_host",
                plugin = plugin_id.as_str(),
                action = self.id.as_str(),
                kind = ?self.kind,
                reason,
                "skipping invalid manifest runtime-action descriptor"
            );
            return None;
        }
        Some(descriptor)
    }
}
