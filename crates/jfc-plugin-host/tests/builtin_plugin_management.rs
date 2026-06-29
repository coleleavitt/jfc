use jfc_plugin_host::{
    BUILTIN_PLUGIN_MANAGEMENT_PLUGIN_ID, PluginStatusKind, builtin_plugin_management_plugin_host,
};
use jfc_plugin_sdk::{PluginCapability, ServiceDescriptorKind};

#[test]
fn builtin_plugin_management_capability_lists_store_descriptors() {
    // Given: the host-owned plugin management capability registration.
    let host = builtin_plugin_management_plugin_host();

    // When: service descriptors and status are read through the host.
    let descriptors = host.service_descriptors();
    let snapshot = host.status_snapshot();

    // Then: store, install, update, remove, and doctor are plugin-owned descriptors.
    let management_descriptors = descriptors
        .iter()
        .filter(|descriptor| descriptor.plugin_id.as_str() == BUILTIN_PLUGIN_MANAGEMENT_PLUGIN_ID)
        .collect::<Vec<_>>();
    let kinds = management_descriptors
        .iter()
        .map(|descriptor| descriptor.kind)
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        vec![
            ServiceDescriptorKind::PluginStoreCatalog,
            ServiceDescriptorKind::PluginTemplateCatalog,
            ServiceDescriptorKind::PluginInstaller,
            ServiceDescriptorKind::PluginUpdater,
            ServiceDescriptorKind::PluginRemoval,
            ServiceDescriptorKind::PluginDiagnostics,
            ServiceDescriptorKind::PluginSmoke,
        ]
    );
    assert!(management_descriptors.iter().any(|descriptor| {
        descriptor.namespace == "jfc plugin install"
            && descriptor.description.contains("configured plugin store")
    }));

    let entry = snapshot
        .plugins
        .iter()
        .find(|entry| entry.plugin_id.as_str() == BUILTIN_PLUGIN_MANAGEMENT_PLUGIN_ID)
        .expect("built-in plugin management status entry");
    assert_eq!(entry.status, PluginStatusKind::Active);
    assert!(
        entry
            .manifest
            .capabilities
            .iter()
            .any(|capability| matches!(capability, PluginCapability::PluginManagement))
    );
}
