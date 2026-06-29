use jfc_plugin_host::{
    BUILTIN_AGENT_LAUNCH_HANDLER, BUILTIN_AGENT_LAUNCH_ID, BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER,
    BUILTIN_BACKGROUND_AGENT_LAUNCH_ID, builtin_agent_workflow_plugin_host,
};
use jfc_plugin_sdk::{AgentLaunchDescriptor, AgentLaunchExecutorDescriptor, PluginId};

use super::*;

#[test]
fn builtin_agent_launch_descriptor_selects_in_process_backend_normal() {
    // Given: the active first-party agent plugin host.
    let host = builtin_agent_workflow_plugin_host().expect("built-in plugins activate");

    // When: the engine selects the agent launch plan from active descriptors.
    let plan = select_builtin_agent_launch_plan(&host).expect("launch plan");

    // Then: Task execution is routed through the descriptor-owned in-process backend.
    assert_eq!(plan.backend, AgentLaunchBackend::InProcess);
    assert_eq!(plan.descriptor.name, BUILTIN_AGENT_LAUNCH_ID);
    assert_eq!(
        plan.descriptor.executor.handler,
        BUILTIN_AGENT_LAUNCH_HANDLER
    );
}

#[test]
fn builtin_background_agent_launch_descriptor_selects_worker_backend_normal() {
    // Given: the active first-party agent plugin host.
    let host = builtin_agent_workflow_plugin_host().expect("built-in plugins activate");

    // When: the engine selects the detached background-worker launch plan.
    let plan = select_background_agent_launch_plan(&host).expect("background launch plan");

    // Then: detached Task execution is routed through the descriptor-owned worker backend.
    assert_eq!(plan.backend, AgentLaunchBackend::BackgroundWorker);
    assert_eq!(plan.descriptor.name, BUILTIN_BACKGROUND_AGENT_LAUNCH_ID);
    assert_eq!(
        plan.descriptor.executor.handler,
        BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER
    );
}

#[test]
fn malformed_builtin_agent_launch_descriptor_is_rejected_robust() {
    // Given: a descriptor that claims to be the built-in launcher but names the wrong handler.
    let descriptor = AgentLaunchDescriptor::new(
        PluginId::new("builtin.jfc-agents"),
        BUILTIN_AGENT_LAUNCH_ID,
        "Broken launcher",
        "Wrong handler",
    )
    .with_executor(AgentLaunchExecutorDescriptor::built_in(
        "jfc-engine::agents::other",
    ));

    // When: the engine tries to turn it into an executable launch plan.
    let result = plan_from_agent_launch_descriptor(&descriptor);

    // Then: the descriptor is rejected before any agent work is started.
    assert!(matches!(
        result,
        Err(AgentLaunchError::UnsupportedBuiltInHandler { .. })
    ));
}

#[test]
fn process_bridge_agent_launch_descriptor_selects_process_backend_normal() {
    // Given: a plugin-declared launch descriptor with a process bridge executor.
    let descriptor = AgentLaunchDescriptor::new(
        PluginId::new("plugin.agents"),
        "variant-agent",
        "Variant Agent",
        "Launches a plugin-defined agent variant.",
    )
    .with_executor(AgentLaunchExecutorDescriptor::process_bridge(
        r#"{"command":"variant-agent","args":["--jsonl"]}"#,
    ));

    // When: the engine turns the descriptor into an executable launch plan.
    let plan = plan_from_agent_launch_descriptor(&descriptor).expect("launch plan");

    // Then: the process bridge command is preserved as the launch backend.
    match plan.backend {
        AgentLaunchBackend::ProcessBridge { command } => {
            assert_eq!(command.command, "variant-agent");
            assert_eq!(command.args, ["--jsonl"]);
        }
        other => panic!("expected process bridge backend, got {other:?}"),
    }
}

#[test]
fn project_agent_launch_descriptor_selects_from_cached_plugin_state_normal() {
    // Given: a project plugin declares a process-bridge agent launcher.
    jfc_plugin_host::clear_discovered_plugin_state_cache_for_tests();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let plugins = tmp.path().join("plugins");
    let plugin = plugins.join("agent-plugin");
    std::fs::create_dir_all(plugin.join("workflows")).expect("workflows dir");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "agent-plugin"
workflows_dir = "workflows"

[[agent_launches]]
name = "variant-agent"
label = "Variant Agent"
description = "Launches a plugin-defined variant agent."

[agent_launches.executor]
kind = "process_bridge"
handler = '{"command":"variant-agent","args":["--jsonl"]}'
"#,
    )
    .expect("write plugin manifest");

    // When: the engine selects that launcher by Task-visible name.
    let plan = select_project_agent_launch_plan(tmp.path(), "variant-agent").expect("plan");

    // Then: the process bridge command comes from cached plugin state.
    match plan.backend {
        AgentLaunchBackend::ProcessBridge { command } => {
            assert_eq!(command.command, "variant-agent");
            assert_eq!(command.args, ["--jsonl"]);
        }
        other => panic!("expected process bridge backend, got {other:?}"),
    }
}

