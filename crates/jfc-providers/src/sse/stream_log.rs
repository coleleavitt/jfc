use super::{ContentBlock, Delta, MessageUsage, SseEvent};

/// Per-event tracing for the Anthropic SSE pipeline. Mirrors what the OWUI
/// provider logs (`chunk_finish` for stop signals, per-tool synthesis logs)
/// so the two paths read consistently in the log file.
pub(crate) fn log_parsed_event(event: &SseEvent) {
    match event {
        SseEvent::MessageStart { message } => {
            tracing::debug!(
                target: "jfc::provider::anthropic_sse",
                id = %message.id,
                "message_start"
            );
        }
        SseEvent::ContentBlockStart {
            index,
            content_block,
        } => {
            let kind = match content_block {
                ContentBlock::Text { .. } => "text",
                ContentBlock::Thinking { .. } => "thinking",
                ContentBlock::RedactedThinking { .. } => "redacted_thinking",
                ContentBlock::ToolUse { .. } => "tool_use",
                ContentBlock::ServerToolUse { .. } => "server_tool_use",
                ContentBlock::ServerToolResult { tool_kind, .. } => tool_kind.wire_type(),
                ContentBlock::Unknown { kind, .. } => kind.as_str(),
            };
            match content_block {
                ContentBlock::ToolUse { id, name, .. } => {
                    tracing::info!(
                        target: "jfc::provider::anthropic_sse",
                        index,
                        tool_name = %name,
                        tool_use_id = %id,
                        "content_block_start tool_use"
                    );
                }
                ContentBlock::ServerToolUse { id, name, .. } => {
                    tracing::info!(
                        target: "jfc::provider::anthropic_sse",
                        index,
                        tool_name = %name,
                        tool_use_id = %id,
                        "content_block_start server_tool_use"
                    );
                }
                ContentBlock::ServerToolResult { tool_use_id, .. } => {
                    tracing::info!(
                        target: "jfc::provider::anthropic_sse",
                        index,
                        kind,
                        tool_use_id = %tool_use_id,
                        "content_block_start server_tool_result"
                    );
                }
                ContentBlock::Unknown { raw, .. } => {
                    tracing::warn!(
                        target: "jfc::provider::anthropic_sse",
                        index,
                        kind,
                        raw_preview = %raw.to_string().chars().take(200).collect::<String>(),
                        "content_block_start unknown"
                    );
                }
                _ => {
                    tracing::debug!(
                        target: "jfc::provider::anthropic_sse",
                        index,
                        kind,
                        "content_block_start"
                    );
                }
            }
        }
        SseEvent::ContentBlockDelta { index, delta } => {
            let (kind, len) = match delta {
                Delta::TextDelta { text } => ("text", text.len()),
                Delta::ThinkingDelta { thinking, .. } => ("thinking", thinking.len()),
                Delta::InputJsonDelta { partial_json } => ("input_json", partial_json.len()),
                Delta::SignatureDelta { signature } => ("signature", signature.len()),
                Delta::CitationsDelta {} => ("citations", 0),
                Delta::ConnectorTextDelta { connector_text } => {
                    ("connector_text", connector_text.len())
                }
                Delta::CompactionContentBlockDelta { content } => ("compaction", content.len()),
                Delta::Unknown { kind, raw } => (kind.as_str(), raw.to_string().len()),
            };
            tracing::trace!(
                target: "jfc::provider::anthropic_sse",
                index,
                kind,
                len,
                "content_block_delta"
            );
        }
        SseEvent::ContentBlockStop { index } => {
            tracing::debug!(
                target: "jfc::provider::anthropic_sse",
                index,
                "content_block_stop"
            );
        }
        SseEvent::MessageDelta {
            delta,
            usage,
            context_management,
        } => {
            tracing::info!(
                target: "jfc::provider::anthropic_sse",
                stop_reason = ?delta.stop_reason,
                input_tokens = usage.as_ref().map(MessageUsage::input_tokens),
                output_tokens = usage.as_ref().map(MessageUsage::output_total),
                has_context_management = context_management.is_some(),
                "message_delta"
            );
        }
        SseEvent::MessageStop => {
            tracing::debug!(target: "jfc::provider::anthropic_sse", "message_stop");
        }
        SseEvent::Error { error } => {
            tracing::warn!(
                target: "jfc::provider::anthropic_sse",
                kind = ?error.kind,
                error = %error.message,
                "sse error event"
            );
        }
        SseEvent::Ping => {} // already filtered above by ev.event == "ping"
        SseEvent::Unknown { kind, raw } => {
            tracing::warn!(
                target: "jfc::provider::anthropic_sse",
                kind = %kind,
                raw_preview = %raw.to_string().chars().take(200).collect::<String>(),
                "unknown SSE event"
            );
        }
    }
}
