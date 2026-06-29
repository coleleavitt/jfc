use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use jfc_plugin_sdk::{
    AgentLaunchDescriptor, BridgeAgentLaunchRequest, BridgeAgentLaunchResult, BridgeEnvelope,
    BridgeRequest, BridgeResponse, ProcessBridgeCommand,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::runtime::{ExecutionResult, ToolErrorCategory};

const AGENT_BRIDGE_TIMEOUT: Duration = Duration::from_secs(30);

pub struct ProcessBridgeAgentLaunchInvocation<'a> {
    pub descriptor: &'a AgentLaunchDescriptor,
    pub command: &'a ProcessBridgeCommand,
    pub task_input: &'a jfc_core::TaskInput,
    pub task_id: Option<&'a str>,
    pub cwd: Option<&'a Path>,
    pub model_id: Option<&'a jfc_provider::ModelId>,
    pub provider_name: Option<&'a str>,
    pub active_team_name: Option<&'a str>,
}

pub async fn execute_process_bridge_agent_launch(
    invocation: ProcessBridgeAgentLaunchInvocation<'_>,
) -> ExecutionResult {
    let request_id = format!("agent-{}", uuid::Uuid::new_v4());
    let request = BridgeEnvelope::request(
        request_id.clone(),
        BridgeRequest::AgentLaunch {
            launch: bridge_agent_launch_request(&invocation),
        },
    );
    let request_line = match serde_json::to_string(&request) {
        Ok(line) => line,
        Err(error) => {
            return ExecutionResult::structured_failure(
                format!(
                    "ProcessBridge agent launcher `{}` could not serialize request: {error}",
                    invocation.descriptor.name
                ),
                ToolErrorCategory::Validation,
                false,
            );
        }
    };

    let mut child = match spawn_bridge_process(invocation.command, &invocation.descriptor.name) {
        Ok(child) => child,
        Err(result) => return result,
    };
    let Some(mut stdin) = child.stdin.take() else {
        return ExecutionResult::structured_failure(
            format!(
                "ProcessBridge agent launcher `{}` could not open bridge stdin",
                invocation.descriptor.name
            ),
            ToolErrorCategory::Configuration,
            false,
        );
    };
    if let Err(error) = stdin.write_all(request_line.as_bytes()).await {
        return bridge_io_failure(invocation.descriptor, "write request", error);
    }
    if let Err(error) = stdin.write_all(b"\n").await {
        return bridge_io_failure(invocation.descriptor, "write newline", error);
    }
    drop(stdin);

    let output = match tokio::time::timeout(AGENT_BRIDGE_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => {
            return bridge_io_failure(invocation.descriptor, "wait for response", error);
        }
        Err(_) => {
            return ExecutionResult::structured_failure(
                format!(
                    "ProcessBridge agent launcher `{}` timed out after {}s",
                    invocation.descriptor.name,
                    AGENT_BRIDGE_TIMEOUT.as_secs()
                ),
                ToolErrorCategory::Transient,
                true,
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout.lines().find(|line| !line.trim().is_empty()) else {
        return missing_response_result(invocation.descriptor, output);
    };
    response_frame_to_result(invocation.descriptor, &request_id, line)
}

fn bridge_agent_launch_request(
    invocation: &ProcessBridgeAgentLaunchInvocation<'_>,
) -> BridgeAgentLaunchRequest {
    let mut request = BridgeAgentLaunchRequest::new(
        invocation.descriptor.name.clone(),
        invocation.task_input.clone(),
    );
    if let Some(task_id) = invocation.task_id {
        request = request.with_task_id(task_id);
    }
    if let Some(cwd) = invocation.cwd {
        request = request.with_cwd(cwd.display().to_string());
    }
    if let Some(model_id) = invocation.model_id {
        request = request.with_model(model_id.as_str());
    }
    if let Some(provider_name) = invocation.provider_name {
        request = request.with_provider(provider_name);
    }
    if let Some(active_team_name) = invocation.active_team_name {
        request = request.with_active_team_name(active_team_name);
    }
    request
}

fn spawn_bridge_process(
    command: &ProcessBridgeCommand,
    launcher_name: &str,
) -> Result<tokio::process::Child, ExecutionResult> {
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
                    "ProcessBridge agent launcher `{launcher_name}` could not start `{}`: {error}",
                    command.command
                ),
                ToolErrorCategory::Configuration,
                false,
            )
        })
}

fn missing_response_result(
    descriptor: &AgentLaunchDescriptor,
    output: std::process::Output,
) -> ExecutionResult {
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stderr = stderr.trim();
    let suffix = if stderr.is_empty() {
        String::new()
    } else {
        format!("; stderr: {stderr}")
    };
    ExecutionResult::structured_failure(
        format!(
            "ProcessBridge agent launcher `{}` exited without a response frame{}",
            descriptor.name, suffix
        ),
        if output.status.success() {
            ToolErrorCategory::Validation
        } else {
            ToolErrorCategory::Business
        },
        false,
    )
}

fn response_frame_to_result(
    descriptor: &AgentLaunchDescriptor,
    request_id: &str,
    line: &str,
) -> ExecutionResult {
    let frame = match serde_json::from_str::<BridgeEnvelope>(line) {
        Ok(frame) => frame,
        Err(error) => {
            return ExecutionResult::structured_failure(
                format!(
                    "ProcessBridge agent launcher `{}` returned invalid JSONL: {error}",
                    descriptor.name
                ),
                ToolErrorCategory::Validation,
                false,
            );
        }
    };
    match frame {
        BridgeEnvelope::Response { id, response } => {
            if id != request_id {
                return ExecutionResult::structured_failure(
                    format!(
                        "ProcessBridge agent launcher `{}` response id `{id}` did not match `{request_id}`",
                        descriptor.name
                    ),
                    ToolErrorCategory::Validation,
                    false,
                );
            }
            bridge_response_to_result(response)
        }
        BridgeEnvelope::Request { .. } => ExecutionResult::structured_failure(
            format!(
                "ProcessBridge agent launcher `{}` returned a request frame, expected response",
                descriptor.name
            ),
            ToolErrorCategory::Validation,
            false,
        ),
    }
}

fn bridge_response_to_result(response: BridgeResponse) -> ExecutionResult {
    match response {
        BridgeResponse::AgentLaunchResult { result } => agent_result_to_execution_result(result),
        BridgeResponse::Error(error) => ExecutionResult::structured_failure(
            format!("ProcessBridge error `{}`: {}", error.code, error.message),
            ToolErrorCategory::Business,
            false,
        ),
        other => ExecutionResult::structured_failure(
            format!("ProcessBridge returned unexpected response: {other:?}"),
            ToolErrorCategory::Validation,
            false,
        ),
    }
}

pub(super) fn agent_result_to_execution_result(result: BridgeAgentLaunchResult) -> ExecutionResult {
    let text = if result.output.is_empty() {
        result
            .payload
            .map(|value| value.to_string())
            .unwrap_or_default()
    } else {
        result.output
    };
    if result.is_error {
        ExecutionResult::structured_failure(text, ToolErrorCategory::Business, false)
    } else {
        ExecutionResult::success(text)
    }
}

fn bridge_io_failure(
    descriptor: &AgentLaunchDescriptor,
    action: &str,
    error: impl std::fmt::Display,
) -> ExecutionResult {
    ExecutionResult::structured_failure(
        format!(
            "ProcessBridge agent launcher `{}` failed to {action}: {error}",
            descriptor.name
        ),
        ToolErrorCategory::Transient,
        true,
    )
}
