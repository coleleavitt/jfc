//! Swarm dispatch: teammate spawning logic extracted from `stream::tool_dispatch`.
//!
//! This module handles the teammate-spawn path of the Task tool: when a Task
//! call carries both `name` and `team_name`, the call is routed here to spin
//! up a persistent in-process teammate rather than a one-shot subagent.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::agents::AgentDef;
use crate::ids::ToolId;
use crate::runtime::{EngineEvent, ExecutionResult, ToolEvent, send_critical};
use crate::swarm::process_bridge_teammate::{
    ProcessBridgeTeammateConfig, start_process_bridge_teammate,
};
use crate::swarm::runner::{
    TeammateEvent, TeammateRunnerConfig, assign_teammate_color, start_teammate, teammate_task_id,
};
use crate::swarm::spawn_lifecycle::{SpawnedTeammate, record_spawned_teammate};
use crate::swarm::types::{BackendType, TeammateIdentity, make_agent_id};
use jfc_core::TaskInput;
use jfc_provider::{ModelId, Provider};

/// Attempt to spawn a teammate for the given `TaskInput`. Returns `true` if
/// the input was a teammate spawn (regardless of success/failure), meaning
/// the caller should skip the normal subagent path. Returns `false` if this
/// isn't a teammate spawn request.
pub fn try_spawn_teammate(
    task_input: &TaskInput,
    task_id: &str,
    tx: &mpsc::Sender<EngineEvent>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    agents: &[AgentDef],
    current_session_id: Option<&str>,
    teammate_event_tx: mpsc::UnboundedSender<TeammateEvent>,
    // Full provider registry, so a teammate whose selected model belongs to a
    // different provider than the leader (e.g. a `gpt-5.5` teammate spawned
    // from a Claude leader) is bound to ITS OWN provider rather than silently
    // inheriting the leader's. Empty falls back to the leader's provider.
    registry: &[Arc<dyn Provider>],
    done: impl FnOnce() + Send + 'static,
) -> bool {
    if !task_input.is_teammate_spawn() {
        return false;
    }

    let tx_task = tx.clone();
    let task_id = task_id.to_owned();
    let project_root = task_input
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
        });
    let launch_plan =
        match crate::agents::select_teammate_agent_launch_plan(task_input, &project_root) {
            Ok(plan) => plan,
            Err(error) => {
                send_critical(
                    &tx_task,
                    EngineEvent::Tool(ToolEvent::Result {
                        tool_id: ToolId::from(task_id),
                        result: ExecutionResult::failure(format!(
                            "Teammate launch descriptor unavailable: {error}"
                        )),
                    }),
                );
                done();
                return true;
            }
        };
    match &launch_plan.backend {
        crate::agents::AgentLaunchBackend::InProcess => {}
        crate::agents::AgentLaunchBackend::BackgroundWorker => {
            send_critical(
                &tx_task,
                EngineEvent::Tool(ToolEvent::Result {
                    tool_id: ToolId::from(task_id),
                    result: ExecutionResult::failure(format!(
                        "Teammate launcher {} resolved to a background-worker backend",
                        launch_plan.descriptor.name
                    )),
                }),
            );
            done();
            return true;
        }
        crate::agents::AgentLaunchBackend::ProcessBridge { .. } => {}
    }

    let name = task_input.name.clone().unwrap_or_default();
    let team_name = task_input.team_name.clone().unwrap_or_default();
    let agent_id = make_agent_id(&name, &team_name);
    let color = assign_teammate_color();
    let agent_def = task_input
        .subagent_type
        .as_deref()
        .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)));
    let (teammate_provider, teammate_model) = match crate::tools::selected_subagent_provider_model(
        task_input,
        agent_def,
        provider.clone(),
        model,
        registry,
    ) {
        Ok(resolved) => resolved,
        Err(error) => {
            send_critical(
                &tx_task,
                EngineEvent::Tool(ToolEvent::Result {
                    tool_id: ToolId::from(task_id),
                    result: ExecutionResult::failure(error),
                }),
            );
            done();
            return true;
        }
    };
    let teammate_model_name = teammate_model.as_str().to_string();

    let runner_task_id = teammate_task_id(&agent_id);
    let (abort_tx, launch_backend_type, launch_backend_label) = match &launch_plan.backend {
        crate::agents::AgentLaunchBackend::InProcess => {
            let config = TeammateRunnerConfig {
                identity: TeammateIdentity {
                    agent_id: agent_id.clone(),
                    agent_name: name.clone(),
                    team_name: team_name.clone(),
                    color: Some(color.clone()),
                    plan_mode_required: task_input.mode.as_deref() == Some("plan"),
                    parent_session_id: current_session_id.unwrap_or("").to_owned(),
                },
                prompt: task_input.prompt.clone(),
                description: task_input.description.clone(),
                model: Some(teammate_model_name.clone()),
                agent_type: task_input.subagent_type.clone(),
                provider: teammate_provider.clone(),
                model_id: teammate_model.clone(),
                system_prompt: None,
                task_store: Some(jfc_session::TaskStore::open_team(&team_name)),
            };
            let (_task_id, abort_tx) = start_teammate(config, teammate_event_tx);
            (abort_tx, BackendType::InProcess, "in_process")
        }
        crate::agents::AgentLaunchBackend::ProcessBridge { command } => {
            let launch = ProcessBridgeTeammateConfig {
                descriptor: launch_plan.descriptor.clone(),
                command: command.clone(),
                task_input: task_input.clone(),
                task_id: runner_task_id.clone(),
                agent_id: agent_id.clone(),
                cwd: project_root.clone(),
                model_id: Some(teammate_model.clone()),
                provider_name: Some(teammate_provider.name().to_owned()),
                active_team_name: Some(team_name.clone()),
            };
            match start_process_bridge_teammate(launch, teammate_event_tx) {
                Ok(abort_tx) => (abort_tx, BackendType::ProcessBridge, "process_bridge"),
                Err(result) => {
                    send_critical(
                        &tx_task,
                        EngineEvent::Tool(ToolEvent::Result {
                            tool_id: ToolId::from(task_id),
                            result,
                        }),
                    );
                    done();
                    return true;
                }
            }
        }
        crate::agents::AgentLaunchBackend::BackgroundWorker => unreachable!(),
    };
    tracing::debug!(
        target: "jfc::swarm",
        launcher = %launch_plan.descriptor.name,
        handler = %launch_plan.descriptor.executor.handler,
        "selected descriptor-owned teammate launch backend"
    );

    record_spawned_teammate(
        &tx_task,
        SpawnedTeammate {
            tool_task_id: task_id,
            runner_task_id,
            launcher_name: launch_plan.descriptor.name,
            launch_backend_label,
            name,
            team_name,
            agent_id,
            color,
            agent_type: task_input.subagent_type.clone(),
            model_name: teammate_model_name,
            max_input_tokens: agent_def.and_then(|a| a.max_input_tokens),
            parent_task_id: task_input.parent_task_id.clone(),
            project_root,
            backend_type: launch_backend_type,
            abort_tx,
            plan_mode_required: task_input.mode.as_deref() == Some("plan"),
            mode: task_input.mode.clone(),
        },
    );
    done();
    true
}

/// Resolve the `(provider, model)` a teammate should run under. The selected
/// `model` is matched against the full provider `registry`: when a provider
/// serves that model id, the teammate is bound to it (enabling heterogeneous
/// teammates — e.g. a `gpt-5.5` teammate under a Claude leader). When nothing
/// resolves (no registry, or an unknown id), the teammate falls back to the
/// leader's `fallback_provider` and the unchanged model id.
#[cfg(test)]
fn bind_teammate_provider(
    registry: &[Arc<dyn Provider>],
    fallback_provider: Arc<dyn Provider>,
    model: ModelId,
) -> (Arc<dyn Provider>, ModelId) {
    match crate::runtime::bootstrap::resolve_provider_model(registry, model.as_str()) {
        Some(res) => (res.provider, res.model),
        None => (fallback_provider, model),
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod teammate_provider_tests;
