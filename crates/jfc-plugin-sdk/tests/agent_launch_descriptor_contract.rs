use jfc_plugin_sdk::{
    AgentLaunchDescriptor, AgentLaunchExecutorDescriptor, AgentLaunchExecutorKind,
    DescriptorVisibility, PluginId,
};

#[test]
fn agent_launch_descriptor_round_trips_as_executor_contract_normal() {
    // Given: an agent launch contract with an explicit process bridge executor.
    let descriptor = AgentLaunchDescriptor::new(
        PluginId::new("plugin.agents"),
        "variant-agent",
        "Variant Agent",
        "Launches a plugin-defined agent variant.",
    )
    .with_visibility(DescriptorVisibility::HostVisible)
    .with_executor(AgentLaunchExecutorDescriptor::new(
        AgentLaunchExecutorKind::ProcessBridge,
        "agents/variant-launcher.sh",
    ));

    // When: the descriptor crosses the SDK serde boundary.
    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: AgentLaunchDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    // Then: launch metadata remains a UI-neutral executor contract.
    assert_eq!(round_trip.plugin_id.as_str(), "plugin.agents");
    assert_eq!(round_trip.name, "variant-agent");
    assert_eq!(round_trip.label, "Variant Agent");
    assert_eq!(
        round_trip.executor.kind,
        AgentLaunchExecutorKind::ProcessBridge
    );
    assert_eq!(round_trip.executor.handler, "agents/variant-launcher.sh");
    assert!(!text.contains("ratatui"));
    assert!(!text.contains("EngineState"));
}

#[test]
fn builtin_agent_launch_descriptor_names_host_handler_normal() {
    // Given: a built-in agent launcher owned by the host.
    let descriptor = AgentLaunchDescriptor::new(
        PluginId::new("builtin.jfc-agents"),
        "jfc.agents.in_process",
        "JFC in-process agents",
        "Launches built-in JFC agents through the in-process backend.",
    )
    .with_executor(AgentLaunchExecutorDescriptor::built_in(
        "jfc-engine::agents::in_process",
    ));

    // When: the descriptor crosses the SDK serde boundary.
    let text = serde_json::to_string(&descriptor).expect("descriptor serializes");
    let round_trip: AgentLaunchDescriptor =
        serde_json::from_str(&text).expect("descriptor deserializes");

    // Then: built-in execution is still a named launch contract, not a hidden source path.
    assert_eq!(round_trip.plugin_id.as_str(), "builtin.jfc-agents");
    assert_eq!(round_trip.executor.kind, AgentLaunchExecutorKind::BuiltIn);
    assert_eq!(
        round_trip.executor.handler,
        "jfc-engine::agents::in_process"
    );
    assert!(!text.contains("tokio::spawn"));
}
