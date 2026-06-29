use std::{
    collections::hash_map::DefaultHasher,
    fmt::Debug,
    hash::{Hash, Hasher},
};

use jfc_plugin_sdk::PluginId;
use serde::{Deserialize, Serialize};

use crate::{
    PluginHealthSummary, PluginHost, PluginHostSnapshot, PluginStatusKind,
    descriptor_issue_types::PluginDescriptorIssue, descriptor_issues::plugin_descriptor_issues,
};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginDescriptorCounts {
    pub plugins: usize,
    pub active_plugins: usize,
    pub failed_plugins: usize,
    pub hooks: usize,
    pub services: usize,
    pub tools: usize,
    pub providers: usize,
    pub resources: usize,
    pub commands: usize,
    pub ui_slots: usize,
    pub ui_panels: usize,
    pub ui_widgets: usize,
    pub runtime_actions: usize,
    pub runtime_extensions: usize,
    pub agent_launches: usize,
    pub metrics: usize,
    pub errors: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginHostDiagnostics {
    pub health: PluginHealthSummary,
    pub counts: PluginDescriptorCounts,
    pub descriptor_digest: String,
    pub descriptor_issues: Vec<PluginDescriptorIssue>,
    pub active_plugins: Vec<PluginId>,
    pub failed_plugins: Vec<PluginId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PluginReloadReport {
    pub diagnostics: PluginHostDiagnostics,
    pub previous_descriptor_digest: Option<String>,
    pub changed: Option<bool>,
}

impl PluginHost {
    pub fn diagnostics(&self) -> PluginHostDiagnostics {
        let snapshot = self.status_snapshot();
        let services = self.service_descriptors();
        let tools = self.tool_descriptors();
        let providers = self.provider_descriptors();
        let resources = self.resource_descriptors();
        let commands = self.command_descriptors();
        let ui_slots = self.ui_slot_descriptors();
        let ui_panels = self.ui_panel_descriptors();
        let ui_widgets = self.ui_widget_descriptors();
        let runtime_actions = self.runtime_action_descriptors();
        let runtime_extensions = self.runtime_extension_descriptors();
        let agent_launches = self.agent_launch_descriptors();
        let metrics = self.metric_descriptors();

        let counts = PluginDescriptorCounts {
            plugins: snapshot.plugins.len(),
            active_plugins: snapshot
                .plugins
                .iter()
                .filter(|plugin| plugin.status == PluginStatusKind::Active)
                .count(),
            failed_plugins: snapshot
                .plugins
                .iter()
                .filter(|plugin| plugin.status == PluginStatusKind::Failed)
                .count(),
            hooks: snapshot
                .plugins
                .iter()
                .map(|plugin| plugin.hooks.len())
                .sum(),
            services: services.len(),
            tools: tools.len(),
            providers: providers.len(),
            resources: resources.len(),
            commands: commands.len(),
            ui_slots: ui_slots.len(),
            ui_panels: ui_panels.len(),
            ui_widgets: ui_widgets.len(),
            runtime_actions: runtime_actions.len(),
            runtime_extensions: runtime_extensions.len(),
            agent_launches: agent_launches.len(),
            metrics: metrics.len(),
            errors: snapshot
                .plugins
                .iter()
                .map(|plugin| plugin.errors.len())
                .sum(),
        };

        let active_plugins = plugin_ids_by_status(&snapshot, PluginStatusKind::Active);
        let failed_plugins = plugin_ids_by_status(&snapshot, PluginStatusKind::Failed);
        let descriptor_issues =
            plugin_descriptor_issues(&ui_slots, &ui_panels, &ui_widgets, &runtime_actions);
        let descriptor_digest = descriptor_digest(
            &snapshot,
            &services,
            &tools,
            &providers,
            &resources,
            &commands,
            &ui_slots,
            &ui_panels,
            &ui_widgets,
            &runtime_actions,
            &runtime_extensions,
            &agent_launches,
            &metrics,
        );

        PluginHostDiagnostics {
            health: snapshot.health_summary(),
            counts,
            descriptor_digest,
            descriptor_issues,
            active_plugins,
            failed_plugins,
        }
    }
}

impl PluginReloadReport {
    pub fn new(diagnostics: PluginHostDiagnostics, previous_digest: Option<&str>) -> Self {
        let changed = previous_digest.map(|digest| digest != diagnostics.descriptor_digest);
        Self {
            diagnostics,
            previous_descriptor_digest: previous_digest.map(str::to_owned),
            changed,
        }
    }
}

fn plugin_ids_by_status(snapshot: &PluginHostSnapshot, status: PluginStatusKind) -> Vec<PluginId> {
    snapshot
        .plugins
        .iter()
        .filter(|plugin| plugin.status == status)
        .map(|plugin| plugin.plugin_id.clone())
        .collect()
}

fn descriptor_digest(
    snapshot: &PluginHostSnapshot,
    services: &[jfc_plugin_sdk::ServiceDescriptor],
    tools: &[jfc_plugin_sdk::ToolDescriptor],
    providers: &[jfc_plugin_sdk::ProviderDescriptor],
    resources: &[jfc_plugin_sdk::ResourceDescriptor],
    commands: &[jfc_plugin_sdk::CommandDescriptor],
    ui_slots: &[jfc_plugin_sdk::UiSlotDescriptor],
    ui_panels: &[jfc_plugin_sdk::UiPanelDescriptor],
    ui_widgets: &[jfc_plugin_sdk::UiWidgetDescriptor],
    runtime_actions: &[jfc_plugin_sdk::RuntimeActionDescriptor],
    runtime_extensions: &[jfc_plugin_sdk::RuntimeExtensionDescriptor],
    agent_launches: &[jfc_plugin_sdk::AgentLaunchDescriptor],
    metrics: &[jfc_plugin_sdk::MetricDescriptor],
) -> String {
    let mut rows = Vec::new();
    push_rows(&mut rows, "plugin", &snapshot.plugins);
    push_rows(&mut rows, "service", services);
    push_rows(&mut rows, "tool", tools);
    push_rows(&mut rows, "provider", providers);
    push_rows(&mut rows, "resource", resources);
    push_rows(&mut rows, "command", commands);
    push_rows(&mut rows, "ui_slot", ui_slots);
    push_rows(&mut rows, "ui_panel", ui_panels);
    push_rows(&mut rows, "ui_widget", ui_widgets);
    push_rows(&mut rows, "runtime_action", runtime_actions);
    push_rows(&mut rows, "runtime_extension", runtime_extensions);
    push_rows(&mut rows, "agent_launch", agent_launches);
    push_rows(&mut rows, "metric", metrics);
    rows.sort();

    let mut hasher = DefaultHasher::new();
    rows.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn push_rows<T>(rows: &mut Vec<String>, prefix: &str, values: &[T])
where
    T: Serialize + Debug,
{
    for value in values {
        match serde_json::to_string(value) {
            Ok(json) => rows.push(format!("{prefix}:{json}")),
            Err(error) => rows.push(format!("{prefix}:serde_error:{error}:{value:?}")),
        }
    }
}
