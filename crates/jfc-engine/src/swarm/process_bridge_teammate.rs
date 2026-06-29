use std::path::PathBuf;
use std::process::Stdio;

use jfc_plugin_sdk::{
    AgentLaunchDescriptor, BridgeAgentLaunchRequest, BridgeEnvelope, BridgeRequest,
    ProcessBridgeCommand,
};
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};
use tokio::sync::{mpsc, watch};

use crate::runtime::{ExecutionResult, ToolErrorCategory};

use super::process_bridge_teammate_loop::run_until_terminal;
use super::runner::TeammateEvent;

pub struct ProcessBridgeTeammateConfig {
    pub descriptor: AgentLaunchDescriptor,
    pub command: ProcessBridgeCommand,
    pub task_input: jfc_core::TaskInput,
    pub task_id: String,
    pub agent_id: String,
    pub cwd: PathBuf,
    pub model_id: Option<jfc_provider::ModelId>,
    pub provider_name: Option<String>,
    pub active_team_name: Option<String>,
}

struct ProcessBridgeTeammateRequest {
    id: String,
    line: String,
}

pub fn start_process_bridge_teammate(
    config: ProcessBridgeTeammateConfig,
    event_tx: mpsc::UnboundedSender<TeammateEvent>,
) -> Result<watch::Sender<bool>, ExecutionResult> {
    let request = bridge_request_line(&config)?;
    let child = spawn_bridge_process(&config.command, &config.descriptor.name)?;
    let (abort_tx, abort_rx) = watch::channel(false);
    tokio::spawn(async move {
        run_process_bridge_teammate(config, request, child, abort_rx, event_tx).await;
    });
    Ok(abort_tx)
}

fn bridge_request_line(
    config: &ProcessBridgeTeammateConfig,
) -> Result<ProcessBridgeTeammateRequest, ExecutionResult> {
    let request_id = format!("teammate-{}", uuid::Uuid::new_v4());
    let line = serde_json::to_string(&BridgeEnvelope::request(
        request_id.clone(),
        BridgeRequest::AgentLaunch {
            launch: bridge_agent_launch_request(config),
        },
    ))
    .map_err(|error| {
        ExecutionResult::structured_failure(
            format!(
                "ProcessBridge teammate launcher `{}` could not serialize request: {error}",
                config.descriptor.name
            ),
            ToolErrorCategory::Validation,
            false,
        )
    })?;
    Ok(ProcessBridgeTeammateRequest {
        id: request_id,
        line,
    })
}

fn bridge_agent_launch_request(config: &ProcessBridgeTeammateConfig) -> BridgeAgentLaunchRequest {
    let mut request =
        BridgeAgentLaunchRequest::new(config.descriptor.name.clone(), config.task_input.clone())
            .with_task_id(&config.task_id)
            .with_cwd(config.cwd.display().to_string());
    if let Some(model_id) = &config.model_id {
        request = request.with_model(model_id.as_str());
    }
    if let Some(provider_name) = &config.provider_name {
        request = request.with_provider(provider_name);
    }
    if let Some(active_team_name) = &config.active_team_name {
        request = request.with_active_team_name(active_team_name);
    }
    request
}

fn spawn_bridge_process(
    command: &ProcessBridgeCommand,
    launcher_name: &str,
) -> Result<Child, ExecutionResult> {
    Command::new(&command.command)
        .args(&command.args)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            ExecutionResult::structured_failure(
                format!(
                    "ProcessBridge teammate launcher `{launcher_name}` could not start `{}`: {error}",
                    command.command
                ),
                ToolErrorCategory::Configuration,
                false,
            )
        })
}

async fn run_process_bridge_teammate(
    config: ProcessBridgeTeammateConfig,
    request: ProcessBridgeTeammateRequest,
    mut child: Child,
    mut abort_rx: watch::Receiver<bool>,
    event_tx: mpsc::UnboundedSender<TeammateEvent>,
) {
    let mut stdin = match write_initial_request(&mut child, &request.line).await {
        Ok(stdin) => stdin,
        Err(error) => {
            let _ = child.start_kill();
            let _ = event_tx.send(TeammateEvent::Failed {
                task_id: config.task_id,
                agent_id: config.agent_id,
                error,
            });
            return;
        }
    };

    let terminal_event = run_until_terminal(
        &config,
        &request.id,
        &mut child,
        &mut stdin,
        &mut abort_rx,
        &event_tx,
    )
    .await;
    let _ = event_tx.send(terminal_event);
}

async fn write_initial_request(
    child: &mut Child,
    request_line: &str,
) -> Result<ChildStdin, String> {
    let Some(mut stdin) = child.stdin.take() else {
        return Err("ProcessBridge teammate launcher could not open bridge stdin".to_owned());
    };
    stdin
        .write_all(request_line.as_bytes())
        .await
        .map_err(|error| {
            format!("ProcessBridge teammate launcher failed to write request: {error}")
        })?;
    stdin.write_all(b"\n").await.map_err(|error| {
        format!("ProcessBridge teammate launcher failed to write newline: {error}")
    })?;
    Ok(stdin)
}
