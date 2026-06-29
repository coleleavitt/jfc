use jfc_plugin_sdk::{
    PluginCapability, PluginId, PluginManifest, PluginSource, PluginVersion, ServiceDescriptor,
    ServiceDescriptorKind, ServiceDescriptorStatus,
};

use crate::{PluginHost, PluginRegistration};

pub const BUILTIN_MCP_PLUGIN_ID: &str = "builtin.mcp";
pub const BUILTIN_TOOL_SERVICES_PLUGIN_ID: &str = "builtin.tool-services";

pub fn builtin_service_host() -> PluginHost {
    let mut host = PluginHost::new();
    for plugin in [builtin_mcp_plugin(), builtin_tool_services_plugin()] {
        if let Err(error) = host.register_internal(plugin) {
            tracing::warn!(target: "jfc::plugin_host", error = %error, "failed to register built-in service plugin");
        }
    }
    if let Err(error) = host.activate_all() {
        tracing::warn!(target: "jfc::plugin_host", error = %error, "failed to activate built-in service plugins");
    }
    host
}

pub fn builtin_mcp_plugin() -> PluginRegistration {
    let plugin_id = PluginId::new(BUILTIN_MCP_PLUGIN_ID);
    let manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new("0.1.0"),
        PluginSource::built_in("jfc-mcp"),
    )
    .with_display_name("Built-in MCP bridge")
    .with_description("Host-visible MCP namespaces, dispatch bridge, and server status surface")
    .with_capability(PluginCapability::Tools)
    .with_capability(PluginCapability::Resources)
    .with_capability(PluginCapability::Bridge);

    PluginRegistration::new(manifest).with_service_descriptors([
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::McpNamespace,
            "mcp tool namespace",
            "mcp__<server>__<tool>",
            "Namespaced model-visible MCP tools resolved through the active MCP registry",
        )
        .with_status(ServiceDescriptorStatus::RuntimeConfigured),
        ServiceDescriptor::new(
            plugin_id,
            ServiceDescriptorKind::McpStatus,
            "mcp server status",
            "/mcp",
            "Connected, failed, and disabled MCP server status used by /mcp and the sidebar",
        )
        .with_status(ServiceDescriptorStatus::RuntimeConfigured),
    ])
}

pub fn builtin_tool_services_plugin() -> PluginRegistration {
    let plugin_id = PluginId::new(BUILTIN_TOOL_SERVICES_PLUGIN_ID);
    let manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new("0.1.0"),
        PluginSource::built_in("jfc-tools"),
    )
    .with_display_name("Built-in tool services")
    .with_description("Host-visible jfc-tools support services behind built-in tool execution")
    .with_capability(PluginCapability::Tools);

    PluginRegistration::new(manifest).with_service_descriptors([
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::ToolProcessRegistry,
            "bash process registry",
            "jfc-tools::bash_processes",
            "Tracks foreground Bash subprocesses so user abort can terminate process trees",
        ),
        ServiceDescriptor::new(
            plugin_id.clone(),
            ServiceDescriptorKind::ToolFilesystemOperations,
            "filesystem operations",
            "jfc-tools::filesystem",
            "Pure Read, Write, and Edit filesystem operations wrapped by engine permissions and undo",
        ),
        ServiceDescriptor::new(
            plugin_id,
            ServiceDescriptorKind::ToolNotebookOperations,
            "notebook operations",
            "jfc-tools::notebook",
            "Pure notebook read/edit operations wrapped by the built-in tool dispatcher",
        ),
    ])
}
