use jfc_plugin_host::{
    PluginHost, PluginStatusKind, builtin_governance_plugin_host,
    register_builtin_governance_plugins,
};
use jfc_plugin_sdk::{PluginCapability, PluginId, PluginSource};

#[test]
fn builtin_governance_capabilities_list_existing_crates() {
    // Given: workspace metadata names for governance, audit, background, and remote crates.
    let workspace_members = ["jfc-economy", "jfc-audit", "jfc-daemon", "jfc-remote"];

    // When: Todo 11c built-ins are registered and activated through the host.
    let host = builtin_governance_plugin_host(workspace_members)
        .expect("governance/background plugins activate");
    let snapshot = host.status_snapshot();

    // Then: each existing crate appears as an active built-in capability in status.
    assert_eq!(snapshot.plugins.len(), 4);
    assert_builtin_capability(&snapshot, "jfc-economy", PluginCapability::Governance);
    assert_builtin_capability(&snapshot, "jfc-audit", PluginCapability::Audit);
    assert_builtin_capability(&snapshot, "jfc-daemon", PluginCapability::Background);
    assert_builtin_capability(&snapshot, "jfc-remote", PluginCapability::Remote);
}

#[test]
fn disabled_daemon_capability_does_not_break_status() {
    // Given: the daemon/background descriptor is registered from malformed workspace metadata.
    let daemon_id = PluginId::new("builtin.jfc-daemon");
    let mut host = PluginHost::new();
    register_builtin_governance_plugins(&mut host, ["jfc-daemon", "jfc-daemon", "not-a-jfc-crate"])
        .expect("daemon descriptor registers once");

    // When: the daemon capability is disabled before activation and status is snapped.
    host.disable_plugin(&daemon_id)
        .expect("daemon capability disables");
    host.activate_all().expect("remaining plugins activate");
    let snapshot = host.status_snapshot();

    // Then: status still reports daemon ownership without starting or requiring daemon runtime state.
    assert_eq!(snapshot.plugins.len(), 1);
    let entry = &snapshot.plugins[0];
    assert_eq!(entry.plugin_id, daemon_id);
    assert_eq!(entry.status, PluginStatusKind::Disabled);
    assert_eq!(entry.source, PluginSource::built_in("jfc-daemon"));
    assert_eq!(
        entry.manifest.capabilities,
        vec![PluginCapability::Background]
    );
}

fn assert_builtin_capability(
    snapshot: &jfc_plugin_host::PluginHostSnapshot,
    crate_name: &str,
    capability: PluginCapability,
) {
    let entry = snapshot
        .plugins
        .iter()
        .find(|entry| entry.plugin_id == PluginId::new(format!("builtin.{crate_name}")))
        .unwrap_or_else(|| panic!("missing status entry for {crate_name}"));
    assert_eq!(entry.status, PluginStatusKind::Active);
    assert_eq!(entry.source, PluginSource::built_in(crate_name));
    assert_eq!(entry.manifest.capabilities, vec![capability]);
}
