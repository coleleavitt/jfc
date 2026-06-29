use jfc_plugin_sdk::{
    DescriptorVisibility, PluginCapability, PluginId, UiMutationScope, UiPanelDescriptor,
    UiPanelRefreshDescriptor, UiPanelRefreshKind,
};

#[test]
fn ui_panel_descriptor_round_trips_without_frontend_types_normal() {
    let descriptor = UiPanelDescriptor::new(
        PluginId::new("plugin.panels"),
        UiMutationScope::InfoSidebar,
        "review.summary",
        "Review Summary",
    )
    .with_body("3 open reviews\n1 blocking approval")
    .with_runtime_action("reviews.open")
    .with_refresh(
        UiPanelRefreshDescriptor::process_bridge("bin/reviews-panel")
            .with_min_interval_ms(5_000)
            .with_auto_refresh_ms(60_000),
    )
    .with_priority(42)
    .with_visibility(DescriptorVisibility::HostVisible);

    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: UiPanelDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    assert_eq!(round_trip.plugin_id.as_str(), "plugin.panels");
    assert_eq!(round_trip.scope, UiMutationScope::InfoSidebar);
    assert_eq!(round_trip.id, "review.summary");
    assert_eq!(round_trip.title, "Review Summary");
    assert_eq!(
        round_trip.body.as_deref(),
        Some("3 open reviews\n1 blocking approval")
    );
    assert_eq!(
        round_trip.runtime_action_id.as_deref(),
        Some("reviews.open")
    );
    let refresh = round_trip.refresh.expect("refresh descriptor");
    assert_eq!(refresh.kind, UiPanelRefreshKind::ProcessBridge);
    assert_eq!(refresh.handler, "bin/reviews-panel");
    assert_eq!(refresh.min_interval_ms, Some(5_000));
    assert_eq!(refresh.auto_refresh_ms, Some(60_000));
    assert_eq!(round_trip.priority, 42);
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("crossterm"));
    assert!(!text.contains("App"));
}

#[test]
fn ui_panel_capability_names_bounded_mutation_scopes_normal() {
    let capability = PluginCapability::UiPanels {
        scopes: vec![UiMutationScope::InfoSidebar, UiMutationScope::TaskPanel],
    };

    let text = serde_json::to_string(&capability).expect("capability serializes");
    let round_trip: PluginCapability =
        serde_json::from_str(&text).expect("capability deserializes");

    assert_eq!(round_trip, capability);
    assert!(text.contains("ui_panels"));
    assert!(text.contains("info_sidebar"));
    assert!(text.contains("task_panel"));
}
