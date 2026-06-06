//! Swarm dispatch: teammate spawning logic extracted from `stream::tool_dispatch`.
//!
//! This module handles the teammate-spawn path of the Task tool: when a Task
//! call carries both `name` and `team_name`, the call is routed here to spin
//! up a persistent in-process teammate rather than a one-shot subagent.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::agents::AgentDef;
use crate::ids::{TaskId, ToolId};
use crate::runtime::{EngineEvent, ExecutionResult, TaskEvent, TeamEvent, ToolEvent, send_critical};
use crate::swarm::runner::{
    TeammateEvent, TeammateRunnerConfig, assign_teammate_color, start_teammate, teammate_task_id,
};
use crate::swarm::types::{BackendType, TeamMember, TeammateIdentity, make_agent_id};
use jfc_core::TaskInput;
use jfc_provider::{ModelId, Provider};

/// Attempt to spawn a teammate for the given `TaskInput`. Returns `true` if
/// the input was a teammate spawn (regardless of success/failure), meaning
/// the caller should skip the normal subagent path. Returns `false` if this
/// isn't a teammate spawn request.
pub(crate) fn try_spawn_teammate(
    task_input: &TaskInput,
    task_id: &str,
    tx: &mpsc::Sender<EngineEvent>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    agents: &[AgentDef],
    current_session_id: Option<&str>,
    teammate_event_tx: mpsc::UnboundedSender<TeammateEvent>,
    done: impl FnOnce() + Send + 'static,
) -> bool {
    if !task_input.is_teammate_spawn() {
        return false;
    }

    let tx_task = tx.clone();
    let task_id = task_id.to_owned();

    let name = task_input.name.clone().unwrap_or_default();
    let team_name = task_input.team_name.clone().unwrap_or_default();
    let agent_id = make_agent_id(&name, &team_name);
    let color = assign_teammate_color();
    let agent_def = task_input
        .subagent_type
        .as_deref()
        .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)));
    let teammate_model = match crate::tools::selected_subagent_model(
        task_input,
        agent_def,
        model,
        provider.name(),
    ) {
        Ok(model) => model,
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
        provider: provider.clone(),
        model_id: teammate_model,
        system_prompt: None,
        task_store: Some(jfc_session::TaskStore::open_team(&team_name)),
    };

    let (_runner_task_id, abort_tx) = start_teammate(config, teammate_event_tx);

    // Persist the new member into the team file so the team roster on disk
    // matches the runtime spawn list.
    let member = TeamMember {
        agent_id: agent_id.clone(),
        name: name.clone(),
        agent_type: task_input.subagent_type.clone(),
        model: Some(teammate_model_name.clone()),
        color: Some(color.clone()),
        plan_mode_required: Some(task_input.mode.as_deref() == Some("plan")),
        joined_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0),
        cwd: None,
        worktree_path: None,
        backend_type: Some(BackendType::InProcess),
        is_active: Some(true),
        mode: task_input.mode.clone(),
    };
    {
        let team_name_clone = team_name.clone();
        tokio::spawn(async move {
            if let Err(e) = crate::swarm::team_helpers::add_member(&team_name_clone, member).await {
                tracing::warn!(
                    target: "jfc::swarm",
                    error = %e,
                    "failed to register spawned teammate in team file"
                );
            }
        });
    }

    // Report spawn as a successful tool result
    let result_json = serde_json::json!({
        "status": "teammate_spawned",
        "teammate_id": agent_id,
        "name": name,
        "team_name": team_name,
        "color": color,
        "message": format!("Spawned successfully.\nagent_id: {agent_id}\nname: {name}\nteam_name: {team_name}\nThe agent is now running and will receive instructions via mailbox.")
    });

    let runner_task_id = teammate_task_id(&agent_id);
    // Notify the leader's main loop that a teammate exists
    send_critical(
        &tx_task,
        EngineEvent::Team(TeamEvent::Spawned {
            name: name.clone(),
            team_name,
            agent_id,
            color: Some(color),
            agent_type: task_input.subagent_type.clone(),
            cwd: std::env::current_dir()
                .ok()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
            abort_tx: Some(abort_tx),
        }),
    );
    send_critical(
        &tx_task,
        EngineEvent::Task(TaskEvent::Started {
            task_id: TaskId::from(runner_task_id),
            description: format!("spawn teammate: {name}"),
            model_used: Some(teammate_model_name),
            max_input_tokens: agent_def.and_then(|a| a.max_input_tokens),
            is_detached: false,
            parent_task_id: task_input.parent_task_id.clone(),
        }),
    );

    send_critical(
        &tx_task,
        EngineEvent::Tool(ToolEvent::Result {
            tool_id: ToolId::from(task_id),
            result: ExecutionResult::success(
                serde_json::to_string_pretty(&result_json).unwrap_or_default(),
            ),
        }),
    );
    done();
    true
}
