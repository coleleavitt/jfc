use std::path::Path;

use jfc_plugin_sdk::{
    AgentLaunchDescriptor, MetricDescriptor, PluginId, ProcessBridgeCommand, ProviderDescriptor,
    RuntimeActionDescriptor, RuntimeExtensionDescriptor, ToolDescriptor, UiPanelDescriptor,
    UiSlotDescriptor, UiWidgetDescriptor,
};
use serde::Deserialize;

use crate::manifest_agent_launch::{self, ManifestAgentLaunchDescriptor};
use crate::manifest_metric::ManifestMetricDescriptor;
use crate::manifest_provider::{self, ManifestProviderDescriptor};
use crate::manifest_runtime_action::ManifestRuntimeActionDescriptor;
use crate::manifest_runtime_extension::{self, ManifestRuntimeExtensionDescriptor};
use crate::manifest_tool::ManifestToolDescriptor;
use crate::manifest_ui_panel::ManifestUiPanelDescriptor;
use crate::manifest_ui_slot::ManifestUiSlotDescriptor;
use crate::manifest_ui_widget::ManifestUiWidgetDescriptor;

#[derive(Debug, Clone)]
pub(crate) struct PluginManifestInfo {
    pub(crate) name: Option<String>,
    pub(crate) workflows_dir: Option<String>,
    pub(crate) process_bridge: Option<ProcessBridgeCommand>,
    tools: Vec<ManifestToolDescriptor>,
    providers: Vec<ManifestProviderDescriptor>,
    ui_slots: Vec<ManifestUiSlotDescriptor>,
    ui_panels: Vec<ManifestUiPanelDescriptor>,
    metrics: Vec<ManifestMetricDescriptor>,
    ui_widgets: Vec<ManifestUiWidgetDescriptor>,
    runtime_actions: Vec<ManifestRuntimeActionDescriptor>,
    runtime_extensions: Vec<ManifestRuntimeExtensionDescriptor>,
    agent_launches: Vec<ManifestAgentLaunchDescriptor>,
}

impl PluginManifestInfo {
    pub(crate) fn tool_descriptors(
        &self,
        plugin_id: &PluginId,
        root: &Path,
    ) -> Vec<ToolDescriptor> {
        let bridge_handler = self
            .process_bridge
            .as_ref()
            .and_then(|command| process_bridge_handler(root, command).ok());
        self.tools
            .iter()
            .map(|tool| tool.to_tool_descriptor(plugin_id, root, bridge_handler.as_deref()))
            .collect()
    }

    pub(crate) fn resolved_process_bridge(&self, root: &Path) -> Option<ProcessBridgeCommand> {
        self.process_bridge
            .as_ref()
            .map(|command| resolve_process_bridge_command(root, command))
    }

    pub(crate) fn provider_descriptors(
        &self,
        plugin_id: &PluginId,
        root: &Path,
    ) -> Vec<ProviderDescriptor> {
        let bridge_handler = self
            .process_bridge
            .as_ref()
            .and_then(|command| process_bridge_handler(root, command).ok());
        manifest_provider::provider_descriptors(
            &self.providers,
            plugin_id,
            root,
            bridge_handler.as_deref(),
        )
    }

    pub(crate) fn ui_slot_descriptors(&self, plugin_id: &PluginId) -> Vec<UiSlotDescriptor> {
        self.ui_slots
            .iter()
            .map(|slot| slot.to_ui_slot_descriptor(plugin_id))
            .collect()
    }

    pub(crate) fn ui_panel_descriptors(
        &self,
        plugin_id: &PluginId,
        root: &Path,
    ) -> Vec<UiPanelDescriptor> {
        let bridge_handler = self
            .process_bridge
            .as_ref()
            .and_then(|command| process_bridge_handler(root, command).ok());
        self.ui_panels
            .iter()
            .map(|panel| panel.to_ui_panel_descriptor(plugin_id, root, bridge_handler.as_deref()))
            .collect()
    }

    pub(crate) fn metric_descriptors(&self, plugin_id: &PluginId) -> Vec<MetricDescriptor> {
        self.metrics
            .iter()
            .map(|metric| metric.to_metric_descriptor(plugin_id))
            .collect()
    }

    pub(crate) fn ui_widget_descriptors(
        &self,
        plugin_id: &PluginId,
        root: &Path,
    ) -> Vec<UiWidgetDescriptor> {
        let bridge_handler = self
            .process_bridge
            .as_ref()
            .and_then(|command| process_bridge_handler(root, command).ok());
        self.ui_widgets
            .iter()
            .map(|widget| {
                widget.to_ui_widget_descriptor(plugin_id, root, bridge_handler.as_deref())
            })
            .collect()
    }

