use std::path::Path;

use jfc_plugin_sdk::{
    AgentLaunchExecutorKind, CommandDescriptor, ExtensionSlot, MetricSurface, PluginCapability,
    PluginId, PluginManifest, PluginVersion, ResourceDescriptor, ResourceKind, RuntimeActionKind,
    RuntimeExtensionTarget,
};

use crate::{
    DiscoveredPluginRoot, PluginDiscovery, PluginDiscoveryOptions, PluginHost, PluginHostError,
    PluginRegistration, PluginReloadReport, process_bridge,
};

const RESOURCE_PLUGIN_VERSION: &str = "0.1.0";

pub fn discovered_resource_plugin_host(
    options: PluginDiscoveryOptions,
) -> Result<PluginHost, PluginHostError> {
    let mut host = PluginHost::new();
    register_discovered_resource_plugins(&mut host, PluginDiscovery::discover(options))?;
    host.activate_all()?;
    Ok(host)
}

pub struct DiscoveredPluginReload {
    pub host: PluginHost,
    pub report: PluginReloadReport,
}

pub fn reload_discovered_resource_plugin_host(
    options: PluginDiscoveryOptions,
    previous_digest: Option<&str>,
) -> Result<DiscoveredPluginReload, PluginHostError> {
    let host = discovered_resource_plugin_host(options)?;
    let report = PluginReloadReport::new(host.diagnostics(), previous_digest);
    Ok(DiscoveredPluginReload { host, report })
}

pub fn register_discovered_resource_plugins<I>(
    host: &mut PluginHost,
    roots: I,
) -> Result<(), PluginHostError>
where
    I: IntoIterator<Item = DiscoveredPluginRoot>,
{
    for root in roots {
        host.register_internal(resource_registration(root))?;
    }
    Ok(())
}

fn resource_registration(root: DiscoveredPluginRoot) -> PluginRegistration {
    let plugin_id = PluginId::new(root.identity.clone());
    let mut manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new(RESOURCE_PLUGIN_VERSION),
        root.source.clone(),
    )
    .with_display_name(root.identity.clone())
    .with_scope(root.scope)
    .with_capability(PluginCapability::Resources)
    .with_capability(PluginCapability::Commands);
    let mut tool_descriptors = root.tool_descriptors.clone();
    if let Some(command) = &root.process_bridge {
        manifest = manifest.with_capability(PluginCapability::Bridge);
        if let Ok(mut descriptors) = process_bridge::describe_tool_descriptors(&plugin_id, command)
        {
            tool_descriptors.append(&mut descriptors);
        }
    }
    if !tool_descriptors.is_empty() {
        manifest = manifest.with_capability(PluginCapability::Tools);
    }
    let provider_descriptors = root.provider_descriptors.clone();
    if !provider_descriptors.is_empty() {
        manifest = manifest.with_capability(PluginCapability::Providers);
    }
    let ui_slot_descriptors = root.ui_slot_descriptors.clone();
    if !ui_slot_descriptors.is_empty() {
        let slots = ui_slot_descriptors
            .iter()
            .map(|descriptor| descriptor.slot)
            .collect::<Vec<ExtensionSlot>>();
        manifest = manifest.with_capability(PluginCapability::UiSlots { slots });
    }
    let ui_panel_descriptors = root.ui_panel_descriptors.clone();
    if !ui_panel_descriptors.is_empty() {
        let scopes = ui_panel_descriptors
            .iter()
            .map(|descriptor| descriptor.scope)
            .collect::<Vec<_>>();
        manifest = manifest.with_capability(PluginCapability::UiPanels { scopes });
    }
    let ui_widget_descriptors = root.ui_widget_descriptors.clone();
    if !ui_widget_descriptors.is_empty() {
        let scopes = ui_widget_descriptors
            .iter()
            .map(|descriptor| descriptor.scope)
            .collect::<Vec<_>>();
        manifest = manifest.with_capability(PluginCapability::UiWidgets { scopes });
    }
    let metric_descriptors = root.metric_descriptors.clone();
    if !metric_descriptors.is_empty() {
        let surfaces = metric_descriptors
            .iter()
            .flat_map(|descriptor| descriptor.surfaces.iter().copied())
            .collect::<Vec<MetricSurface>>();
        manifest = manifest.with_capability(PluginCapability::Metrics { surfaces });
    }
    let runtime_action_descriptors = root.runtime_action_descriptors.clone();
    if !runtime_action_descriptors.is_empty() {
        let actions = runtime_action_descriptors
            .iter()
            .map(|descriptor| descriptor.kind)
            .collect::<Vec<RuntimeActionKind>>();
        manifest = manifest.with_capability(PluginCapability::RuntimeActions { actions });
    }
    let runtime_extension_descriptors = root.runtime_extension_descriptors.clone();
    if !runtime_extension_descriptors.is_empty() {
        let targets = runtime_extension_descriptors
            .iter()
            .map(|descriptor| descriptor.target)
            .collect::<Vec<RuntimeExtensionTarget>>();
        manifest = manifest.with_capability(PluginCapability::RuntimeExtensions { targets });
    }
    let agent_launch_descriptors = root.agent_launch_descriptors.clone();
    if !agent_launch_descriptors.is_empty() {
        let executors = agent_launch_descriptors
            .iter()
            .map(|descriptor| descriptor.executor.kind)
            .collect::<Vec<AgentLaunchExecutorKind>>();
        manifest = manifest.with_capability(PluginCapability::AgentLaunches { executors });
    }

    PluginRegistration::new(manifest)
        .with_resource_descriptors([
            resource_descriptor(
                &root,
                &plugin_id,
                ResourceKind::Skill,
                root.path.join("skills"),
            ),
            resource_descriptor(
                &root,
                &plugin_id,
                ResourceKind::Agent,
                root.path.join("agents"),
            ),
            resource_descriptor(
                &root,
                &plugin_id,
                ResourceKind::Workflow,
                root.workflow_dir().path,
            ),
        ])
        .with_command_descriptor(command_descriptor(&root, &plugin_id))
        .with_tool_descriptors(tool_descriptors)
        .with_provider_descriptors(provider_descriptors)
        .with_ui_slot_descriptors(ui_slot_descriptors)
        .with_ui_panel_descriptors(ui_panel_descriptors)
        .with_ui_widget_descriptors(ui_widget_descriptors)
        .with_metric_descriptors(metric_descriptors)
        .with_runtime_action_descriptors(runtime_action_descriptors)
        .with_runtime_extension_descriptors(runtime_extension_descriptors)
        .with_agent_launch_descriptors(agent_launch_descriptors)
}

fn resource_descriptor(
    root: &DiscoveredPluginRoot,
    plugin_id: &PluginId,
    kind: ResourceKind,
    path: impl AsRef<Path>,
) -> ResourceDescriptor {
    ResourceDescriptor::new(plugin_id.clone(), kind, path.as_ref().to_string_lossy())
        .with_source_info(root.source.clone(), root.scope)
        .with_namespace(root.namespace.clone())
}

fn command_descriptor(root: &DiscoveredPluginRoot, plugin_id: &PluginId) -> CommandDescriptor {
    CommandDescriptor::new(
        plugin_id.clone(),
        root.namespace.clone(),
        format!("Markdown commands from {}", root.identity),
    )
    .with_path(root.path.join("commands").to_string_lossy())
    .with_source_info(root.source.clone(), root.scope)
    .with_namespace(root.namespace.clone())
}
