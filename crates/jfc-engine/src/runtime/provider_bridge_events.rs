use jfc_plugin_sdk::{
    BridgeEnvelope, BridgeFallbackReason, BridgeProviderStreamEvent, BridgeResponse,
    BridgeStopReason,
};
use jfc_provider::{FallbackReason, FallbackTriggered, ModelId, StopReason, StreamEvent};

pub(crate) fn response_line_to_event(
    provider_name: &str,
    request_id: &str,
    line: &str,
) -> anyhow::Result<StreamEvent> {
    let frame = serde_json::from_str::<BridgeEnvelope>(line).map_err(|error| {
        anyhow::anyhow!("ProcessBridge provider `{provider_name}` returned invalid JSONL: {error}")
    })?;
    match frame {
        BridgeEnvelope::Response { id, response } => {
            if id != request_id {
                anyhow::bail!(
                    "ProcessBridge provider `{provider_name}` response id `{id}` did not match `{request_id}`"
                );
            }
            bridge_response_to_event(provider_name, response)
        }
        BridgeEnvelope::Request { .. } => {
            anyhow::bail!(
                "ProcessBridge provider `{provider_name}` returned a request frame, expected response"
            )
        }
    }
}

fn bridge_response_to_event(
    provider_name: &str,
    response: BridgeResponse,
) -> anyhow::Result<StreamEvent> {
    match response {
        BridgeResponse::ProviderEvent { event } => Ok(provider_event_to_stream_event(event)),
        BridgeResponse::Error(error) => anyhow::bail!(
            "ProcessBridge provider `{provider_name}` error `{}`: {}",
            error.code,
            error.message
        ),
        other => {
            anyhow::bail!(
                "ProcessBridge provider `{provider_name}` returned unexpected response: {other:?}"
            )
        }
    }
}

fn provider_event_to_stream_event(event: BridgeProviderStreamEvent) -> StreamEvent {
    match event {
        BridgeProviderStreamEvent::TextDelta { index, delta } => {
            StreamEvent::TextDelta { index, delta }
        }
        BridgeProviderStreamEvent::TextDone { index, text } => {
            StreamEvent::TextDone { index, text }
        }
        BridgeProviderStreamEvent::ThinkingDelta {
            index,
            delta,
            estimated_tokens,
        } => StreamEvent::ThinkingDelta {
            index,
            delta,
            estimated_tokens,
        },
        BridgeProviderStreamEvent::ThinkingTokens { index, delta } => {
            StreamEvent::ThinkingTokens { index, delta }
        }
        BridgeProviderStreamEvent::ThinkingDone {
            index,
            text,
            signature,
        } => StreamEvent::ThinkingDone {
            index,
            text,
            signature,
        },
        BridgeProviderStreamEvent::RedactedThinkingDone { index, data } => {
            StreamEvent::RedactedThinkingDone { index, data }
        }
        BridgeProviderStreamEvent::ToolDelta { index, delta } => {
            StreamEvent::ToolDelta { index, delta }
        }
        BridgeProviderStreamEvent::ToolDone {
            index,
            tool_name,
            tool_use_id,
            input_json,
            thought_signature,
        } => StreamEvent::ToolDone {
            index,
            tool_name,
            tool_use_id,
            input_json,
            thought_signature,
        },
        BridgeProviderStreamEvent::ServerToolResult {
            tool_use_id,
            tool_kind,
            content,
        } => StreamEvent::ServerToolResult {
            tool_use_id,
            tool_kind: jfc_provider::ServerToolResultKind::from_wire_type(&tool_kind),
            content,
        },
        BridgeProviderStreamEvent::Done { stop_reason } => StreamEvent::Done {
            stop_reason: bridge_stop_reason_to_provider(stop_reason),
        },
        BridgeProviderStreamEvent::Usage {
            input_tokens,
            output_tokens,
            thinking_tokens,
            cache_read_tokens,
            cache_write_tokens,
        } => StreamEvent::Usage {
            input_tokens,
            output_tokens,
            thinking_tokens,
            cache_read_tokens,
            cache_write_tokens,
        },
        BridgeProviderStreamEvent::ResponseMetadata {
            response_id,
            input_tokens,
        } => StreamEvent::ResponseMetadata {
            response_id,
            input_tokens,
        },
        BridgeProviderStreamEvent::Error { message } => StreamEvent::Error { message },
        BridgeProviderStreamEvent::Keepalive => StreamEvent::Keepalive,
        BridgeProviderStreamEvent::FallbackTriggered {
            original_model,
            fallback_model,
            reason,
        } => StreamEvent::FallbackTriggered(FallbackTriggered {
            original_model: ModelId::new(original_model),
            fallback_model: ModelId::new(fallback_model),
            reason: bridge_fallback_reason_to_provider(reason),
        }),
    }
}

fn bridge_stop_reason_to_provider(reason: BridgeStopReason) -> StopReason {
    match reason {
        BridgeStopReason::EndTurn => StopReason::EndTurn,
        BridgeStopReason::ToolUse => StopReason::ToolUse,
        BridgeStopReason::PauseTurn => StopReason::PauseTurn,
        BridgeStopReason::Refusal => StopReason::Refusal,
        BridgeStopReason::MaxTokens => StopReason::MaxTokens,
        BridgeStopReason::StopSequence => StopReason::StopSequence,
        BridgeStopReason::Other(reason) => StopReason::Other(reason),
    }
}

fn bridge_fallback_reason_to_provider(reason: BridgeFallbackReason) -> FallbackReason {
    match reason {
        BridgeFallbackReason::ModelNotFound => FallbackReason::ModelNotFound,
        BridgeFallbackReason::Overloaded => FallbackReason::Overloaded,
        BridgeFallbackReason::ModelRefusal => FallbackReason::ModelRefusal,
        BridgeFallbackReason::PermissionDenied => FallbackReason::PermissionDenied,
        BridgeFallbackReason::ServerError | BridgeFallbackReason::Other(_) => {
            FallbackReason::ServerError
        }
    }
}
