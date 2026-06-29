use jfc_plugin_host::{PluginHost, PluginRegistration};
use jfc_plugin_sdk::{
    DescriptorVisibility, ExtensionSlot, PluginId, PluginManifest, PluginSource, PluginVersion,
    RuntimeActionKind, UiSlotDescriptor,
};

#[test]
fn ui_slot_descriptors_are_collected_from_active_plugins_normal() {
    let mut host = PluginHost::new();
    let active_id = PluginId::new("plugin.active");
    let disabled_id = PluginId::new("plugin.disabled");
    host.register_internal(
        plugin(active_id.clone()).with_ui_slot_descriptor(
            UiSlotDescriptor::new(
                active_id.clone(),
                ExtensionSlot::StatusLine,
                "plugin.health",
                "Plugin health",
            )
            .with_priority(63),
        ),
    )
    .expect("active plugin registers");
    host.register_internal(
        plugin(disabled_id.clone()).with_ui_slot_descriptor(
            UiSlotDescriptor::new(
                disabled_id.clone(),
                ExtensionSlot::StatusLine,
                "hidden",
                "Hidden",
            )
            .with_visibility(DescriptorVisibility::Internal),
        ),
    )
    .expect("disabled plugin registers");
    host.disable_plugin(&disabled_id).expect("plugin disables");

    host.activate_all().expect("plugins activate");
    let slots = host.ui_slot_descriptors();

    assert_eq!(slots.len(), 1);
    assert_eq!(slots[0].plugin_id, active_id);
    assert_eq!(slots[0].id, "plugin.health");
    assert_eq!(slots[0].slot, ExtensionSlot::StatusLine);
}

#[test]
fn plugin_health_summary_counts_statuses_and_errors_normal() {
    let mut host = PluginHost::new();
    host.register_internal(plugin(PluginId::new("plugin.active")))
        .expect("active plugin registers");
    host.register_internal(plugin(PluginId::new("plugin.disabled")))
        .expect("disabled plugin registers");
    host.disable_plugin(&PluginId::new("plugin.disabled"))
        .expect("plugin disables");
    host.register_internal(
        plugin(PluginId::new("plugin.failed"))
            .with_activation(|_| Err(jfc_plugin_host::PluginHostError::plugin("boom"))),
    )
    .expect("failed plugin registers");

    let result = host.activate_all();
    assert!(result.is_err(), "activation should fail");
    let summary = host.status_snapshot().health_summary();

    assert_eq!(summary.total, 3);
    assert_eq!(summary.active, 1);
    assert_eq!(summary.disabled, 1);
    assert_eq!(summary.failed, 1);
    assert_eq!(summary.error_count, 1);
}

#[test]
fn built_in_status_line_plugin_registers_goal_and_health_slots_normal() {
    let host =
        jfc_plugin_host::builtin_status_line_plugin_host().expect("status-line plugin activates");
    let slots = host.ui_slot_descriptors();
    let summary = host.status_snapshot().health_summary();

    assert_eq!(summary.active, 1);
    assert!(slots.iter().any(|slot| {
        slot.id == jfc_plugin_host::BUILTIN_GOAL_STATUS_SLOT_ID
            && slot.slot == ExtensionSlot::StatusLine
    }));
    assert!(slots.iter().any(|slot| {
        slot.id == jfc_plugin_host::BUILTIN_PLUGIN_HEALTH_SLOT_ID
            && slot.slot == ExtensionSlot::StatusLine
    }));
    assert!(slots.iter().any(|slot| {
        slot.id == "command_palette.compact_conversation"
            && slot.slot == ExtensionSlot::CommandPalette
            && slot.label == "Compact Conversation (/compact)"
    }));
    assert!(slots.iter().any(|slot| {
        slot.id == "command_palette.plugin_diagnostics"
            && slot.slot == ExtensionSlot::CommandPalette
            && slot.label == "Run Plugin Diagnostics"
    }));
    assert!(slots.iter().any(|slot| {
        slot.id == jfc_plugin_host::BUILTIN_MESSAGE_RENDERER_SLOT_ID
            && slot.slot == ExtensionSlot::MessageRenderer
    }));
    let actions = host.runtime_action_descriptors();
    let command_slots = slots
        .iter()
        .filter(|slot| slot.slot == ExtensionSlot::CommandPalette)
        .collect::<Vec<_>>();
    for slot in &command_slots {
        assert!(
            actions
                .iter()
                .any(|action| action.plugin_id == slot.plugin_id && action.id == slot.id),
            "missing runtime action for command-palette slot {}",
            slot.id
        );
    }
    assert!(actions.iter().any(|action| {
        action.id == "command_palette.toggle_info_sidebar"
            && action.kind == RuntimeActionKind::HostAction
    }));
    assert!(actions.iter().any(|action| {
        action.id == "command_palette.compact_conversation"
            && action.kind == RuntimeActionKind::SlashCommand
    }));
    assert!(actions.iter().any(|action| {
        action.id == "command_palette.plugin_diagnostics"
            && action.kind == RuntimeActionKind::PluginDiagnostics
    }));

    let snapshot = host.status_snapshot();
    let manifest = &snapshot.plugins[0].manifest;
    assert!(manifest.capabilities.iter().any(|capability| {
        matches!(
            capability,
            jfc_plugin_sdk::PluginCapability::UiSlots { slots }
                if slots.contains(&ExtensionSlot::CommandPalette)
                    && slots.contains(&ExtensionSlot::MessageRenderer)
        )
    }));
    assert!(manifest.capabilities.iter().any(|capability| {
        matches!(
            capability,
            jfc_plugin_sdk::PluginCapability::RuntimeActions { actions }
                if actions.contains(&RuntimeActionKind::HostAction)
                    && actions.contains(&RuntimeActionKind::SlashCommand)
                    && actions.contains(&RuntimeActionKind::PluginDiagnostics)
        )
    }));
}

fn plugin(plugin_id: PluginId) -> PluginRegistration {
    PluginRegistration::new(PluginManifest::new(
        plugin_id,
        PluginVersion::new("0.1.0"),
        PluginSource::built_in("test"),
    ))
}
