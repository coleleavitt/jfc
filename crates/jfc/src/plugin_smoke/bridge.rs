use std::process::Stdio;
use std::time::Duration;

use anyhow::Context;
use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeProviderStreamEvent, BridgeRequest, BridgeResponse, ProcessBridgeCommand,
};
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

const BRIDGE_SMOKE_TIMEOUT: Duration = Duration::from_secs(180);

pub(super) async fn run_bridge_request(
    command: &ProcessBridgeCommand,
    id: &str,
    request: BridgeRequest,
) -> anyhow::Result<Vec<BridgeEnvelope>> {
    let request = serde_json::to_string(&BridgeEnvelope::request(id, request))?;
    let mut child = Command::new(&command.command)
        .args(&command.args)
        .kill_on_drop(true)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .with_context(|| format!("failed to start process bridge `{}`", command.command))?;
    let mut stdin = child
        .stdin
        .take()
        .context("process bridge stdin unavailable")?;
    stdin.write_all(request.as_bytes()).await?;
    stdin.write_all(b"\n").await?;
    drop(stdin);

    let output = tokio::time::timeout(BRIDGE_SMOKE_TIMEOUT, child.wait_with_output())
        .await
        .with_context(|| {
            format!(
                "process bridge `{}` timed out after {}s",
                command.command,
                BRIDGE_SMOKE_TIMEOUT.as_secs()
            )
        })??;
    let stderr = String::from_utf8_lossy(&output.stderr);
    if !output.status.success() {
        anyhow::bail!(
            "process bridge `{}` exited with {}; stderr: {}",
            command.command,
            output.status,
            stderr.trim()
        );
    }
    parse_bridge_stdout(&String::from_utf8_lossy(&output.stdout))
}

pub(super) fn parse_process_bridge_handler(handler: &str) -> anyhow::Result<ProcessBridgeCommand> {
    let trimmed = handler.trim();
    if trimmed.is_empty() {
        anyhow::bail!("process-bridge handler is empty");
    }
    if trimmed.starts_with('{') {
        return Ok(serde_json::from_str::<ProcessBridgeCommand>(trimmed)?);
    }
    Ok(ProcessBridgeCommand::new(trimmed))
}

pub(super) fn ensure_describe_response(
    frames: &[BridgeEnvelope],
    name: &str,
) -> anyhow::Result<()> {
    if frames.iter().any(|frame| {
        matches!(
            frame,
            BridgeEnvelope::Response {
                response: BridgeResponse::Descriptors { .. },
                ..
            }
        )
    }) {
        return Ok(());
    }
    anyhow::bail!("process bridge `{name}` did not return descriptors for describe")
}

pub(super) fn tool_result_text(frames: &[BridgeEnvelope]) -> Option<String> {
    frames.iter().find_map(|frame| match frame {
        BridgeEnvelope::Response {
            response:
                BridgeResponse::ToolResult {
                    output,
                    is_error: false,
                    ..
                },
            ..
        } => Some(output.clone()),
        BridgeEnvelope::Response {
            response: BridgeResponse::Error(error),
            ..
        } => Some(format!("error {}: {}", error.code, error.message)),
        _ => None,
    })
}

pub(super) fn provider_text(frames: &[BridgeEnvelope]) -> Option<String> {
    let mut deltas = String::new();
    let mut done_text = None;
    for frame in frames {
        if let BridgeEnvelope::Response {
            response: BridgeResponse::ProviderEvent { event },
            ..
        } = frame
        {
            match event {
                BridgeProviderStreamEvent::TextDelta { delta, .. } => deltas.push_str(delta),
                BridgeProviderStreamEvent::TextDone { text, .. } => done_text = Some(text.clone()),
                BridgeProviderStreamEvent::Error { message } => return Some(message.clone()),
                _ => {}
            }
        }
    }
    done_text.or_else(|| {
        if deltas.is_empty() {
            None
        } else {
            Some(deltas)
        }
    })
}

fn parse_bridge_stdout(stdout: &str) -> anyhow::Result<Vec<BridgeEnvelope>> {
    let frames = stdout
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(serde_json::from_str::<BridgeEnvelope>)
        .collect::<Result<Vec<_>, _>>()?;
    if frames.is_empty() {
        anyhow::bail!("process bridge returned no response frames");
    }
    Ok(frames)
}
