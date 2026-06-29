use jfc_plugin_sdk::{
    DescriptorVisibility, PluginCapability, PluginId, RuntimeActionDescriptor, RuntimeActionKind,
    RuntimeActionOpenPanelTarget, RuntimeActionPayloadError,
};

#[test]
fn runtime_action_descriptor_round_trips_without_engine_state_normal() {
    let descriptor = RuntimeActionDescriptor::new(
        PluginId::new("plugin.actions"),
        "metrics.refresh",
        "Refresh metrics",
        "Refresh plugin-owned metrics",
        RuntimeActionKind::RefreshMetrics,
    )
    .with_priority(90)
    .with_visibility(DescriptorVisibility::HostVisible)
    .with_payload(serde_json::json!({ "surface": "panel" }));

    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: RuntimeActionDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    assert_eq!(round_trip.plugin_id.as_str(), "plugin.actions");
    assert_eq!(round_trip.id, "metrics.refresh");
    assert_eq!(round_trip.kind, RuntimeActionKind::RefreshMetrics);
    assert_eq!(round_trip.priority, 90);
    assert!(!text.contains("EngineState"));
    assert!(!text.contains("ratatui"));
}

#[test]
fn runtime_action_capability_names_curated_actions_normal() {
    let capability = PluginCapability::RuntimeActions {
        actions: vec![
            RuntimeActionKind::OpenPanel,
            RuntimeActionKind::SendTeammateMessage,
            RuntimeActionKind::PluginSmoke,
            RuntimeActionKind::PluginDiagnostics,
        ],
    };

    let text = serde_json::to_string(&capability).expect("capability serializes");
    let round_trip: PluginCapability =
        serde_json::from_str(&text).expect("capability deserializes");

    assert_eq!(round_trip, capability);
    assert!(text.contains("runtime_actions"));
    assert!(text.contains("send_teammate_message"));
    assert!(text.contains("plugin_smoke"));
    assert!(text.contains("plugin_diagnostics"));
}

#[test]
fn runtime_action_helpers_emit_canonical_payloads_normal() {
    let plugin_id = PluginId::new("plugin.actions");

    let host = RuntimeActionDescriptor::new(
        plugin_id.clone(),
        "host.toggle",
        "Toggle",
        "Toggle host surface",
        RuntimeActionKind::HostAction,
    )
    .with_host_action("toggle_info_sidebar");
    let slash = RuntimeActionDescriptor::new(
        plugin_id.clone(),
        "slash.help",
        "Help",
        "Show help",
        RuntimeActionKind::SlashCommand,
    )
    .with_slash_command("/help");
    let panel = RuntimeActionDescriptor::new(
        plugin_id.clone(),
        "panel.open",
        "Open Panel",
        "Open panel",
        RuntimeActionKind::OpenPanel,
    )
    .with_open_panel_target(RuntimeActionOpenPanelTarget::InfoSidebar);
    let smoke = RuntimeActionDescriptor::new(
        plugin_id,
        "plugin.smoke",
        "Smoke",
        "Smoke plugin",
        RuntimeActionKind::PluginSmoke,
    )
    .with_plugin_smoke_target("demo");

    assert_eq!(
        host.payload.as_ref().unwrap()["action"],
        "toggle_info_sidebar"
    );
    assert_eq!(slash.payload.as_ref().unwrap()["command"], "/help");
    assert_eq!(panel.payload.as_ref().unwrap()["panel"], "info_sidebar");
    assert_eq!(smoke.payload.as_ref().unwrap()["plugin"], "demo");
}

#[test]
fn runtime_action_open_panel_target_parses_supported_aliases_normal() {
    assert_eq!(
        RuntimeActionOpenPanelTarget::parse("right_sidebar"),
        Some(RuntimeActionOpenPanelTarget::InfoSidebar)
    );
    assert_eq!(
        RuntimeActionOpenPanelTarget::parse("sessions"),
        Some(RuntimeActionOpenPanelTarget::SessionsSidebar)
    );
    assert_eq!(RuntimeActionOpenPanelTarget::parse("floating"), None);
}

#[test]
fn runtime_action_payload_parsers_share_typed_contract_normal() {
    let plugin_id = PluginId::new("plugin.actions");
    let host = RuntimeActionDescriptor::new(
        plugin_id.clone(),
        "host.toggle",
        "Toggle",
        "Toggle host surface",
        RuntimeActionKind::HostAction,
    )
    .with_host_action("toggle_info_sidebar");
    let slash = RuntimeActionDescriptor::new(
        plugin_id.clone(),
        "slash.help",
        "Help",
        "Show help",
        RuntimeActionKind::SlashCommand,
    )
    .with_slash_command("/help docs");
    let panel = RuntimeActionDescriptor::new(
        plugin_id,
        "panel.open",
        "Open Panel",
        "Open panel",
        RuntimeActionKind::OpenPanel,
    )
    .with_payload(serde_json::json!({
        "panel": "right_sidebar",
        "panel_id": "review.panel",
        "panel_plugin_id": "plugin.panels",
        "widget_id": "review.widget",
        "widget_plugin_id": "plugin.widgets",
        "execute_panel_action": true,
        "execute_widget_action": false
    }));

    assert_eq!(host.host_action_payload(), Ok("toggle_info_sidebar"));
    assert_eq!(slash.slash_command_payload(), Ok("/help docs"));
    let open_panel = panel.open_panel_payload().expect("open panel payload");
    assert_eq!(open_panel.target, RuntimeActionOpenPanelTarget::InfoSidebar);
    assert_eq!(open_panel.panel_id, Some("review.panel"));
    assert_eq!(open_panel.panel_plugin_id, Some("plugin.panels"));
    assert_eq!(open_panel.widget_id, Some("review.widget"));
    assert_eq!(open_panel.widget_plugin_id, Some("plugin.widgets"));
    assert!(open_panel.execute_panel_action);
    assert!(!open_panel.execute_widget_action);
}

#[test]
fn runtime_action_payload_parsers_reject_invalid_shape_robust() {
    let plugin_id = PluginId::new("plugin.actions");
    let slash = RuntimeActionDescriptor::new(
        plugin_id.clone(),
        "slash.help",
        "Help",
        "Show help",
        RuntimeActionKind::SlashCommand,
    )
    .with_payload(serde_json::json!({ "command": "help" }));
    let panel = RuntimeActionDescriptor::new(
        plugin_id,
        "panel.open",
        "Open Panel",
        "Open panel",
        RuntimeActionKind::OpenPanel,
    )
    .with_payload(serde_json::json!({
        "panel": "info_sidebar",
        "execute_panel_action": "yes"
    }));

    assert_eq!(
        slash.validate_payload(),
        Err(RuntimeActionPayloadError::InvalidSlashCommand)
    );
    assert_eq!(
        panel.validate_payload(),
        Err(RuntimeActionPayloadError::InvalidOpenPanelExecuteFlag)
    );
}
