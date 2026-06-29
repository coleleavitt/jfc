use std::error::Error;
use std::io::{self, BufRead, Write};

use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeErrorDto, BridgeRequest, BridgeResponse, DescriptorVisibility, PluginId,
    ToolDescriptor, ToolExecutorKind,
};

fn main() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let frame = read_frame(&mut lines)?;
    let id = frame.id().to_owned();
    let response = match frame {
        BridgeEnvelope::Request {
            request: BridgeRequest::Describe,
            ..
        } => BridgeResponse::Descriptors {
            descriptors: serde_json::json!({ "tools": [tool_descriptor()] }),
        },
        BridgeEnvelope::Request {
            request: BridgeRequest::ToolCall { tool, input, .. },
            ..
        } if tool == "external_echo" => BridgeResponse::ToolResult {
            output: format!("external echo: {}", message_from_input(&input)),
            is_error: false,
            payload: Some(serde_json::json!({ "echoed": message_from_input(&input) })),
        },
        BridgeEnvelope::Request {
            request: BridgeRequest::ToolCall { tool, .. },
            ..
        } => BridgeResponse::Error(BridgeErrorDto::new(
            "unknown_tool",
            format!("process bridge tool does not handle `{tool}`"),
        )),
        _ => BridgeResponse::Error(BridgeErrorDto::new(
            "unsupported_request",
            "expected tool_call request",
        )),
    };
    write_frame(&BridgeEnvelope::response(id, response))?;
    Ok(())
}

fn tool_descriptor() -> ToolDescriptor {
    ToolDescriptor::new(
        PluginId::new("example-process-tool-plugin"),
        "external_echo",
        "External Echo",
        serde_json::json!({
            "type": "object",
            "properties": {
                "message": {
                    "type": "string",
                    "description": "Message to echo."
                }
            },
            "required": ["message"],
            "additionalProperties": false
        }),
    )
    .with_executor(ToolExecutorKind::ProcessBridge, "")
    .with_visibility(DescriptorVisibility::ModelVisible)
}

fn message_from_input(input: &serde_json::Value) -> &str {
    input
        .get("message")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("hello from process bridge")
}

fn read_frame<I>(lines: &mut I) -> Result<BridgeEnvelope, Box<dyn Error>>
where
    I: Iterator<Item = io::Result<String>>,
{
    let Some(line) = lines.next() else {
        return Err("bridge stdin closed".into());
    };
    Ok(serde_json::from_str(&line?)?)
}

fn write_frame(frame: &BridgeEnvelope) -> Result<(), Box<dyn Error>> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, frame)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}
