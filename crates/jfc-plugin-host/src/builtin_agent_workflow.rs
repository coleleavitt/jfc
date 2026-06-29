use jfc_plugin_sdk::{
    AgentLaunchDescriptor, AgentLaunchExecutorDescriptor, AgentLaunchExecutorKind,
    PluginCapability, PluginId, PluginManifest, PluginScope, PluginSource, PluginVersion,
    ResourceDescriptor, ResourceKind,
};

use crate::{PluginHost, PluginHostError, PluginRegistration};

const AGENT_WORKFLOW_PLUGIN_VERSION: &str = "0.1.0";
pub const BUILTIN_AGENTS_PLUGIN_ID: &str = "builtin.jfc-agents";
pub const BUILTIN_WORKFLOWS_PLUGIN_ID: &str = "builtin.jfc-workflows";
pub const BUILTIN_AGENT_RESOURCE_PATH: &str = "builtin://jfc-agents/agents";
pub const BUILTIN_SKILL_RESOURCE_PATH: &str = "builtin://jfc-agents/skills";
pub const BUILTIN_WORKFLOW_RESOURCE_PATH: &str = "builtin://jfc-engine/workflows";
pub const BUILTIN_AGENT_LAUNCH_ID: &str = "jfc.agents.in_process";
pub const BUILTIN_AGENT_LAUNCH_HANDLER: &str = "jfc-engine::agents::in_process";
pub const BUILTIN_BACKGROUND_AGENT_LAUNCH_ID: &str = "jfc.agents.background_worker";
pub const BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER: &str = "jfc-engine::daemon::background_worker";

pub fn builtin_agent_workflow_plugin_host() -> Result<PluginHost, PluginHostError> {
    let mut host = PluginHost::new();
    register_builtin_agent_workflow_plugins(&mut host)?;
    host.activate_all()?;
    Ok(host)
}

pub fn register_builtin_agent_workflow_plugins(
    host: &mut PluginHost,
) -> Result<(), PluginHostError> {
    host.register_internal(builtin_agents_plugin())?;
    host.register_internal(builtin_workflows_plugin())?;
    Ok(())
}

fn builtin_agents_plugin() -> PluginRegistration {
    let plugin_id = PluginId::new(BUILTIN_AGENTS_PLUGIN_ID);
    let manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new(AGENT_WORKFLOW_PLUGIN_VERSION),
        PluginSource::built_in("jfc-agents"),
    )
    .with_display_name("JFC Agents")
    .with_description("Built-in agent and skill resource pack")
    .with_scope(PluginScope::Workspace)
    .with_capability(PluginCapability::Resources)
    .with_capability(PluginCapability::AgentLaunches {
        executors: vec![AgentLaunchExecutorKind::BuiltIn],
    });

    PluginRegistration::new(manifest)
        .with_resource_descriptors([
            ResourceDescriptor::new(
                plugin_id.clone(),
                ResourceKind::Skill,
                BUILTIN_SKILL_RESOURCE_PATH,
            )
            .with_namespace("builtin"),
            ResourceDescriptor::new(plugin_id, ResourceKind::Agent, BUILTIN_AGENT_RESOURCE_PATH)
                .with_namespace("builtin"),
        ])
        .with_agent_launch_descriptors([
            AgentLaunchDescriptor::new(
                PluginId::new(BUILTIN_AGENTS_PLUGIN_ID),
                BUILTIN_AGENT_LAUNCH_ID,
                "JFC in-process agents",
                "Launches built-in JFC agents through the in-process backend.",
            )
            .with_executor(AgentLaunchExecutorDescriptor::built_in(
                BUILTIN_AGENT_LAUNCH_HANDLER,
            )),
            AgentLaunchDescriptor::new(
                PluginId::new(BUILTIN_AGENTS_PLUGIN_ID),
                BUILTIN_BACKGROUND_AGENT_LAUNCH_ID,
                "JFC background worker agents",
                "Launches detached JFC agents through the background worker.",
            )
            .with_executor(AgentLaunchExecutorDescriptor::built_in(
                BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER,
            )),
        ])
}

fn builtin_workflows_plugin() -> PluginRegistration {
    let plugin_id = PluginId::new(BUILTIN_WORKFLOWS_PLUGIN_ID);
    let manifest = PluginManifest::new(
        plugin_id.clone(),
        PluginVersion::new(AGENT_WORKFLOW_PLUGIN_VERSION),
        PluginSource::built_in("jfc-engine"),
    )
    .with_display_name("JFC Workflows")
    .with_description("Built-in JavaScript workflow resource pack")
    .with_scope(PluginScope::Workspace)
    .with_capability(PluginCapability::Resources);

    PluginRegistration::new(manifest).with_resource_descriptor(
        ResourceDescriptor::new(
            plugin_id,
            ResourceKind::Workflow,
            BUILTIN_WORKFLOW_RESOURCE_PATH,
        )
        .with_namespace("builtin"),
    )
}
