use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, ServiceDescriptor, ServiceDescriptorKind,
    ServiceDescriptorStatus,
};

#[test]
fn service_descriptor_carries_mcp_namespace_status_metadata() {
    // Given: an MCP namespace service descriptor that remains host-visible only.
    let descriptor = ServiceDescriptor::new(
        PluginId::new("builtin.mcp"),
        ServiceDescriptorKind::McpNamespace,
        "mcp tool namespace",
        "mcp__<server>__<tool>",
        "Namespaced model-visible MCP tools resolved through the active MCP registry",
    )
    .with_status(ServiceDescriptorStatus::RuntimeConfigured)
    .with_visibility(DescriptorVisibility::HostVisible);

    // When: the descriptor crosses the SDK serde boundary.
    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: ServiceDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    // Then: plugin ownership, namespace, status, and visibility survive the round trip.
    assert_eq!(round_trip.plugin_id.as_str(), "builtin.mcp");
    assert_eq!(round_trip.kind, ServiceDescriptorKind::McpNamespace);
    assert_eq!(round_trip.namespace, "mcp__<server>__<tool>");
    assert_eq!(
        round_trip.status,
        ServiceDescriptorStatus::RuntimeConfigured
    );
    assert_eq!(round_trip.visibility, DescriptorVisibility::HostVisible);
}

#[test]
fn service_descriptor_carries_plugin_management_kind_metadata() {
    // Given: the host-owned plugin installer is advertised as a service descriptor.
    let descriptor = ServiceDescriptor::new(
        PluginId::new("builtin.plugin-management"),
        ServiceDescriptorKind::PluginInstaller,
        "plugin installer",
        "jfc plugin install",
        "Installs local or git-backed plugins into the configured plugin store",
    );

    // When: the descriptor crosses the SDK serde boundary.
    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: ServiceDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    // Then: plugin-management kinds remain stable and human-readable.
    assert_eq!(round_trip.kind, ServiceDescriptorKind::PluginInstaller);
    assert_eq!(round_trip.kind.as_str(), "plugin_installer");
    assert_eq!(round_trip.namespace, "jfc plugin install");
}
