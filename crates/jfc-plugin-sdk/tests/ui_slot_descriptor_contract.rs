use jfc_plugin_sdk::{
    DescriptorVisibility, ExtensionSlot, PluginId, UiSlotActionDescriptor, UiSlotDescriptor,
};

#[test]
fn ui_slot_descriptor_round_trips_without_frontend_types_normal() {
    let descriptor = UiSlotDescriptor::new(
        PluginId::new("builtin.status"),
        ExtensionSlot::StatusLine,
        "goal.elapsed",
        "Goal elapsed time",
    )
    .with_priority(92)
    .with_visibility(DescriptorVisibility::ModelVisible);

    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: UiSlotDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    assert_eq!(round_trip.plugin_id.as_str(), "builtin.status");
    assert_eq!(round_trip.slot, ExtensionSlot::StatusLine);
    assert_eq!(round_trip.id, "goal.elapsed");
    assert_eq!(round_trip.priority, 92);
    assert_eq!(round_trip.visibility, DescriptorVisibility::ModelVisible);
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("crossterm"));
}

#[test]
fn message_renderer_slot_round_trips_as_frontend_neutral_contract_normal() {
    let descriptor = UiSlotDescriptor::new(
        PluginId::new("builtin.markdown"),
        ExtensionSlot::MessageRenderer,
        "message_renderer.markdown",
        "Markdown message renderer",
    );

    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: UiSlotDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    assert_eq!(round_trip.slot, ExtensionSlot::MessageRenderer);
    assert!(text.contains("message_renderer"));
    assert!(!text.contains("ratatui"));
}

#[test]
fn command_palette_action_round_trips_as_descriptor_data_normal() {
    let descriptor = UiSlotDescriptor::new(
        PluginId::new("plugin.palette"),
        ExtensionSlot::CommandPalette,
        "palette.open_report",
        "Open Plugin Report",
    )
    .with_priority(42)
    .with_slash_command("/plugin-report");

    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: UiSlotDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    assert_eq!(
        round_trip.action,
        Some(UiSlotActionDescriptor::SlashCommand {
            command: "/plugin-report".to_owned(),
        })
    );
    assert!(text.contains("slash_command"));
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("crossterm"));
}
