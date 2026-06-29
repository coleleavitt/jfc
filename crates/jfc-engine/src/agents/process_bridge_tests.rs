use jfc_plugin_sdk::{
    AgentLaunchDescriptor, AgentLaunchExecutorDescriptor, BridgeAgentLaunchResult, PluginId,
    ProcessBridgeCommand,
};

use super::process_bridge::{
    ProcessBridgeAgentLaunchInvocation, agent_result_to_execution_result,
    execute_process_bridge_agent_launch,
};

fn task_input() -> jfc_core::TaskInput {
    jfc_core::TaskInput {
        description: "inspect code".to_owned(),
        prompt: "find the sharp edges".to_owned(),
        subagent_type: Some("reviewer".to_owned()),
        category: None,
        run_in_background: false,
        model: None,
        launcher: None,
        effort: None,
        name: None,
        team_name: None,
        mode: None,
        isolation: None,
        parent_task_id: None,
        schema: None,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        cwd: None,
    }
}

fn descriptor() -> AgentLaunchDescriptor {
    AgentLaunchDescriptor::new(
        PluginId::new("plugin.agents"),
        "variant-agent",
        "Variant Agent",
        "Launches a plugin-defined agent variant.",
    )
    .with_executor(AgentLaunchExecutorDescriptor::process_bridge("sh"))
}

#[tokio::test]
async fn process_bridge_agent_launch_executes_jsonl_contract_normal() {
    // Given: a process bridge command that echoes a valid agent-launch result.
    let command = ProcessBridgeCommand::new("sh").with_args([
        "-c",
        r#"read line
id=$(printf '%s' "$line" | sed -n 's/.*"id":"\([^"]*\)".*/\1/p')
printf '{"type":"response","id":"%s","response":{"kind":"agent_launch_result","result":{"output":"bridge launched","is_error":false}}}\n' "$id"
"#,
    ]);
    let descriptor = descriptor();
    let task = task_input();

    // When: the engine launches the agent through the process bridge.
    let result = execute_process_bridge_agent_launch(ProcessBridgeAgentLaunchInvocation {
        descriptor: &descriptor,
        command: &command,
        task_input: &task,
        task_id: Some("task_1"),
        cwd: None,
        model_id: None,
        provider_name: Some("test-provider"),
        active_team_name: None,
    })
    .await;

    // Then: the bridge response becomes the observable Task result.
    assert!(!result.is_error());
    assert_eq!(result.output, "bridge launched");
}

#[test]
fn agent_launch_result_error_maps_to_failed_execution_robust() {
    // Given: an agent-launch result frame that reports failure.
    let result = BridgeAgentLaunchResult::failure("agent failed");

    // When: it is converted to the engine execution result.
    let execution_result = agent_result_to_execution_result(result);

    // Then: the failure is surfaced as a failed tool result.
    assert!(execution_result.is_error());
    assert_eq!(execution_result.output, "agent failed");
}
