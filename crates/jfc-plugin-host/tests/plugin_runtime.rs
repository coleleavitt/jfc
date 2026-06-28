use jfc_plugin_host::{PluginHost, PluginHostError, PluginRegistration, PluginRuntime};
use jfc_plugin_sdk::{
    DescriptorVisibility, ExtensionSlot, PluginId, PluginManifest, PluginScope, PluginSource,
    PluginVersion, ProcessBridgeCommand, ProviderDescriptor, RuntimeActionDescriptor,
    RuntimeActionKind, ToolDescriptor, UiMutationScope, UiSlotDescriptor, UiWidgetDescriptor,
    UiWidgetKind, UiWidgetRefreshDescriptor,
};
use serde_json::json;

#[test]
fn runtime_maps_active_plugin_descriptors_with_source_provenance() {
    let mut host = PluginHost::new();
    host.register_internal(
        plugin("runtime.active", PluginSource::built_in("runtime-active"))
            .with_tool_descriptor(ToolDescriptor::new(
                PluginId::new("runtime.active"),
                "ReadFile",
                "Read a file",
                json!({"type":"object"}),
            ))
            .with_provider_descriptor(
                ProviderDescriptor::new(PluginId::new("runtime.active"), "anthropic")
                    .with_model("claude-sonnet"),
            )
            .with_command_descriptor(
                jfc_plugin_sdk::CommandDescriptor::new(
                    PluginId::new("runtime.active"),
                    "review",
                    "Review the current diff",
                )
                .with_source_info(
                    PluginSource::built_in("command-source"),
                    PluginScope::Workspace,
                ),
            )
            .with_ui_slot_descriptor(UiSlotDescriptor::new(
                PluginId::new("runtime.active"),
                ExtensionSlot::StatusLine,
                "health",
                "Plugin health",
            ))
            .with_runtime_action_descriptor(RuntimeActionDescriptor::new(
                PluginId::new("runtime.active"),
                "open-health",
                "Open health",
                "Open plugin health details",
                RuntimeActionKind::OpenPanel,
            )),
    )
    .expect("plugin registers");
    host.activate_all().expect("plugin activates");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");

    assert_eq!(
        runtime
            .tools()
            .get("ReadFile")
            .unwrap()
            .plugin_id()
            .as_str(),
        "runtime.active"
    );
    assert_eq!(
        runtime.tools().get("ReadFile").unwrap().source(),
        &PluginSource::built_in("runtime-active")
    );
    assert_eq!(
        runtime.providers().get("anthropic").unwrap().source(),
        &PluginSource::built_in("runtime-active")
    );
    assert_eq!(
        runtime.commands().get("review").unwrap().source(),
        &PluginSource::built_in("command-source")
    );
    assert!(
        runtime
            .ui_slots()
            .contains_key(&(ExtensionSlot::StatusLine, "health".to_owned()))
    );
    assert!(runtime.runtime_actions().contains_key("open-health"));
}

#[test]
fn runtime_maps_reject_duplicate_descriptor_ids() {
    let mut host = PluginHost::new();
    host.register_internal(
        plugin("runtime.first", PluginSource::built_in("first")).with_tool_descriptor(
            ToolDescriptor::new(
                PluginId::new("runtime.first"),
                "ReadFile",
                "Read a file",
                json!({"type":"object"}),
            ),
        ),
    )
    .expect("first plugin registers");
    host.register_internal(
        plugin("runtime.second", PluginSource::built_in("second")).with_tool_descriptor(
            ToolDescriptor::new(
                PluginId::new("runtime.second"),
                "ReadFile",
                "Read another file",
                json!({"type":"object"}),
            ),
        ),
    )
    .expect("second plugin registers");
    host.activate_all().expect("plugins activate");

    let result = PluginRuntime::from_host(&host);

    assert!(matches!(
        result,
        Err(PluginHostError::DuplicateDescriptorId { descriptor_kind, descriptor_id, first_plugin_id, duplicate_plugin_id })
            if descriptor_kind == "tool"
                && descriptor_id == "ReadFile"
                && first_plugin_id == "runtime.first"
                && duplicate_plugin_id == "runtime.second"
    ));
}

