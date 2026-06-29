use std::path::Path;

use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, RuntimeExtensionDescriptor, RuntimeExtensionExecutorDescriptor,
    RuntimeExtensionExecutorKind, RuntimeExtensionRefreshDescriptor, RuntimeExtensionTarget,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestRuntimeExtensionDescriptor {
    target: RuntimeExtensionTarget,
    id: String,
    label: String,
    #[serde(default)]
    priority: Option<i32>,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
    executor: RuntimeExtensionExecutorDescriptor,
    #[serde(default)]
    refresh: Option<RuntimeExtensionRefreshDescriptor>,
}

pub(crate) fn runtime_extension_descriptors(
    extensions: &[ManifestRuntimeExtensionDescriptor],
    plugin_id: &PluginId,
    root: &Path,
    bridge_handler: Option<&str>,
) -> Vec<RuntimeExtensionDescriptor> {
    extensions
        .iter()
        .map(|extension| {
            let mut descriptor = RuntimeExtensionDescriptor::new(
                plugin_id.clone(),
                extension.target,
                extension.id.clone(),
                extension.label.clone(),
            )
            .with_priority(extension.priority.unwrap_or_default())
            .with_visibility(
                extension
                    .visibility
                    .unwrap_or(DescriptorVisibility::HostVisible),
            )
            .with_executor(normalize_runtime_extension_executor(
                root,
                extension.executor.clone(),
                bridge_handler,
            ));
            if let Some(refresh) = extension.refresh.clone() {
                descriptor = descriptor.with_refresh(refresh);
            }
            descriptor
        })
        .collect()
}

fn normalize_runtime_extension_executor(
    root: &Path,
    executor: RuntimeExtensionExecutorDescriptor,
    bridge_handler: Option<&str>,
) -> RuntimeExtensionExecutorDescriptor {
    if executor.kind != RuntimeExtensionExecutorKind::ProcessBridge {
        return executor;
    }
    if executor.handler.trim().is_empty() {
        return RuntimeExtensionExecutorDescriptor::process_bridge(
            bridge_handler.unwrap_or_default(),
        );
    }
    if executor.handler.trim_start().starts_with('{') {
        return executor;
    }
    let path = Path::new(&executor.handler);
    let handler = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    RuntimeExtensionExecutorDescriptor::process_bridge(handler.to_string_lossy().into_owned())
}
