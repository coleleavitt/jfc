use std::process::Stdio;
use std::time::Duration;

use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeRequest, BridgeResponse, ProcessBridgeCommand, ToolDescriptor,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::runtime::{ExecutionResult, ToolErrorCategory};
use crate::types::ToolInput;

const PROCESS_BRIDGE_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) async fn execute_process_bridge_descriptor_route(
    descriptor: &ToolDescriptor,
    input: &ToolInput,
) -> ExecutionResult {
    let command = match parse_process_bridge_handler(&descriptor.executor.handler) {
        Ok(command) => command,
        Err(result) => return result,
    };
    let request_id = format!("tool-{}", uuid::Uuid::new_v4());
    let request = BridgeEnvelope::request(
        request_id.clone(),
        BridgeRequest::ToolCall {
            tool: descriptor.name.clone(),
            tool_id: None,
            input: input.to_value(),
        },
    );
    let request_line = match serde_json::to_string(&request) {
        Ok(line) => line,
        Err(error) => {
            return ExecutionResult::structured_failure(
                format!(
                    "ProcessBridge descriptor `{}` could not serialize request: {error}",
                    descriptor.name
                ),
                ToolErrorCategory::Validation,
                false,
            );
        }
    };

    let mut child = match Command::new(&command.command)
        .args(&command.args)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            return ExecutionResult::structured_failure(
                format!(
                    "ProcessBridge descriptor `{}` could not start `{}`: {error}",
                    descriptor.name, command.command
                ),
                ToolErrorCategory::Configuration,
                false,
            );
        }
    };

    let Some(mut stdin) = child.stdin.take() else {
        return ExecutionResult::structured_failure(
            format!(
                "ProcessBridge descriptor `{}` could not open bridge stdin",
                descriptor.name
            ),
            ToolErrorCategory::Configuration,
            false,
        );
    };

    if let Err(error) = stdin.write_all(request_line.as_bytes()).await {
        return bridge_io_failure(descriptor, "write request", error);
    }
    if let Err(error) = stdin.write_all(b"\n").await {
        return bridge_io_failure(descriptor, "write newline", error);
    }
    drop(stdin);

    let output = match tokio::time::timeout(PROCESS_BRIDGE_TIMEOUT, child.wait_with_output()).await
    {
        Ok(Ok(output)) => output,
        Ok(Err(error)) => return bridge_io_failure(descriptor, "wait for response", error),
        Err(_) => {
            return ExecutionResult::structured_failure(
                format!(
                    "ProcessBridge descriptor `{}` timed out after {}s",
                    descriptor.name,
                    PROCESS_BRIDGE_TIMEOUT.as_secs()
                ),
                ToolErrorCategory::Transient,
                true,
            );
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout.lines().find(|line| !line.trim().is_empty()) else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        let stderr_suffix = if stderr.is_empty() {
            String::new()
        } else {
            format!("; stderr: {stderr}")
        };
        return ExecutionResult::structured_failure(
            format!(
                "ProcessBridge descriptor `{}` exited without a response frame{}",
                descriptor.name, stderr_suffix
            ),
            if output.status.success() {
                ToolErrorCategory::Validation
            } else {
                ToolErrorCategory::Business
            },
            false,
        );
    };

    response_frame_to_result(descriptor, &request_id, line)
}

fn parse_process_bridge_handler(handler: &str) -> Result<ProcessBridgeCommand, ExecutionResult> {
    let trimmed = handler.trim();
    if trimmed.is_empty() {
        return Err(ExecutionResult::structured_failure(
            "ProcessBridge descriptor handler is empty".to_string(),
            ToolErrorCategory::Configuration,
            false,
        ));
    }
    if trimmed.starts_with('{') {
        return serde_json::from_str::<ProcessBridgeCommand>(trimmed).map_err(|error| {
            ExecutionResult::structured_failure(
                format!("ProcessBridge descriptor handler JSON is invalid: {error}"),
                ToolErrorCategory::Configuration,
                false,
            )
        });
    }
    Ok(ProcessBridgeCommand::new(trimmed))
}

fn response_frame_to_result(
    descriptor: &ToolDescriptor,
    request_id: &str,
    line: &str,
) -> ExecutionResult {
    let frame = match serde_json::from_str::<BridgeEnvelope>(line) {
        Ok(frame) => frame,
        Err(error) => {
            return ExecutionResult::structured_failure(
                format!(
                    "ProcessBridge descriptor `{}` returned invalid JSONL: {error}",
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
                        "ProcessBridge descriptor `{}` response id `{id}` did not match `{request_id}`",
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
                "ProcessBridge descriptor `{}` returned a request frame, expected response",
                descriptor.name
            ),
            ToolErrorCategory::Validation,
            false,
        ),
    }
}

fn bridge_response_to_result(response: BridgeResponse) -> ExecutionResult {
    match response {
        BridgeResponse::ToolResult {
            output,
            is_error,
            payload,
        } => {
            let text = if output.is_empty() {
                payload.map(|value| value.to_string()).unwrap_or_default()
            } else {
                output
            };
            if is_error {
                ExecutionResult::structured_failure(text, ToolErrorCategory::Business, false)
            } else {
                ExecutionResult::success(text)
            }
        }
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

fn bridge_io_failure(
    descriptor: &ToolDescriptor,
    action: &str,
    error: impl std::fmt::Display,
) -> ExecutionResult {
    ExecutionResult::structured_failure(
        format!(
            "ProcessBridge descriptor `{}` failed to {action}: {error}",
            descriptor.name
        ),
        ToolErrorCategory::Transient,
        true,
    )
}
