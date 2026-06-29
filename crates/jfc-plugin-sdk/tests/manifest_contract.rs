use jfc_plugin_sdk::{
    CompatibilityStatus, ExtensionSlot, HookName, PluginCapability, PluginId, PluginManifest,
    PluginScope, PluginSource, PluginVersion,
};

#[test]
fn manifest_round_trips_stable_contract_when_plugin_declares_capabilities() {
    // Given: a manifest with typed ids, source, scopes, hook names, and UI-agnostic slots.
    let manifest = PluginManifest::new(
        PluginId::new("builtin.tools"),
        PluginVersion::new("1.2.3"),
        PluginSource::built_in("jfc-tools"),
    )
    .with_display_name("Built-in tools")
    .with_description("Registers the default JFC tool catalog")
    .with_scope(PluginScope::Workspace)
    .with_capability(PluginCapability::Tools)
    .with_capability(PluginCapability::Hooks {
        hooks: vec![HookName::PreToolUse, HookName::PostToolUse],
    })
    .with_capability(PluginCapability::UiSlots {
        slots: vec![ExtensionSlot::StatusLine, ExtensionSlot::CommandPalette],
    });

    // When: the manifest crosses the serde boundary.
    let json = serde_json::to_string(&manifest).expect("manifest serializes");
    let round_trip: PluginManifest = serde_json::from_str(&json).expect("manifest deserializes");

    // Then: typed fields survive unchanged and compatibility is accepted.
    assert_eq!(round_trip.id().as_str(), "builtin.tools");
    assert_eq!(round_trip.version().as_str(), "1.2.3");
    assert_eq!(
        round_trip.compatibility_status(1),
        CompatibilityStatus::Compatible
    );
    assert!(json.contains("pre_tool_use"));
    assert!(!json.contains("ratatui"));
    assert!(!json.contains("crossterm"));
}

#[test]
fn manifest_reports_incompatible_when_schema_is_newer_than_host() {
    // Given: a manifest written for a newer schema version than this host supports.
    let manifest = PluginManifest::new(
        PluginId::new("external.future"),
        PluginVersion::new("9.0.0"),
        PluginSource::package("registry.example", "future-plugin", "sha256:abc"),
    )
    .with_schema_version(7);

    // When: compatibility is checked by a host that supports schema v1.
    let report = manifest.compatibility_report(1);

    // Then: the report carries a serializable compatibility error DTO.
    assert_eq!(report.status, CompatibilityStatus::Incompatible);
    assert_eq!(report.errors.len(), 1);
    assert_eq!(report.errors[0].code, "unsupported_manifest_schema");
    assert_eq!(
        report.errors[0].plugin_id.as_deref(),
        Some("external.future")
    );
}
