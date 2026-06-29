use jfc_plugin_sdk::{
    DescriptorVisibility, MetricDescriptor, MetricSurface, MetricUnit, PluginCapability, PluginId,
};

#[test]
fn metric_descriptor_round_trips_without_frontend_types_normal() {
    let descriptor = MetricDescriptor::new(
        PluginId::new("builtin.observability"),
        "cache.hit_rate",
        "Cache hit rate",
        "Session prompt cache hit rate",
        MetricUnit::Percent,
    )
    .with_surfaces([MetricSurface::StatusLine, MetricSurface::Sidebar])
    .with_priority(84)
    .with_visibility(DescriptorVisibility::HostVisible);

    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: MetricDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    assert_eq!(round_trip.plugin_id.as_str(), "builtin.observability");
    assert_eq!(round_trip.id, "cache.hit_rate");
    assert_eq!(round_trip.unit, MetricUnit::Percent);
    assert_eq!(
        round_trip.surfaces,
        vec![MetricSurface::StatusLine, MetricSurface::Sidebar]
    );
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("crossterm"));
}

#[test]
fn metric_capability_names_host_surfaces_without_widgets_normal() {
    let capability = PluginCapability::Metrics {
        surfaces: vec![MetricSurface::Sidebar, MetricSurface::Panel],
    };

    let text = serde_json::to_string(&capability).expect("capability serializes");
    let round_trip: PluginCapability =
        serde_json::from_str(&text).expect("capability deserializes");

    assert_eq!(round_trip, capability);
    assert!(text.contains("metrics"));
    assert!(text.contains("sidebar"));
    assert!(!text.contains("ratatui"));
}
