use std::path::Path;

use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, ToolApprovalPolicy, ToolDescriptor, ToolExecutorDescriptor,
    ToolExecutorKind,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestToolDescriptor {
    name: String,
    description: String,
    #[serde(default = "default_input_schema")]
    input_schema: toml::Value,
    #[serde(default)]
    executor: Option<ManifestToolExecutor>,
    #[serde(default)]
    approval_policy: Option<ToolApprovalPolicy>,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
}

impl ManifestToolDescriptor {
    pub(crate) fn to_tool_descriptor(
        &self,
        plugin_id: &PluginId,
        root: &Path,
        bridge_handler: Option<&str>,
    ) -> ToolDescriptor {
        let input_schema =
            serde_json::to_value(&self.input_schema).unwrap_or_else(|_| serde_json::json!({}));
        let executor = self
            .executor
            .clone()
            .map(ManifestToolExecutor::into_executor)
            .map(|executor| normalize_executor(root, executor, bridge_handler))
            .or_else(|| {
                bridge_handler.map(|handler| {
                    ToolExecutorDescriptor::new(ToolExecutorKind::ProcessBridge, handler)
                })
            })
            .unwrap_or_else(|| ToolExecutorDescriptor::built_in(""));
        ToolDescriptor::new(
            plugin_id.clone(),
            self.name.clone(),
            self.description.clone(),
            input_schema,
        )
        .with_executor(executor.kind, executor.handler)
        .with_approval_policy(self.approval_policy.unwrap_or_default())
        .with_visibility(self.visibility.unwrap_or(DescriptorVisibility::HostVisible))
    }
}

#[derive(Debug, Clone, Deserialize)]
struct ManifestToolExecutor {
    kind: ToolExecutorKind,
    #[serde(default)]
    handler: String,
}

impl ManifestToolExecutor {
    fn into_executor(self) -> ToolExecutorDescriptor {
        ToolExecutorDescriptor::new(self.kind, self.handler)
    }
}

fn normalize_executor(
    root: &Path,
    executor: ToolExecutorDescriptor,
    bridge_handler: Option<&str>,
) -> ToolExecutorDescriptor {
    if executor.kind != ToolExecutorKind::ProcessBridge {
        return executor;
    }
    if executor.handler.trim().is_empty() {
        return ToolExecutorDescriptor::new(
            ToolExecutorKind::ProcessBridge,
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
    ToolExecutorDescriptor::new(
        ToolExecutorKind::ProcessBridge,
        handler.to_string_lossy().into_owned(),
    )
}

fn default_input_schema() -> toml::Value {
    toml::Value::Table(toml::map::Map::from_iter([(
        "type".to_owned(),
        toml::Value::String("object".to_owned()),
    )]))
}
