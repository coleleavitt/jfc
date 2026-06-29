use tokio::sync::{mpsc, watch};

use crate::ids::{TaskId, ToolId};
use crate::runtime::{
    EngineEvent, ExecutionResult, TaskEvent, TeamEvent, ToolEvent, send_critical,
};

use super::types::{BackendType, TeamMember};

pub struct SpawnedTeammate {
    pub tool_task_id: String,
    pub runner_task_id: String,
    pub launcher_name: String,
    pub launch_backend_label: &'static str,
    pub name: String,
    pub team_name: String,
    pub agent_id: String,
    pub color: String,
    pub agent_type: Option<String>,
    pub model_name: String,
    pub max_input_tokens: Option<u64>,
    pub parent_task_id: Option<String>,
    pub project_root: std::path::PathBuf,
    pub backend_type: BackendType,
    pub abort_tx: watch::Sender<bool>,
    pub plan_mode_required: bool,
    pub mode: Option<String>,
}

pub fn record_spawned_teammate(tx: &mpsc::Sender<EngineEvent>, teammate: SpawnedTeammate) {
    register_agent(&teammate);
    persist_member(&teammate);
    emit_spawn_events(tx, teammate);
}

fn register_agent(teammate: &SpawnedTeammate) {
    let agent_id_label = teammate.agent_id.clone();
    let name_label = teammate.name.clone();
    let team = teammate.team_name.clone();
    tokio::spawn(async move {
        let registry = crate::tools::agent_registry();
        let id = jfc_agent::AgentId::from_label(&agent_id_label);
        registry
            .register(jfc_agent::AgentState::new(
                id.clone(),
                jfc_agent::AgentRole::Teammate { team_name: team },
                name_label,
            ))
            .await;
        registry
            .update_status(&id, jfc_agent::AgentStatus::Running)
            .await;
    });
}

fn persist_member(teammate: &SpawnedTeammate) {
    let member = TeamMember {
        agent_id: teammate.agent_id.clone(),
        name: teammate.name.clone(),
        agent_type: teammate.agent_type.clone(),
        model: Some(teammate.model_name.clone()),
        color: Some(teammate.color.clone()),
        plan_mode_required: Some(teammate.plan_mode_required),
        joined_at: joined_at_millis(),
        cwd: None,
        worktree_path: None,
        backend_type: Some(teammate.backend_type),
        is_active: Some(true),
        mode: teammate.mode.clone(),
    };
    let team_name = teammate.team_name.clone();
    tokio::spawn(async move {
        if let Err(error) = crate::swarm::team_helpers::add_member(&team_name, member).await {
            tracing::warn!(
                target: "jfc::swarm",
                error = %error,
                "failed to register spawned teammate in team file"
            );
        }
    });
}

fn joined_at_millis() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn emit_spawn_events(tx: &mpsc::Sender<EngineEvent>, teammate: SpawnedTeammate) {
    let result_json = serde_json::json!({
        "status": "teammate_spawned",
        "launcher": teammate.launcher_name,
        "launch_backend": teammate.launch_backend_label,
        "teammate_id": teammate.agent_id,
        "name": teammate.name,
        "team_name": teammate.team_name,
        "color": teammate.color,
        "message": format!("Spawned successfully.\nagent_id: {}\nname: {}\nteam_name: {}\nThe agent is now running and will receive instructions via mailbox.", teammate.agent_id, teammate.name, teammate.team_name)
    });

    send_critical(
        tx,
        EngineEvent::Team(TeamEvent::Spawned {
            name: teammate.name.clone(),
            team_name: teammate.team_name.clone(),
            agent_id: teammate.agent_id.clone(),
            color: Some(teammate.color.clone()),
            agent_type: teammate.agent_type.clone(),
            cwd: teammate.project_root.to_string_lossy().into_owned(),
            backend_type: teammate.backend_type,
            abort_tx: Some(teammate.abort_tx),
        }),
    );
    send_critical(
        tx,
        EngineEvent::Task(TaskEvent::Started {
            task_id: TaskId::from(teammate.runner_task_id),
            description: format!("spawn teammate: {}", teammate.name),
            model_used: Some(teammate.model_name),
            max_input_tokens: teammate.max_input_tokens,
            is_detached: false,
            parent_task_id: teammate.parent_task_id,
        }),
    );
    send_critical(
        tx,
        EngineEvent::Tool(ToolEvent::Result {
            tool_id: ToolId::from(teammate.tool_task_id),
            result: ExecutionResult::success(
                serde_json::to_string_pretty(&result_json).unwrap_or_default(),
            ),
        }),
    );
}
