use jfc_plugin_host::{
    PluginHost, PluginStatusKind, builtin_ux_plugin_host, register_builtin_ux_plugins,
};
use jfc_plugin_sdk::{PluginCapability, PluginId, PluginSource};

#[test]
fn builtin_ux_capabilities_list_existing_crates() {
    // Given: workspace metadata names for UX/product and frontend-adjacent crates.
    let workspace_members = ["jfc-design", "jfc-voice", "jfc-markdown", "jfc-theme"];

    // When: Todo 11d built-ins are registered and activated through the host.
    let (host, report) = builtin_ux_plugin_host(workspace_members).expect("UX plugins activate");
    let snapshot = host.status_snapshot();

    // Then: each existing crate appears as an active built-in capability in status.
    assert_eq!(snapshot.plugins.len(), 4);
    assert_eq!(
        report.registered_crates,
        vec!["jfc-design", "jfc-voice", "jfc-markdown", "jfc-theme"]
    );
    assert_eq!(report.missing_optional_crates, Vec::<&str>::new());
    assert_builtin_capability(&snapshot, "jfc-design", PluginCapability::Design);
    assert_builtin_capability(&snapshot, "jfc-voice", PluginCapability::Voice);
    assert_builtin_capability(&snapshot, "jfc-markdown", PluginCapability::FrontendSupport);
    assert_builtin_capability(&snapshot, "jfc-theme", PluginCapability::FrontendSupport);
}

#[test]
fn missing_optional_voice_capability_is_reported_not_invented() {
    // Given: stale workspace metadata that lacks the optional voice crate reference.
    let mut host = PluginHost::new();
    let workspace_members = ["jfc-design", "jfc-markdown", "jfc-theme", "not-a-jfc-crate"];

    // When: UX/product built-ins are registered from the metadata names.
    let report = register_builtin_ux_plugins(&mut host, workspace_members)
        .expect("registration records absent voice without inventing a crate");

    // Then: voice is reported stale/absent and no synthetic voice descriptor appears.
    assert_eq!(
        report.registered_crates,
        vec!["jfc-design", "jfc-markdown", "jfc-theme"]
    );
    assert_eq!(report.missing_optional_crates, vec!["jfc-voice"]);
    let snapshot = host.status_snapshot();
    assert_eq!(snapshot.plugins.len(), 3);
    assert!(
        snapshot
            .plugins
            .iter()
            .all(|entry| entry.plugin_id.as_str() != "builtin.jfc-voice")
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
