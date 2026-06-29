use std::path::Path;

use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, UiMutationScope, UiPanelDescriptor, UiPanelRefreshDescriptor,
    UiPanelRefreshKind,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestUiPanelDescriptor {
    scope: UiMutationScope,
    id: String,
    title: String,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    runtime_action_id: Option<String>,
    #[serde(default)]
    refresh: Option<UiPanelRefreshDescriptor>,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
}

impl ManifestUiPanelDescriptor {
    pub(crate) fn to_ui_panel_descriptor(
        &self,
        plugin_id: &PluginId,
        root: &Path,
        bridge_handler: Option<&str>,
    ) -> UiPanelDescriptor {
        let mut descriptor = UiPanelDescriptor::new(
            plugin_id.clone(),
            self.scope,
            self.id.clone(),
            self.title.clone(),
        )
        .with_priority(self.priority.unwrap_or_default())
        .with_visibility(self.visibility.unwrap_or(DescriptorVisibility::HostVisible));
        if let Some(body) = self.body.clone() {
            descriptor = descriptor.with_body(body);
        }
        if let Some(runtime_action_id) = self.runtime_action_id.clone() {
            descriptor = descriptor.with_runtime_action(runtime_action_id);
        }
        if let Some(refresh) = self.refresh.clone() {
            descriptor =
                descriptor.with_refresh(normalize_panel_refresh(root, refresh, bridge_handler));
        }
        descriptor
    }
}

fn normalize_panel_refresh(
    root: &Path,
    refresh: UiPanelRefreshDescriptor,
    bridge_handler: Option<&str>,
) -> UiPanelRefreshDescriptor {
    if refresh.kind != UiPanelRefreshKind::ProcessBridge {
        return refresh;
    }
    let mut refresh = refresh;
    if refresh.handler.trim().is_empty() {
        refresh.handler = bridge_handler.unwrap_or_default().to_owned();
        return refresh;
    }
    if refresh.handler.trim_start().starts_with('{') {
        return refresh;
    }
    let path = Path::new(&refresh.handler);
    let handler = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    refresh.handler = handler.to_string_lossy().into_owned();
    refresh
}
