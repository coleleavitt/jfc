use std::error::Error;
use std::io::{self, BufRead, Write};

use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeErrorDto, BridgeProviderContent, BridgeProviderMessage,
    BridgeProviderRole, BridgeProviderStreamEvent, BridgeRequest, BridgeResponse, BridgeStopReason,
    DescriptorVisibility, PluginId, ProviderDescriptor, ProviderExecutorKind,
};

fn main() -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let frame = read_frame(&mut lines)?;
    let id = frame.id().to_owned();
    match frame {
        BridgeEnvelope::Request {
            request: BridgeRequest::Describe,
            ..
        } => write_frame(&BridgeEnvelope::response(
            id,
            BridgeResponse::Descriptors {
                descriptors: serde_json::json!({ "providers": [provider_descriptor()] }),
            },
        ))?,
        BridgeEnvelope::Request {
            request:
                BridgeRequest::ProviderStream {
                    provider,
                    messages,
                    options,
                },
            ..
        } if provider == "external-demo" => {
            let text = demo_response(&options.model, &messages);
            write_provider_event(
                &id,
                BridgeProviderStreamEvent::TextDelta {
                    index: 0,
                    delta: text.clone(),
                },
            )?;
            write_provider_event(
                &id,
                BridgeProviderStreamEvent::TextDone {
                    index: 0,
                    text: text.clone(),
                },
            )?;
            write_provider_event(
                &id,
                BridgeProviderStreamEvent::Usage {
                    input_tokens: 1,
                    output_tokens: u32::try_from(text.split_whitespace().count())
                        .unwrap_or(u32::MAX),
                    thinking_tokens: None,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                },
            )?;
            write_provider_event(
                &id,
                BridgeProviderStreamEvent::Done {
                    stop_reason: BridgeStopReason::EndTurn,
                },
            )?;
        }
        BridgeEnvelope::Request {
            request: BridgeRequest::ProviderStream { provider, .. },
            ..
        } => write_frame(&BridgeEnvelope::response(
            id,
            BridgeResponse::Error(BridgeErrorDto::new(
                "unknown_provider",
                format!("process bridge provider does not handle `{provider}`"),
            )),
        ))?,
        _ => write_frame(&BridgeEnvelope::response(
            id,
            BridgeResponse::Error(BridgeErrorDto::new(
                "unsupported_request",
                "expected provider_stream request",
            )),
        ))?,
    }
    Ok(())
}

fn provider_descriptor() -> ProviderDescriptor {
    ProviderDescriptor::new(
        PluginId::new("example-process-provider-plugin"),
        "external-demo",
    )
    .with_model_info(
        "external-demo-chat",
        "External Demo Chat",
        Some(8192),
        Some(1024),
    )
    .with_executor(ProviderExecutorKind::ProcessBridge, "")
    .with_visibility(DescriptorVisibility::HostVisible)
}

fn demo_response(model: &str, messages: &[BridgeProviderMessage]) -> String {
    let prompt = last_user_text(messages).unwrap_or("no user text");
    format!("{model} received: {prompt}")
}

fn last_user_text(messages: &[BridgeProviderMessage]) -> Option<&str> {
    messages
        .iter()
        .rev()
        .find(|message| message.role == BridgeProviderRole::User)
        .and_then(|message| {
            message.content.iter().find_map(|content| match content {
                BridgeProviderContent::Text { text } => Some(text.as_str()),
                BridgeProviderContent::Thinking { .. }
                | BridgeProviderContent::ToolResult { .. }
                | BridgeProviderContent::ToolUse { .. }
                | BridgeProviderContent::ServerToolUse { .. }
                | BridgeProviderContent::ServerToolResult { .. }
                | BridgeProviderContent::Attachment { .. }
                | BridgeProviderContent::RedactedThinking { .. } => None,
            })
        })
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

fn write_provider_event(id: &str, event: BridgeProviderStreamEvent) -> Result<(), Box<dyn Error>> {
    write_frame(&BridgeEnvelope::response(
        id,
        BridgeResponse::ProviderEvent { event },
    ))
}

fn write_frame(frame: &BridgeEnvelope) -> Result<(), Box<dyn Error>> {
    let mut stdout = io::stdout().lock();
    serde_json::to_writer(&mut stdout, frame)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}