    pub(crate) fn runtime_action_descriptors(
        &self,
        plugin_id: &PluginId,
    ) -> Vec<RuntimeActionDescriptor> {
        self.runtime_actions
            .iter()
            .filter_map(|action| action.to_runtime_action_descriptor(plugin_id))
            .collect()
    }

    pub(crate) fn runtime_extension_descriptors(
        &self,
        plugin_id: &PluginId,
        root: &Path,
    ) -> Vec<RuntimeExtensionDescriptor> {
        let bridge_handler = self
            .process_bridge
            .as_ref()
            .and_then(|command| process_bridge_handler(root, command).ok());
        manifest_runtime_extension::runtime_extension_descriptors(
            &self.runtime_extensions,
            plugin_id,
            root,
            bridge_handler.as_deref(),
        )
    }

    pub(crate) fn agent_launch_descriptors(
        &self,
        plugin_id: &PluginId,
        root: &Path,
    ) -> Vec<AgentLaunchDescriptor> {
        let bridge_handler = self
            .process_bridge
            .as_ref()
            .and_then(|command| process_bridge_handler(root, command).ok());
        manifest_agent_launch::agent_launch_descriptors(
            &self.agent_launches,
            plugin_id,
            root,
            bridge_handler.as_deref(),
        )
    }
}

pub(crate) fn read_manifest(path: &Path) -> Option<PluginManifestInfo> {
    read_jfc_manifest(path).or_else(|| read_codex_manifest(path))
}

fn read_jfc_manifest(path: &Path) -> Option<PluginManifestInfo> {
    let text = std::fs::read_to_string(path.join(".jfc-plugin.toml")).ok()?;
    let manifest = toml::from_str::<JfcPluginManifest>(&text).ok()?;
    Some(PluginManifestInfo {
        name: Some(manifest.plugin.name),
        workflows_dir: manifest.plugin.workflows_dir,
        process_bridge: manifest.process_bridge,
        tools: manifest.tools,
        providers: manifest.providers,
        ui_slots: manifest.ui_slots,
        ui_panels: manifest.ui_panels,
        metrics: manifest.metrics,
        ui_widgets: manifest.ui_widgets,
        runtime_actions: manifest.runtime_actions,
        runtime_extensions: manifest.runtime_extensions,
        agent_launches: manifest.agent_launches,
    })
}

fn read_codex_manifest(path: &Path) -> Option<PluginManifestInfo> {
    let text = std::fs::read_to_string(path.join(".codex-plugin/plugin.json")).ok()?;
    let manifest = serde_json::from_str::<CodexPluginManifest>(&text).ok()?;
    Some(PluginManifestInfo {
        name: manifest.name,
        workflows_dir: None,
        process_bridge: None,
        tools: Vec::new(),
        providers: Vec::new(),
        ui_slots: Vec::new(),
        ui_panels: Vec::new(),
        metrics: Vec::new(),
        ui_widgets: Vec::new(),
        runtime_actions: Vec::new(),
        runtime_extensions: Vec::new(),
        agent_launches: Vec::new(),
    })
}

fn process_bridge_handler(
    root: &Path,
    command: &ProcessBridgeCommand,
) -> Result<String, serde_json::Error> {
    serde_json::to_string(&resolve_process_bridge_command(root, command))
}

fn resolve_process_bridge_command(
    root: &Path,
    command: &ProcessBridgeCommand,
) -> ProcessBridgeCommand {
    let path = Path::new(&command.command);
    let command_path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    ProcessBridgeCommand {
        command: command_path.to_string_lossy().into_owned(),
        args: command.args.clone(),
    }
}

#[derive(Debug, Clone, Deserialize)]
struct JfcPluginManifest {
    plugin: JfcPluginMeta,
    #[serde(default)]
    tools: Vec<ManifestToolDescriptor>,
    #[serde(default)]
    providers: Vec<ManifestProviderDescriptor>,
    #[serde(default)]
    ui_slots: Vec<ManifestUiSlotDescriptor>,
    #[serde(default)]
    ui_panels: Vec<ManifestUiPanelDescriptor>,
    #[serde(default)]
    metrics: Vec<ManifestMetricDescriptor>,
    #[serde(default)]
    ui_widgets: Vec<ManifestUiWidgetDescriptor>,
    #[serde(default)]
    runtime_actions: Vec<ManifestRuntimeActionDescriptor>,
    #[serde(default)]
    runtime_extensions: Vec<ManifestRuntimeExtensionDescriptor>,
    #[serde(default)]
    agent_launches: Vec<ManifestAgentLaunchDescriptor>,
    process_bridge: Option<ProcessBridgeCommand>,
}

#[derive(Debug, Clone, Deserialize)]
struct JfcPluginMeta {
    name: String,
    workflows_dir: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct CodexPluginManifest {
    name: Option<String>,
}
