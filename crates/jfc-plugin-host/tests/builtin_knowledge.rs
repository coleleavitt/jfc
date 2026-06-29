use jfc_plugin_host::{
    PluginStatusKind, builtin_knowledge_plugin_host, register_builtin_knowledge_plugins,
};
use jfc_plugin_sdk::{PluginCapability, PluginId, PluginSource};

#[test]
fn builtin_knowledge_capabilities_list_existing_crates() {
    // Given: workspace metadata names for the knowledge/data crates that exist in this worktree.
    let workspace_members = ["jfc-web", "jfc-memory", "jfc-learn", "jfc-compress"];

    // When: knowledge/data built-ins are registered and activated through the host.
    let (host, report) =
        builtin_knowledge_plugin_host(workspace_members).expect("knowledge plugins activate");
    let snapshot = host.status_snapshot();

    // Then: each existing crate appears as an active built-in resource capability in status.
    assert_eq!(
        report.registered_crates,
        vec!["jfc-web", "jfc-memory", "jfc-learn", "jfc-compress"]
    );
    assert_eq!(report.missing_optional_crates, vec!["jfc-graph"]);

    for crate_name in ["jfc-web", "jfc-memory", "jfc-learn", "jfc-compress"] {
        let entry = snapshot
            .plugins
            .iter()
            .find(|entry| entry.plugin_id == PluginId::new(format!("builtin.{crate_name}")))
            .unwrap_or_else(|| panic!("missing status entry for {crate_name}"));
        assert_eq!(entry.status, PluginStatusKind::Active);
        assert_eq!(entry.source, PluginSource::built_in(crate_name));
        assert!(
            entry
                .manifest
                .capabilities
                .iter()
                .any(|capability| matches!(capability, PluginCapability::Resources)),
            "{crate_name} must advertise resource capability"
        );
    }
}

#[test]
fn missing_optional_graph_capability_is_reported_not_panicked() {
    // Given: malformed/stale workspace metadata that lacks the optional graph crate reference.
    let mut host = jfc_plugin_host::PluginHost::new();
    let workspace_members = ["jfc-web", "not-a-jfc-crate", "jfc-web"];

    // When: knowledge/data built-ins are registered from the metadata names.
    let report = register_builtin_knowledge_plugins(&mut host, workspace_members)
        .expect("registration skips absent optional graph without panic");

    // Then: the existing web capability is registered once and graph is reported as stale/absent.
    assert_eq!(report.registered_crates, vec!["jfc-web"]);
    assert_eq!(report.missing_optional_crates, vec!["jfc-graph"]);
    let snapshot = host.status_snapshot();
    assert_eq!(snapshot.plugins.len(), 1);
    assert_eq!(snapshot.plugins[0].plugin_id.as_str(), "builtin.jfc-web");
    assert_eq!(snapshot.plugins[0].status, PluginStatusKind::Registered);
}
