use std::path::Path;

use jfc_plugin_sdk::{
    AgentLaunchDescriptor, AgentLaunchExecutorDescriptor, AgentLaunchExecutorKind,
    DescriptorVisibility, PluginId,
};
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub(crate) struct ManifestAgentLaunchDescriptor {
    name: String,
    label: String,
    description: String,
    #[serde(default)]
    visibility: Option<DescriptorVisibility>,
    executor: AgentLaunchExecutorDescriptor,
}

pub(crate) fn agent_launch_descriptors(
    launchers: &[ManifestAgentLaunchDescriptor],
    plugin_id: &PluginId,
    root: &Path,
    bridge_handler: Option<&str>,
) -> Vec<AgentLaunchDescriptor> {
    launchers
        .iter()
        .map(|launcher| {
            AgentLaunchDescriptor::new(
                plugin_id.clone(),
                launcher.name.clone(),
                launcher.label.clone(),
                launcher.description.clone(),
            )
            .with_visibility(
                launcher
                    .visibility
                    .unwrap_or(DescriptorVisibility::HostVisible),
            )
            .with_executor(normalize_agent_launch_executor(
                root,
                launcher.executor.clone(),
                bridge_handler,
            ))
        })
        .collect()
}

fn normalize_agent_launch_executor(
    root: &Path,
    executor: AgentLaunchExecutorDescriptor,
    bridge_handler: Option<&str>,
) -> AgentLaunchExecutorDescriptor {
    if executor.kind != AgentLaunchExecutorKind::ProcessBridge {
        return executor;
    }
    if executor.handler.trim().is_empty() {
        return AgentLaunchExecutorDescriptor::process_bridge(bridge_handler.unwrap_or_default());
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
    AgentLaunchExecutorDescriptor::process_bridge(handler.to_string_lossy().into_owned())
}
