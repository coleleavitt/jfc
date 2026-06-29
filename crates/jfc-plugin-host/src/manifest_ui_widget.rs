use std::path::Path;

use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, UiMutationScope, UiWidgetDescriptor, UiWidgetKind,
    UiWidgetRefreshDescriptor, UiWidgetRefreshKind,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestUiWidgetDescriptor {
    scope: UiMutationScope,
    id: String,
    label: String,
    kind: UiWidgetKind,
    #[serde(default)]
    body: Option<String>,
    #[serde(default)]
    runtime_action_id: Option<String>,
    #[serde(default)]
    refresh: Option<UiWidgetRefreshDescriptor>,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
}

impl ManifestUiWidgetDescriptor {
    pub(crate) fn to_ui_widget_descriptor(
        &self,
        plugin_id: &PluginId,
        root: &Path,
        bridge_handler: Option<&str>,
    ) -> UiWidgetDescriptor {
        let mut descriptor = UiWidgetDescriptor::new(
            plugin_id.clone(),
            self.scope,
            self.id.clone(),
            self.label.clone(),
            self.kind,
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
                descriptor.with_refresh(normalize_widget_refresh(root, refresh, bridge_handler));
        }
        descriptor
    }
}

fn normalize_widget_refresh(
    root: &Path,
    refresh: UiWidgetRefreshDescriptor,
    bridge_handler: Option<&str>,
) -> UiWidgetRefreshDescriptor {
    if refresh.kind != UiWidgetRefreshKind::ProcessBridge {
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
