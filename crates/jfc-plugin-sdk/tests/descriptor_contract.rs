use jfc_plugin_sdk::{
    AuthDescriptor, AuthMethodDescriptor, CommandDescriptor, DescriptorVisibility, PluginId,
    ProviderDescriptor, ProviderExecutorKind, ResourceDescriptor, ResourceKind, ToolApprovalPolicy,
    ToolDescriptor, ToolExecutorKind,
};

#[test]
fn descriptors_round_trip_without_frontend_types_when_declaring_capabilities() {
    // Given: descriptors for the stable plugin contract surface.
    let plugin_id = PluginId::new("builtin.catalog");
    let tool = ToolDescriptor::new(
        plugin_id.clone(),
        "bash",
        "Run a shell command",
        serde_json::json!({"type":"object","required":["command"]}),
    )
    .with_visibility(DescriptorVisibility::ModelVisible);
    let provider = ProviderDescriptor::new(plugin_id.clone(), "anthropic")
        .with_model("claude-sonnet-4-6")
        .with_model("claude-opus-4-7");
    let resource =
        ResourceDescriptor::new(plugin_id.clone(), ResourceKind::Skill, "skills/rust-style");
    let command = CommandDescriptor::new(plugin_id.clone(), "doctor", "Run diagnostics");
    let auth =
        AuthDescriptor::new(plugin_id, "anthropic").with_method(AuthMethodDescriptor::ApiKey {
            label: "API key".to_owned(),
            env_var: Some("ANTHROPIC_API_KEY".to_owned()),
        });

    // When: each descriptor crosses the serde boundary.
    let payload = serde_json::json!({
        "tool": tool,
        "provider": provider,
        "resource": resource,
        "command": command,
        "auth": auth,
    });

    // Then: the payload remains UI-agnostic and preserves descriptor identity.
    let text = payload.to_string();
    assert!(text.contains("model_visible"));
    assert!(text.contains("claude-opus-4-7"));
    assert!(text.contains("api_key"));
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("crossterm"));
}

#[test]
fn tool_descriptor_carries_non_model_executor_and_approval_metadata() {
    // Given: a built-in mutating tool descriptor registered by the host.
    let descriptor = ToolDescriptor::new(
        PluginId::new("builtin.tools"),
        "Bash",
        "Run a shell command",
        serde_json::json!({"type":"object","required":["command"]}),
    )
    .with_executor(ToolExecutorKind::BuiltIn, "Bash")
    .with_approval_policy(ToolApprovalPolicy::Mutating)
    .with_visibility(DescriptorVisibility::ModelVisible);

    // When: the descriptor crosses the SDK serde boundary.
    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: ToolDescriptor = serde_json::from_str(&text).expect("descriptor deserializes");

    // Then: host-only execution and approval metadata survives without changing the input schema.
    assert_eq!(round_trip.executor.kind, ToolExecutorKind::BuiltIn);
    assert_eq!(round_trip.executor.handler, "Bash");
    assert_eq!(round_trip.approval_policy, ToolApprovalPolicy::Mutating);
    assert_eq!(round_trip.input_schema, descriptor.input_schema);
}

#[test]
fn provider_descriptor_carries_bridge_executor_and_model_metadata() {
    // Given: a provider descriptor that advertises bridge execution without exposing Provider.
    let descriptor = ProviderDescriptor::new(PluginId::new("builtin.providers"), "openai")
        .with_executor(ProviderExecutorKind::ProcessBridge, "stdio:openai")
        .with_model_info("gpt-5.1", "GPT-5.1", Some(400_000), Some(128_000))
        .with_visibility(DescriptorVisibility::HostVisible);

    // When: the descriptor crosses the SDK serde boundary.
    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: ProviderDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    // Then: host-only bridge metadata and model catalog metadata survive the round trip.
    assert_eq!(
        round_trip.executor.kind,
        ProviderExecutorKind::ProcessBridge
    );
    assert_eq!(round_trip.executor.handler, "stdio:openai");
    assert_eq!(round_trip.models[0].id, "gpt-5.1");
    assert_eq!(round_trip.models[0].display_name, "GPT-5.1");
    assert_eq!(round_trip.models[0].context_window_tokens, Some(400_000));
    assert_eq!(round_trip.models[0].max_output_tokens, Some(128_000));
}
