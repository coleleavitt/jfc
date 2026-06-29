use std::path::Path;

use jfc_plugin_host::{
    BUILTIN_AGENT_LAUNCH_HANDLER, BUILTIN_AGENT_LAUNCH_ID, BUILTIN_AGENTS_PLUGIN_ID,
    BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER, BUILTIN_BACKGROUND_AGENT_LAUNCH_ID, PluginHost,
    builtin_agent_workflow_plugin_host, cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{AgentLaunchDescriptor, AgentLaunchExecutorKind, ProcessBridgeCommand};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AgentLaunchBackend {
    InProcess,
    BackgroundWorker,
    ProcessBridge { command: ProcessBridgeCommand },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLaunchPlan {
    pub descriptor: AgentLaunchDescriptor,
    pub backend: AgentLaunchBackend,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentLaunchError {
    #[error("built-in agent launch descriptor is missing")]
    MissingBuiltInLauncher,
    #[error("agent launch descriptor `{name}` is missing")]
    MissingLauncher { name: String },
    #[error("built-in agent launch host failed: {0}")]
    Host(#[from] jfc_plugin_host::PluginHostError),
    #[error("agent launcher {name} uses unsupported executor {executor:?}")]
    UnsupportedExecutor {
        name: String,
        executor: AgentLaunchExecutorKind,
    },
    #[error("agent launcher {name} uses unsupported built-in handler {handler}")]
    UnsupportedBuiltInHandler { name: String, handler: String },
    #[error("agent launcher {name} process bridge handler is invalid: {message}")]
    InvalidProcessBridgeHandler { name: String, message: String },
}

pub fn select_default_agent_launch_plan() -> Result<AgentLaunchPlan, AgentLaunchError> {
    let host = builtin_agent_workflow_plugin_host()?;
    select_builtin_agent_launch_plan(&host)
}

pub fn select_default_background_agent_launch_plan() -> Result<AgentLaunchPlan, AgentLaunchError> {
    let host = builtin_agent_workflow_plugin_host()?;
    select_background_agent_launch_plan(&host)
}

pub fn select_task_agent_launch_plan(
    task_input: &jfc_core::TaskInput,
    project_root: &Path,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    match task_input.launcher.as_deref().map(str::trim) {
        None | Some("") => select_default_agent_launch_plan(),
        Some(BUILTIN_AGENT_LAUNCH_ID) => select_default_agent_launch_plan(),
        Some(BUILTIN_BACKGROUND_AGENT_LAUNCH_ID) => select_default_background_agent_launch_plan(),
        Some(name) => select_project_agent_launch_plan(project_root, name),
    }
}

pub fn select_background_task_agent_launch_plan(
    task_input: &jfc_core::TaskInput,
    project_root: &Path,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    match task_input.launcher.as_deref().map(str::trim) {
        None | Some("") => select_default_background_agent_launch_plan(),
        Some(BUILTIN_BACKGROUND_AGENT_LAUNCH_ID) => select_default_background_agent_launch_plan(),
        Some(BUILTIN_AGENT_LAUNCH_ID) => select_default_agent_launch_plan(),
        Some(name) => select_project_agent_launch_plan(project_root, name),
    }
}

pub fn select_teammate_agent_launch_plan(
    task_input: &jfc_core::TaskInput,
    project_root: &Path,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    match task_input.launcher.as_deref().map(str::trim) {
        None | Some("") => select_default_agent_launch_plan(),
        Some(BUILTIN_AGENT_LAUNCH_ID) => select_default_agent_launch_plan(),
        Some(BUILTIN_BACKGROUND_AGENT_LAUNCH_ID) => select_default_background_agent_launch_plan(),
        Some(name) => select_project_agent_launch_plan(project_root, name),
    }
}

pub fn background_worker_execution_task_input(
    task_input: &jfc_core::TaskInput,
    launch_plan: &AgentLaunchPlan,
) -> jfc_core::TaskInput {
    let mut worker_task_input = task_input.clone();
    if matches!(launch_plan.backend, AgentLaunchBackend::BackgroundWorker) {
        worker_task_input.launcher = None;
    }
    worker_task_input
}

pub fn select_builtin_agent_launch_plan(
    host: &PluginHost,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    select_agent_launch_plan(host, BUILTIN_AGENT_LAUNCH_ID)
}

pub fn select_background_agent_launch_plan(
    host: &PluginHost,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    select_agent_launch_plan(host, BUILTIN_BACKGROUND_AGENT_LAUNCH_ID)
}

pub fn select_project_agent_launch_plan(
    project_root: &Path,
    name: &str,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    let state = cached_discovered_resource_plugin_state(
        crate::workflows::registry::plugin_discovery_options_for(project_root),
    )?;
    select_agent_launch_plan_by_name(&state.host, name)
}

pub fn select_agent_launch_plan_by_name(
    host: &PluginHost,
    name: &str,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    let descriptor = host
        .agent_launch_descriptors()
        .into_iter()
        .find(|descriptor| descriptor.name == name)
        .ok_or_else(|| AgentLaunchError::MissingLauncher {
            name: name.to_owned(),
        })?;
    plan_from_agent_launch_descriptor(&descriptor)
}

fn select_agent_launch_plan(
    host: &PluginHost,
    name: &str,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    let descriptor = host
        .agent_launch_descriptors()
        .into_iter()
        .find(|descriptor| {
            descriptor.plugin_id.as_str() == BUILTIN_AGENTS_PLUGIN_ID && descriptor.name == name
        })
        .ok_or(AgentLaunchError::MissingBuiltInLauncher)?;
    plan_from_agent_launch_descriptor(&descriptor)
}

pub fn plan_from_agent_launch_descriptor(
    descriptor: &AgentLaunchDescriptor,
) -> Result<AgentLaunchPlan, AgentLaunchError> {
    match descriptor.executor.kind {
        AgentLaunchExecutorKind::BuiltIn
            if descriptor.executor.handler == BUILTIN_AGENT_LAUNCH_HANDLER =>
        {
            Ok(AgentLaunchPlan {
                descriptor: descriptor.clone(),
                backend: AgentLaunchBackend::InProcess,
            })
        }
        AgentLaunchExecutorKind::BuiltIn
            if descriptor.executor.handler == BUILTIN_BACKGROUND_AGENT_LAUNCH_HANDLER =>
        {
            Ok(AgentLaunchPlan {
                descriptor: descriptor.clone(),
                backend: AgentLaunchBackend::BackgroundWorker,
            })
        }
        AgentLaunchExecutorKind::BuiltIn => Err(AgentLaunchError::UnsupportedBuiltInHandler {
            name: descriptor.name.clone(),
            handler: descriptor.executor.handler.clone(),
        }),
        AgentLaunchExecutorKind::ProcessBridge => Ok(AgentLaunchPlan {
            descriptor: descriptor.clone(),
            backend: AgentLaunchBackend::ProcessBridge {
                command: parse_process_bridge_handler(
                    &descriptor.name,
                    &descriptor.executor.handler,
                )?,
            },
        }),
    }
}

fn parse_process_bridge_handler(
    name: &str,
    handler: &str,
) -> Result<ProcessBridgeCommand, AgentLaunchError> {
    let trimmed = handler.trim();
    if trimmed.is_empty() {
        return Err(AgentLaunchError::InvalidProcessBridgeHandler {
            name: name.to_owned(),
            message: "handler is empty".to_owned(),
        });
    }
    if trimmed.starts_with('{') {
        return serde_json::from_str::<ProcessBridgeCommand>(trimmed).map_err(|error| {
            AgentLaunchError::InvalidProcessBridgeHandler {
                name: name.to_owned(),
                message: error.to_string(),
            }
        });
    }
    Ok(ProcessBridgeCommand::new(trimmed))
}

#[cfg(test)]
#[path = "launch_tests.rs"]
mod launch_tests;
