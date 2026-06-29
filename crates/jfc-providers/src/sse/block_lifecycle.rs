use jfc_provider::StreamEvent;

use super::{
    BlockState, ContentBlock, Delta, append_input_delta, estimate_signature_thinking_tokens,
    estimate_thinking_text_tokens, initial_input_json,
};

pub(crate) fn start_content_block(
    index: usize,
    content_block: ContentBlock,
    blocks: &mut Vec<Option<BlockState>>,
) -> Option<StreamEvent> {
    while blocks.len() <= index {
        blocks.push(None);
    }
    blocks[index] = Some(match content_block {
        ContentBlock::Text { .. } => BlockState::Text {
            accumulated: String::new(),
        },
        ContentBlock::Thinking { .. } => BlockState::Thinking {
            accumulated: String::new(),
            estimated_tokens: 0,
            signature: None,
        },
        ContentBlock::RedactedThinking { data } => BlockState::RedactedThinking { data },
        ContentBlock::ToolUse { id, name, input } => BlockState::ToolUse {
            id,
            name,
            input: initial_input_json(input),
        },
        ContentBlock::ServerToolUse { id, name, input } => {
            // Server-side tools may send full input in the start
            // block, or stream it via input_json_delta when the
            // fine-grained tool streaming beta is active. Treat `{}` as
            // "not started yet" so later deltas produce valid JSON.
            let input_str = initial_input_json(input);
            BlockState::ServerToolUse {
                id,
                name,
                input: input_str,
            }
        }
        ContentBlock::ServerToolResult {
            tool_use_id,
            tool_kind,
            content,
        } => BlockState::ServerToolResult {
            tool_use_id,
            tool_kind,
            content,
        },
        ContentBlock::Unknown { kind, raw } => {
            tracing::warn!(
                target: "jfc::provider::anthropic_sse",
                kind = %kind,
                raw_preview = %raw.to_string().chars().take(200).collect::<String>(),
                "unknown content_block type ignored"
            );
            BlockState::Ignored { kind }
        }
    });
    None
}

pub(crate) fn apply_content_delta(
    index: usize,
    delta: Delta,
    blocks: &mut [Option<BlockState>],
) -> Option<StreamEvent> {
    match delta {
        Delta::TextDelta { text } => {
            if let Some(Some(BlockState::Text { accumulated })) = blocks.get_mut(index) {
                accumulated.push_str(&text);
            }
            Some(StreamEvent::TextDelta { index, delta: text })
        }
        Delta::ThinkingDelta {
            thinking,
            estimated_tokens,
        } => {
            let token_delta =
                estimated_tokens.unwrap_or_else(|| estimate_thinking_text_tokens(&thinking));
            if let Some(Some(BlockState::Thinking {
                accumulated,
                estimated_tokens,
                ..
            })) = blocks.get_mut(index)
            {
                accumulated.push_str(&thinking);
                *estimated_tokens = estimated_tokens.saturating_add(token_delta);
            }
            // One-shot visibility into whether the server actually honors
            // the thinking-token-count beta: log the first delta that
            // carries an estimate. If this never fires on a thinking turn,
            // the beta isn't reaching the server (header gate) rather than
            // a display bug.
            if estimated_tokens.is_some() {
                tracing::trace!(
                    target: "jfc::provider::anthropic_sse",
                    index,
                    estimated_tokens,
                    delta_len = thinking.len(),
                    "thinking_delta carried estimated_tokens"
                );
            }
            Some(StreamEvent::ThinkingDelta {
                index,
                delta: thinking,
                estimated_tokens: (token_delta > 0).then_some(token_delta),
            })
        }
        Delta::InputJsonDelta { partial_json } => {
            if let Some(Some(
                BlockState::ToolUse { input, .. } | BlockState::ServerToolUse { input, .. },
            )) = blocks.get_mut(index)
            {
                append_input_delta(input, &partial_json);
            }
            Some(StreamEvent::ToolDelta {
                index,
                delta: partial_json,
            })
        }
        Delta::SignatureDelta { signature } => {
            let Some(Some(BlockState::Thinking {
                estimated_tokens,
                signature: slot,
                ..
            })) = blocks.get_mut(index)
            else {
                return None;
            };
            *slot = Some(signature.clone());
            let signature_total = estimate_signature_thinking_tokens(&signature);
            if signature_total <= *estimated_tokens {
                return None;
            }
            let delta = signature_total - *estimated_tokens;
            *estimated_tokens = signature_total;
            Some(StreamEvent::ThinkingTokens { index, delta })
        }
        Delta::CitationsDelta {}
        | Delta::ConnectorTextDelta { .. }
        | Delta::CompactionContentBlockDelta { .. } => None,
        Delta::Unknown { kind, raw } => {
            tracing::warn!(
                target: "jfc::provider::anthropic_sse",
                index,
                kind = %kind,
                raw_preview = %raw.to_string().chars().take(200).collect::<String>(),
                "unknown content_block_delta type ignored"
            );
            None
        }
    }
}

pub(crate) fn stop_content_block(
    index: usize,
    blocks: &mut [Option<BlockState>],
) -> Option<StreamEvent> {
    match blocks.get_mut(index).and_then(|b| b.take()) {
        Some(BlockState::Text { accumulated }) => Some(StreamEvent::TextDone {
            index,
            text: accumulated,
        }),
        Some(BlockState::Thinking {
            accumulated,
            signature,
            ..
        }) => Some(StreamEvent::ThinkingDone {
            index,
            text: accumulated,
            signature,
        }),
        Some(BlockState::RedactedThinking { data }) => {
            Some(StreamEvent::RedactedThinkingDone { index, data })
        }
        Some(BlockState::ToolUse { id, name, input }) => Some(StreamEvent::ToolDone {
            index,
            tool_name: name,
            tool_use_id: id,
            input_json: input,
            thought_signature: None,
        }),
        // Server-side tools emit a prefixed tool name so stream.rs
        // can recognize them and skip local dispatch.
        Some(BlockState::ServerToolUse { id, name, input }) => {
            tracing::info!(
                target: "jfc::provider::anthropic_sse",
                index,
                tool_name = %name,
                tool_use_id = %id,
                "server_tool_use block complete"
            );
            Some(StreamEvent::ToolDone {
                index,
                tool_name: format!("server_tool_use:{name}"),
                tool_use_id: id,
                input_json: input,
                thought_signature: None,
            })
        }
        // Server-side tool result block (e.g. web_search). The
        // content is captured intact so the runtime can attach
        // it to the streaming assistant message for byte-faithful
        // re-emission on pause_turn resume. See cli.js v142:394261.
        Some(BlockState::ServerToolResult {
            tool_use_id,
            tool_kind,
            content,
        }) => {
            tracing::info!(
                target: "jfc::provider::anthropic_sse",
                index,
                wire_type = tool_kind.wire_type(),
                tool_use_id = %tool_use_id,
                content_preview = %content
                    .to_string()
                    .chars()
                    .take(200)
                    .collect::<String>(),
                "server_tool_result block complete"
            );
            Some(StreamEvent::ServerToolResult {
                tool_use_id,
                tool_kind,
                content,
            })
        }
        Some(BlockState::Ignored { kind }) => {
            tracing::debug!(
                target: "jfc::provider::anthropic_sse",
                index,
                kind = %kind,
                "ignored content block stopped"
            );
            None
        }
        None => None,
    }
}
