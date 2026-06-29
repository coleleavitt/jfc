use std::io::Write;
use std::process::{Command, Stdio};

use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeRequest, BridgeResponse, PluginId, ProcessBridgeCommand, ToolDescriptor,
    ToolExecutorKind,
};
use serde::Deserialize;

use crate::PluginHostError;

pub(crate) fn describe_tool_descriptors(
    plugin_id: &PluginId,
    command: &ProcessBridgeCommand,
) -> Result<Vec<ToolDescriptor>, PluginHostError> {
    let request = BridgeEnvelope::request("describe-tools", BridgeRequest::Describe);
    let request = serde_json::to_string(&request).map_err(|error| {
        PluginHostError::plugin(format!("bridge describe encode failed: {error}"))
    })?;
    let mut child = Command::new(&command.command)
        .args(&command.args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            PluginHostError::plugin(format!(
                "bridge describe failed to start `{}`: {error}",
                command.command
            ))
        })?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(PluginHostError::plugin("bridge describe stdin unavailable"));
    };
    stdin
        .write_all(request.as_bytes())
        .and_then(|()| stdin.write_all(b"\n"))
        .map_err(|error| {
            PluginHostError::plugin(format!("bridge describe write failed: {error}"))
        })?;
    drop(stdin);

    let output = child.wait_with_output().map_err(|error| {
        PluginHostError::plugin(format!("bridge describe wait failed: {error}"))
    })?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let Some(line) = stdout.lines().find(|line| !line.trim().is_empty()) else {
        return Err(PluginHostError::plugin(format!(
            "bridge describe returned no response{}",
            stderr_suffix(&String::from_utf8_lossy(&output.stderr))
        )));
    };
    let frame = serde_json::from_str::<BridgeEnvelope>(line).map_err(|error| {
        PluginHostError::plugin(format!("bridge describe JSON failed: {error}"))
    })?;
    let BridgeEnvelope::Response { response, .. } = frame else {
        return Err(PluginHostError::plugin(
            "bridge describe returned request frame",
        ));
    };
    let BridgeResponse::Descriptors { descriptors } = response else {
        return Err(PluginHostError::plugin(
            "bridge describe returned non-descriptor response",
        ));
    };
    parse_tool_descriptors(plugin_id, command, descriptors)
}

fn parse_tool_descriptors(
    plugin_id: &PluginId,
    command: &ProcessBridgeCommand,
    descriptors: serde_json::Value,
) -> Result<Vec<ToolDescriptor>, PluginHostError> {
    let parsed = if descriptors.is_array() {
        serde_json::from_value::<Vec<ToolDescriptor>>(descriptors)
    } else {
        serde_json::from_value::<DescriptorPayload>(descriptors).map(|payload| payload.tools)
    }
    .map_err(|error| PluginHostError::plugin(format!("bridge descriptors invalid: {error}")))?;
    let handler = serde_json::to_string(command).map_err(|error| {
        PluginHostError::plugin(format!("bridge handler encode failed: {error}"))
    })?;
    Ok(parsed
        .into_iter()
        .map(|descriptor| normalize_descriptor(plugin_id, &handler, descriptor))
        .collect())
}

fn normalize_descriptor(
    plugin_id: &PluginId,
    bridge_handler: &str,
    mut descriptor: ToolDescriptor,
) -> ToolDescriptor {
    descriptor.plugin_id = plugin_id.clone();
    if descriptor.executor.kind == ToolExecutorKind::ProcessBridge
        && descriptor.executor.handler.trim().is_empty()
    {
        descriptor.executor.handler = bridge_handler.to_owned();
    }
    descriptor
}

fn stderr_suffix(stderr: &str) -> String {
    let stderr = stderr.trim();
    if stderr.is_empty() {
        String::new()
    } else {
        format!("; stderr: {stderr}")
    }
}

#[derive(Debug, Deserialize)]
struct DescriptorPayload {
    #[serde(default)]
    tools: Vec<ToolDescriptor>,
}
