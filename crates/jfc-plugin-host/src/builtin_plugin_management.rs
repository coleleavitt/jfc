use jfc_plugin_sdk::{
    PluginCapability, PluginId, PluginManifest, PluginSource, PluginVersion, ServiceDescriptor,
    ServiceDescriptorKind,
};

use crate::{
    DiscoveredPluginReload, PluginDiscovery, PluginDiscoveryOptions, PluginHost, PluginHostError,
    PluginRegistration, PluginReloadReport, register_discovered_resource_plugins,
};

pub const BUILTIN_PLUGIN_MANAGEMENT_PLUGIN_ID: &str = "builtin.plugin-management";

pub fn builtin_plugin_management_plugin_host() -> PluginHost {
    let mut host = PluginHost::new();
    if let Err(error) = register_builtin_plugin_management_plugin(&mut host) {
        tracing::warn!(target: "jfc::plugin_host", error = %error, "failed to register built-in plugin management plugin");
    }
    if let Err(error) = host.activate_all() {
        tracing::warn!(target: "jfc::plugin_host", error = %error, "failed to activate built-in plugin management plugin");
    }
    host
}

pub fn plugin_management_plugin_host(
    options: PluginDiscoveryOptions,
) -> Result<PluginHost, PluginHostError> {
    let mut host = PluginHost::new();
    register_builtin_plugin_management_plugin(&mut host)?;
    register_discovered_resource_plugins(&mut host, PluginDiscovery::discover(options))?;
    host.activate_all()?;
    Ok(host)
}

pub fn reload_plugin_management_plugin_host(
    options: PluginDiscoveryOptions,
    previous_digest: Option<&str>,
) -> Result<DiscoveredPluginReload, PluginHostError> {
    let host = plugin_management_plugin_host(options)?;
    let report = PluginReloadReport::new(host.diagnostics(), previous_digest);
    Ok(DiscoveredPluginReload { host, report })
}

pub fn register_builtin_plugin_management_plugin(
    host: &mut PluginHost,
) -> Result<(), PluginHostError> {
    host.register_internal(builtin_plugin_management_plugin())
}

pub fn builtin_plugin_management_plugin() -> PluginRegistration {
    let plugin_id = PluginId::new(BUILTIN_PLUGIN_MANAGEMENT_PLUGIN_ID);
    let manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new("0.1.0"),
        PluginSource::built_in("jfc"),
    )
    .with_display_name("Built-in plugin management")
    .with_description(
        "Host-visible plugin store, install, update, remove, and diagnostics services",
    )
    .with_capability(PluginCapability::PluginManagement);

    PluginRegistration::new(manifest).with_service_descriptors([
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::PluginStoreCatalog,
            "plugin store catalog",
            "jfc plugin list",
            "Lists installed local plugins discovered through the configured plugin store",
        ),
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::PluginTemplateCatalog,
            "plugin template catalog",
            "jfc plugin templates",
            "Lists first-party SDK plugin templates available for installation",
        ),
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::PluginInstaller,
            "plugin installer",
            "jfc plugin install",
            "Installs local, git-backed, or first-party template plugins into the configured plugin store",
        ),
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::PluginUpdater,
            "plugin updater",
            "jfc plugin update",
            "Updates one git-backed plugin or all git-backed plugins in the configured plugin store",
        ),
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::PluginRemoval,
            "plugin removal",
            "jfc plugin remove",
            "Removes installed plugins from the configured plugin store",
        ),
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::PluginDiagnostics,
            "plugin diagnostics",
            "jfc plugin doctor",
            "Prints plugin reload, descriptor digest, health, and service diagnostics",
        ),
        ServiceDescriptor::new(
            plugin_id,
            ServiceDescriptorKind::PluginSmoke,
            "plugin smoke",
            "jfc plugin smoke",
            "Runs process-bridge descriptor smoke checks for an installed plugin",
        ),
    ])
}
