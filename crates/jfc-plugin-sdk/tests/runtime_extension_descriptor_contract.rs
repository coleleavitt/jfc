use jfc_plugin_sdk::{
    DescriptorVisibility, PluginId, RuntimeExtensionDescriptor, RuntimeExtensionExecutorDescriptor,
    RuntimeExtensionExecutorKind, RuntimeExtensionRefreshDescriptor, RuntimeExtensionRefreshKind,
    RuntimeExtensionTarget,
};

#[test]
fn prompt_context_runtime_extension_round_trips_as_executable_contract_normal() {
    // Given: a prompt-context runtime extension with a static executor.
    let descriptor = RuntimeExtensionDescriptor::new(
        PluginId::new("plugin.context"),
        RuntimeExtensionTarget::PromptContext,
        "context.repo-map",
        "Repository map",
    )
    .with_priority(25)
    .with_visibility(DescriptorVisibility::HostVisible)
    .with_executor(RuntimeExtensionExecutorDescriptor::new(
        RuntimeExtensionExecutorKind::StaticText,
        "Always mention the plugin-provided repo map.",
    ));

    // When: the descriptor crosses the SDK serde boundary.
    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: RuntimeExtensionDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    // Then: target, executor, and metadata remain frontend-neutral descriptor data.
    assert_eq!(round_trip.plugin_id.as_str(), "plugin.context");
    assert_eq!(round_trip.target, RuntimeExtensionTarget::PromptContext);
    assert_eq!(
        round_trip.executor.kind,
        RuntimeExtensionExecutorKind::StaticText
    );
    assert_eq!(
        round_trip.executor.handler,
        "Always mention the plugin-provided repo map."
    );
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("EngineState"));
}

#[test]
fn prompt_context_runtime_extension_refresh_cadence_round_trips_normal() {
    let descriptor = RuntimeExtensionDescriptor::new(
        PluginId::new("plugin.context"),
        RuntimeExtensionTarget::PromptContext,
        "context.dynamic",
        "Dynamic context",
    )
    .with_executor(RuntimeExtensionExecutorDescriptor::process_bridge(
        "bin/context",
    ))
    .with_refresh(
        RuntimeExtensionRefreshDescriptor::process_bridge()
            .with_min_interval_ms(1_000)
            .with_auto_refresh_ms(60_000),
    );

    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: RuntimeExtensionDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    let refresh = round_trip.refresh.expect("refresh descriptor");
    assert_eq!(refresh.kind, RuntimeExtensionRefreshKind::ProcessBridge);
    assert_eq!(refresh.min_interval_ms, Some(1_000));
    assert_eq!(refresh.auto_refresh_ms, Some(60_000));
    assert!(text.contains("prompt_context"));
    assert!(text.contains("auto_refresh_ms"));
    assert!(!text.contains("EngineState"));
}

#[test]
fn message_renderer_runtime_extension_round_trips_without_tui_types_normal() {
    // Given: a message renderer that names an executable host handler.
    let descriptor = RuntimeExtensionDescriptor::new(
        PluginId::new("builtin.jfc-markdown"),
        RuntimeExtensionTarget::MessageRenderer,
        "message_renderer.markdown",
        "Markdown message renderer",
    )
    .with_executor(RuntimeExtensionExecutorDescriptor::new(
        RuntimeExtensionExecutorKind::BuiltIn,
        "jfc-markdown::message_renderer",
    ));

    // When: the descriptor crosses the SDK serde boundary.
    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: RuntimeExtensionDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    // Then: renderer registration is executable contract data, not frontend internals.
    assert_eq!(round_trip.target, RuntimeExtensionTarget::MessageRenderer);
    assert_eq!(
        round_trip.executor.kind,
        RuntimeExtensionExecutorKind::BuiltIn
    );
    assert_eq!(
        round_trip.executor.handler,
        "jfc-markdown::message_renderer"
    );
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("crossterm"));
}