#[test]
fn runtime_maps_hide_disabled_plugin_descriptors() {
    let mut host = PluginHost::new();
    let active = PluginId::new("runtime.visible");
    let disabled = PluginId::new("runtime.hidden");
    host.register_internal(
        plugin(active.as_str(), PluginSource::built_in("visible")).with_runtime_action_descriptor(
            RuntimeActionDescriptor::new(
                active.clone(),
                "visible-action",
                "Visible action",
                "Visible runtime action",
                RuntimeActionKind::PluginDiagnostics,
            ),
        ),
    )
    .expect("active plugin registers");
    host.register_internal(
        plugin(disabled.as_str(), PluginSource::built_in("hidden")).with_runtime_action_descriptor(
            RuntimeActionDescriptor::new(
                disabled.clone(),
                "hidden-action",
                "Hidden action",
                "Hidden runtime action",
                RuntimeActionKind::PluginDiagnostics,
            ),
        ),
    )
    .expect("disabled plugin registers");
    host.disable_plugin(&disabled).expect("plugin disables");
    host.activate_all().expect("plugins activate");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");

    assert!(runtime.runtime_actions().contains_key("visible-action"));
    assert!(!runtime.runtime_actions().contains_key("hidden-action"));
}

#[test]
fn runtime_maps_ignore_internal_descriptors() {
    let mut host = PluginHost::new();
    host.register_internal(
        plugin("runtime.internal", PluginSource::built_in("internal")).with_tool_descriptor(
            ToolDescriptor::new(
                PluginId::new("runtime.internal"),
                "InternalOnly",
                "Internal only tool",
                json!({"type":"object"}),
            )
            .with_visibility(DescriptorVisibility::Internal),
        ),
    )
    .expect("plugin registers");
    host.activate_all().expect("plugin activates");

    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");

    assert!(runtime.tools().is_empty());
}

#[tokio::test]
async fn runtime_refreshes_ui_widget_through_process_bridge_seam() {
    let plugin_id = PluginId::new("runtime.widget");
    let mut host = PluginHost::new();
    host.register_internal(
        plugin(plugin_id.as_str(), PluginSource::built_in("widget-source"))
            .with_ui_widget_descriptor(
                UiWidgetDescriptor::new(
                    plugin_id.clone(),
                    UiMutationScope::InfoSidebar,
                    "review.queue",
                    "Review Queue",
                    UiWidgetKind::Text,
                )
                .with_refresh(UiWidgetRefreshDescriptor::process_bridge(
                    bridge_refresh_handler(),
                )),
            ),
    )
    .expect("plugin registers");
    host.activate_all().expect("plugin activates");
    let runtime = PluginRuntime::from_host(&host).expect("runtime builds");

    let result = runtime
        .refresh_ui_widget_snapshot(
            &plugin_id,
            UiMutationScope::InfoSidebar,
            "review.queue",
            Some(json!({ "cursor": "old" })),
        )
        .await
        .expect("widget refreshes through host seam");

    assert_eq!(result.body.as_deref(), Some("from host seam"));
    assert_eq!(result.state, Some(json!({ "cursor": "next" })));
}

fn plugin(id: &str, source: PluginSource) -> PluginRegistration {
    PluginRegistration::new(PluginManifest::new(
        PluginId::new(id),
        PluginVersion::new("0.1.0"),
        source,
    ))
}

fn bridge_refresh_handler() -> String {
    let script = "read line\nid=$(printf '%s\n' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nprintf '{\"type\":\"response\",\"id\":\"%s\",\"response\":{\"kind\":\"ui_widget_refresh\",\"result\":{\"body\":\"from host seam\",\"state\":{\"cursor\":\"next\"}}}}\n' \"$id\"\n";
    let command = ProcessBridgeCommand::new("/bin/sh").with_args(["-c", script]);
    serde_json::to_string(&command).expect("bridge handler json")
}
