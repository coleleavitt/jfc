use std::path::Path;

use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, ProviderDescriptor, ProviderExecutorDescriptor,
    ProviderExecutorKind, ProviderModelDescriptor,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestProviderDescriptor {
    provider: String,
    #[serde(default)]
    models: Vec<ProviderModelDescriptor>,
    #[serde(default)]
    executor: Option<ManifestProviderExecutor>,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
}

pub(crate) fn provider_descriptors(
    providers: &[ManifestProviderDescriptor],
    plugin_id: &PluginId,
    root: &Path,
    bridge_handler: Option<&str>,
) -> Vec<ProviderDescriptor> {
    providers
        .iter()
        .map(|provider| provider.to_provider_descriptor(plugin_id, root, bridge_handler))
        .collect()
}

impl ManifestProviderDescriptor {
    fn to_provider_descriptor(
        &self,
        plugin_id: &PluginId,
        root: &Path,
        bridge_handler: Option<&str>,
    ) -> ProviderDescriptor {
        let executor = self
            .executor
            .clone()
            .map(ManifestProviderExecutor::into_executor)
            .map(|executor| normalize_provider_executor(root, executor, bridge_handler))
            .or_else(|| {
                bridge_handler.map(|handler| {
                    ProviderExecutorDescriptor::new(ProviderExecutorKind::ProcessBridge, handler)
                })
            })
            .unwrap_or_else(|| ProviderExecutorDescriptor::built_in(""));
        let mut descriptor = ProviderDescriptor::new(plugin_id.clone(), self.provider.clone())
            .with_executor(executor.kind, executor.handler)
            .with_visibility(self.visibility.unwrap_or(DescriptorVisibility::HostVisible));
        descriptor.models.extend(self.models.clone());
        descriptor
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestProviderExecutor {
    kind: ProviderExecutorKind,
    #[serde(default)]
    handler: String,
}

impl ManifestProviderExecutor {
    fn into_executor(self) -> ProviderExecutorDescriptor {
        ProviderExecutorDescriptor::new(self.kind, self.handler)
    }
}

fn normalize_provider_executor(
    root: &Path,
    executor: ProviderExecutorDescriptor,
    bridge_handler: Option<&str>,
) -> ProviderExecutorDescriptor {
    if executor.kind != ProviderExecutorKind::ProcessBridge {
        return executor;
    }
    if executor.handler.trim().is_empty() {
        return ProviderExecutorDescriptor::new(
            ProviderExecutorKind::ProcessBridge,
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
    ProviderExecutorDescriptor::new(
        ProviderExecutorKind::ProcessBridge,
        handler.to_string_lossy().into_owned(),
    )
}
