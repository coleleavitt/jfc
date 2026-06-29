use jfc_plugin_sdk::{
    AgentLaunchDescriptor, CommandDescriptor, DescriptorVisibility, MetricDescriptor,
    PluginManifest, ProviderDescriptor, ResourceDescriptor, RuntimeActionDescriptor,
    RuntimeExtensionDescriptor, ServiceDescriptor, ToolDescriptor, UiPanelDescriptor,
    UiSlotDescriptor, UiWidgetDescriptor,
};

use crate::{PluginHost, PluginStatusKind, host::PluginEntry};

impl PluginHost {
    pub fn tool_descriptors(&self) -> Vec<ToolDescriptor> {
        let mut plugin_tools = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| (entry.activation_sort_key(), entry.tool_descriptors.clone()))
            .collect::<Vec<_>>();
        plugin_tools.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_tools
            .into_iter()
            .flat_map(|(_, tools)| tools)
            .filter(|tool| tool.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn service_descriptors(&self) -> Vec<ServiceDescriptor> {
        let mut plugin_services = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.service_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_services.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_services
            .into_iter()
            .flat_map(|(_, services)| services)
            .filter(|service| service.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn provider_descriptors(&self) -> Vec<ProviderDescriptor> {
        let mut plugin_providers = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.provider_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_providers.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_providers
            .into_iter()
            .flat_map(|(_, providers)| providers)
            .filter(|provider| provider.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn resource_descriptors(&self) -> Vec<ResourceDescriptor> {
        let mut plugin_resources = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| (entry.activation_sort_key(), entry.resource_descriptors()))
            .collect::<Vec<_>>();
        plugin_resources.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_resources
            .into_iter()
            .flat_map(|(_, resources)| resources)
            .collect()
    }

    pub fn command_descriptors(&self) -> Vec<CommandDescriptor> {
        let mut plugin_commands = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| (entry.activation_sort_key(), entry.command_descriptors()))
            .collect::<Vec<_>>();
        plugin_commands.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_commands
            .into_iter()
            .flat_map(|(_, commands)| commands)
            .collect()
    }

    pub fn ui_slot_descriptors(&self) -> Vec<UiSlotDescriptor> {
        let mut plugin_slots = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.ui_slot_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_slots.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_slots
            .into_iter()
            .flat_map(|(_, slots)| slots)
            .filter(|slot| slot.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn ui_widget_descriptors(&self) -> Vec<UiWidgetDescriptor> {
        let mut plugin_widgets = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.ui_widget_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_widgets.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_widgets
            .into_iter()
            .flat_map(|(_, widgets)| widgets)
            .filter(|widget| widget.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn ui_panel_descriptors(&self) -> Vec<UiPanelDescriptor> {
        let mut plugin_panels = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.ui_panel_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_panels.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_panels
            .into_iter()
            .flat_map(|(_, panels)| panels)
            .filter(|panel| panel.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn runtime_extension_descriptors(&self) -> Vec<RuntimeExtensionDescriptor> {
        let mut plugin_extensions = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.runtime_extension_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_extensions.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_extensions
            .into_iter()
            .flat_map(|(_, extensions)| extensions)
            .filter(|extension| extension.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn runtime_action_descriptors(&self) -> Vec<RuntimeActionDescriptor> {
        let mut plugin_actions = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.runtime_action_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_actions.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_actions
            .into_iter()
            .flat_map(|(_, actions)| actions)
            .filter(|action| action.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn agent_launch_descriptors(&self) -> Vec<AgentLaunchDescriptor> {
        let mut plugin_launchers = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.agent_launch_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_launchers.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_launchers
            .into_iter()
            .flat_map(|(_, launchers)| launchers)
            .filter(|launcher| launcher.visibility != DescriptorVisibility::Internal)
            .collect()
    }

    pub fn metric_descriptors(&self) -> Vec<MetricDescriptor> {
        let mut plugin_metrics = self
            .plugins
            .values()
            .filter(|entry| entry.status == PluginStatusKind::Active)
            .map(|entry| {
                (
                    entry.activation_sort_key(),
                    entry.metric_descriptors.clone(),
                )
            })
            .collect::<Vec<_>>();
        plugin_metrics.sort_by(|left, right| left.0.cmp(&right.0));
        plugin_metrics
            .into_iter()
            .flat_map(|(_, metrics)| metrics)
            .filter(|metric| metric.visibility != DescriptorVisibility::Internal)
            .collect()
    }
}

impl PluginEntry {
    pub(crate) fn resource_descriptors(&self) -> Vec<ResourceDescriptor> {
        self.resource_descriptors
            .iter()
            .cloned()
            .map(|descriptor| resource_with_host_source(descriptor, self.plugin.manifest()))
            .collect()
    }

    pub(crate) fn command_descriptors(&self) -> Vec<CommandDescriptor> {
        self.command_descriptors
            .iter()
            .cloned()
            .map(|descriptor| command_with_host_source(descriptor, self.plugin.manifest()))
            .collect()
    }
}

fn resource_with_host_source(
    mut descriptor: ResourceDescriptor,
    manifest: &PluginManifest,
) -> ResourceDescriptor {
    if descriptor.source.is_none() {
        descriptor.source = Some(manifest.source.clone());
    }
    if descriptor.scope.is_none() {
        descriptor.scope = manifest.scopes.first().copied();
    }
    descriptor
}

fn command_with_host_source(
    mut descriptor: CommandDescriptor,
    manifest: &PluginManifest,
) -> CommandDescriptor {
    if descriptor.source.is_none() {
        descriptor.source = Some(manifest.source.clone());
    }
    if descriptor.scope.is_none() {
        descriptor.scope = manifest.scopes.first().copied();
    }
    descriptor
}