#[test]
fn background_task_launcher_selects_project_plugin_process_bridge_robust() {
    // Given: a background Task explicitly names a project plugin launcher.
    jfc_plugin_host::clear_discovered_plugin_state_cache_for_tests();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    install_variant_launcher(tmp.path());
    let task = jfc_core::TaskInput {
        description: "background inspect".to_owned(),
        prompt: "inspect".to_owned(),
        subagent_type: None,
        category: None,
        run_in_background: true,
        model: None,
        launcher: Some("variant-agent".to_owned()),
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
    };

    // When: background launch policy resolves the Task launcher.
    let plan = select_background_task_agent_launch_plan(&task, tmp.path()).expect("launch plan");

    // Then: the caller sees the plugin process bridge and can fail closed until durable lifecycle support exists.
    match plan.backend {
        AgentLaunchBackend::ProcessBridge { command } => {
            assert_eq!(command.command, "variant-agent");
            assert_eq!(command.args, ["--jsonl"]);
        }
        other => panic!("expected process bridge backend, got {other:?}"),
    }
}

#[test]
fn teammate_launcher_selects_project_plugin_process_bridge_robust() {
    // Given: a teammate Task explicitly names a project plugin launcher.
    jfc_plugin_host::clear_discovered_plugin_state_cache_for_tests();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    install_variant_launcher(tmp.path());
    let task = jfc_core::TaskInput {
        description: "spawn teammate".to_owned(),
        prompt: "inspect".to_owned(),
        subagent_type: None,
        category: None,
        run_in_background: false,
        model: None,
        launcher: Some("variant-agent".to_owned()),
        effort: None,
        name: Some("reviewer".to_owned()),
        team_name: Some("alpha".to_owned()),
        mode: None,
        isolation: None,
        parent_task_id: None,
        schema: None,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        cwd: None,
    };

    // When: teammate launch policy resolves the Task launcher.
    let plan = select_teammate_agent_launch_plan(&task, tmp.path()).expect("launch plan");

    // Then: the caller sees the plugin process bridge and can fail closed until teammate lifecycle support exists.
    match plan.backend {
        AgentLaunchBackend::ProcessBridge { command } => {
            assert_eq!(command.command, "variant-agent");
            assert_eq!(command.args, ["--jsonl"]);
        }
        other => panic!("expected process bridge backend, got {other:?}"),
    }
}

#[test]
fn background_worker_execution_clears_builtin_background_launcher_normal() {
    // Given: a background Task explicitly names the built-in detached worker launcher.
    let mut task = task_input(None);
    task.run_in_background = true;
    task.launcher = Some(BUILTIN_BACKGROUND_AGENT_LAUNCH_ID.to_owned());
    let plan = select_default_background_agent_launch_plan().expect("background plan");

    // When: the daemon worker prepares the task for in-worker execution.
    let worker_task = background_worker_execution_task_input(&task, &plan);

    // Then: the worker does not recursively select the detached-worker launcher again.
    assert_eq!(worker_task.launcher, None);
    assert!(worker_task.run_in_background);
}

#[test]
fn background_worker_execution_preserves_plugin_process_launcher_normal() {
    // Given: a background Task explicitly names a project plugin process-bridge launcher.
    jfc_plugin_host::clear_discovered_plugin_state_cache_for_tests();
    let tmp = tempfile::TempDir::new().expect("tempdir");
    install_variant_launcher(tmp.path());
    let mut task = task_input(None);
    task.run_in_background = true;
    task.launcher = Some("variant-agent".to_owned());
    let plan = select_background_task_agent_launch_plan(&task, tmp.path()).expect("launch plan");

    // When: the daemon worker prepares the task for in-worker execution.
    let worker_task = background_worker_execution_task_input(&task, &plan);

    // Then: the selected plugin launcher is preserved for process-bridge execution.
    assert_eq!(worker_task.launcher.as_deref(), Some("variant-agent"));
}

fn task_input(launcher: Option<&str>) -> jfc_core::TaskInput {
    jfc_core::TaskInput {
        description: "inspect".to_owned(),
        prompt: "inspect".to_owned(),
        subagent_type: None,
        category: None,
        run_in_background: false,
        model: None,
        launcher: launcher.map(str::to_owned),
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

fn install_variant_launcher(project_root: &std::path::Path) {
    let plugin = project_root.join("plugins").join("agent-plugin");
    std::fs::create_dir_all(plugin.join("workflows")).expect("workflows dir");
    std::fs::write(
        plugin.join(".jfc-plugin.toml"),
        r#"[plugin]
name = "agent-plugin"
workflows_dir = "workflows"

[[agent_launches]]
name = "variant-agent"
label = "Variant Agent"
description = "Launches a plugin-defined variant agent."

[agent_launches.executor]
kind = "process_bridge"
handler = '{"command":"variant-agent","args":["--jsonl"]}'
"#,
    )
    .expect("write plugin manifest");
}
