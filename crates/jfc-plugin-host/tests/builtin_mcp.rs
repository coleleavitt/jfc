use jfc_plugin_host::{
    BUILTIN_MCP_PLUGIN_ID, BUILTIN_TOOL_SERVICES_PLUGIN_ID, PluginHost, PluginStatusKind,
    builtin_mcp_plugin, builtin_service_host, builtin_tool_services_plugin,
};
use jfc_plugin_sdk::{PluginCapability, PluginId, ServiceDescriptorKind, ServiceDescriptorStatus};

#[test]
fn builtin_mcp_capability_lists_descriptors() {
    // Given: the host-owned built-in MCP capability plugin registration.
    let host = builtin_service_host();

    // When: service descriptors and status are read through the host.
    let descriptors = host.service_descriptors();
    let snapshot = host.status_snapshot();

    // Then: MCP namespaces/status are visible as built-in plugin-owned descriptors.
    let mcp_descriptors = descriptors
        .iter()
        .filter(|descriptor| descriptor.plugin_id.as_str() == BUILTIN_MCP_PLUGIN_ID)
        .collect::<Vec<_>>();
    assert_eq!(mcp_descriptors.len(), 2);
    assert!(mcp_descriptors.iter().any(|descriptor| {
        descriptor.kind == ServiceDescriptorKind::McpNamespace
            && descriptor.namespace == "mcp__<server>__<tool>"
            && descriptor.status == ServiceDescriptorStatus::RuntimeConfigured
    }));
    assert!(mcp_descriptors.iter().any(|descriptor| {
        descriptor.kind == ServiceDescriptorKind::McpStatus
            && descriptor.namespace == "/mcp"
            && descriptor.status == ServiceDescriptorStatus::RuntimeConfigured
    }));

    let entry = snapshot
        .plugins
        .iter()
        .find(|entry| entry.plugin_id.as_str() == BUILTIN_MCP_PLUGIN_ID)
        .expect("built-in MCP plugin status entry");
    assert_eq!(entry.status, PluginStatusKind::Active);
    assert!(
        entry
            .manifest
            .capabilities
            .iter()
            .any(|capability| matches!(capability, PluginCapability::Bridge))
    );
}

#[test]
fn disabled_mcp_capability_hides_only_mcp() {
    // Given: MCP and adjacent jfc-tools service plugins are registered side by side.
    let mut host = PluginHost::new();
    host.register_internal(builtin_mcp_plugin())
        .expect("MCP plugin registers");
    host.register_internal(builtin_tool_services_plugin())
        .expect("tool service plugin registers");
    host.activate_all().expect("built-in plugins activate");

    // When: only the MCP plugin is disabled.
    host.disable_plugin(&PluginId::new(BUILTIN_MCP_PLUGIN_ID))
        .expect("MCP plugin disables");
    let descriptors = host.service_descriptors();

    // Then: MCP descriptors are hidden, while non-MCP tool services remain visible.
    assert!(
        descriptors
            .iter()
            .all(|descriptor| descriptor.plugin_id.as_str() != BUILTIN_MCP_PLUGIN_ID)
    );
    let remaining_kinds = descriptors
        .iter()
        .filter(|descriptor| descriptor.plugin_id.as_str() == BUILTIN_TOOL_SERVICES_PLUGIN_ID)
        .map(|descriptor| descriptor.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        remaining_kinds,
        vec![
            ServiceDescriptorKind::ToolProcessRegistry,
            ServiceDescriptorKind::ToolFilesystemOperations,
            ServiceDescriptorKind::ToolNotebookOperations,
        ]
    );

    let snapshot = host.status_snapshot();
    let mcp_status = snapshot
        .plugins
        .iter()
        .find(|entry| entry.plugin_id.as_str() == BUILTIN_MCP_PLUGIN_ID)
        .expect("MCP plugin status entry")
        .status;
    assert_eq!(mcp_status, PluginStatusKind::Disabled);
}
