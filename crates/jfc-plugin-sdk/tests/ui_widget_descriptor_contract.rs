use jfc_plugin_sdk::{
    DescriptorVisibility, PluginCapability, PluginId, UiMutationScope, UiWidgetDescriptor,
    UiWidgetKind, UiWidgetRefreshDescriptor, UiWidgetRefreshKind,
};

#[test]
fn ui_widget_descriptor_round_trips_without_frontend_types_normal() {
    let descriptor = UiWidgetDescriptor::new(
        PluginId::new("plugin.panels"),
        UiMutationScope::InfoSidebar,
        "review.queue",
        "Review Queue",
        UiWidgetKind::Text,
    )
    .with_body("3 open reviews")
    .with_runtime_action("reviews.refresh")
    .with_refresh(
        UiWidgetRefreshDescriptor::process_bridge("bin/reviews-widget")
            .with_min_interval_ms(5_000)
            .with_auto_refresh_ms(60_000),
    )
    .with_priority(71)
    .with_visibility(DescriptorVisibility::HostVisible);

    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: UiWidgetDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    assert_eq!(round_trip.plugin_id.as_str(), "plugin.panels");
    assert_eq!(round_trip.scope, UiMutationScope::InfoSidebar);
    assert_eq!(round_trip.id, "review.queue");
    assert_eq!(round_trip.kind, UiWidgetKind::Text);
    assert_eq!(round_trip.body.as_deref(), Some("3 open reviews"));
    assert_eq!(
        round_trip.runtime_action_id.as_deref(),
        Some("reviews.refresh")
    );
    let refresh = round_trip.refresh.expect("refresh descriptor");
    assert_eq!(refresh.kind, UiWidgetRefreshKind::ProcessBridge);
    assert_eq!(refresh.handler, "bin/reviews-widget");
    assert_eq!(refresh.min_interval_ms, Some(5_000));
    assert_eq!(refresh.auto_refresh_ms, Some(60_000));
    assert_eq!(round_trip.priority, 71);
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("crossterm"));
    assert!(!text.contains("App"));
    assert!(!text.contains("EngineState"));
}

#[test]
fn ui_widget_capability_names_bounded_mutation_scopes_normal() {
    let capability = PluginCapability::UiWidgets {
        scopes: vec![UiMutationScope::InfoSidebar, UiMutationScope::TaskPanel],
    };

    let text = serde_json::to_string(&capability).expect("capability serializes");
    let round_trip: PluginCapability =
        serde_json::from_str(&text).expect("capability deserializes");

    assert_eq!(round_trip, capability);
    assert!(text.contains("ui_widgets"));
    assert!(text.contains("info_sidebar"));
    assert!(text.contains("task_panel"));
}
