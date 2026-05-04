#![allow(dead_code)]

use eventsource_stream::Eventsource;
use futures::{StreamExt, TryStreamExt};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::provider::{
    EventStream, ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent, ToolDef,
};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SseEvent {
    MessageStart {
        message: MessageStart,
    },
    ContentBlockStart {
        index: usize,
        content_block: ContentBlock,
    },
    ContentBlockDelta {
        index: usize,
        delta: Delta,
    },
    ContentBlockStop {
        index: usize,
    },
    MessageDelta {
        delta: MessageDeltaData,
        #[serde(default)]
        usage: Option<MessageUsage>,
    },
    MessageStop,
    Ping,
    Error {
        error: ErrorBody,
    },
}

#[derive(Debug, Deserialize)]
pub struct MessageStart {
    #[allow(dead_code)]
    pub id: String,
    #[serde(default)]
    pub usage: Option<MessageUsage>,
}

#[derive(Debug, Deserialize)]
pub struct MessageUsage {
    #[serde(default)]
    pub input_tokens: Option<u32>,
    #[serde(default)]
    pub output_tokens: Option<u32>,
    #[serde(default)]
    pub cache_creation_input_tokens: Option<u32>,
    #[serde(default)]
    pub cache_read_input_tokens: Option<u32>,
}

impl MessageUsage {
    fn input_total(&self) -> u32 {
        self.input_tokens.unwrap_or_default()
            + self.cache_creation_input_tokens.unwrap_or_default()
            + self.cache_read_input_tokens.unwrap_or_default()
    }

    fn output_total(&self) -> u32 {
        self.output_tokens.unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[allow(dead_code)]
        input: Value,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Delta {
    TextDelta { text: String },
    ThinkingDelta { thinking: String },
    InputJsonDelta { partial_json: String },
    SignatureDelta { signature: String },
    CitationsDelta {},
    ConnectorTextDelta { connector_text: String },
}

#[derive(Debug, Deserialize)]
pub struct MessageDeltaData {
    pub stop_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ErrorBody {
    pub message: String,
}

pub enum BlockState {
    Text {
        accumulated: String,
    },
    Thinking {
        accumulated: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
}

pub fn parse_stop_reason(s: Option<&str>) -> StopReason {
    match s {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        Some(other) => StopReason::Other(other.to_owned()),
        None => StopReason::EndTurn,
    }
}

pub fn translate(
    event: SseEvent,
    blocks: &mut Vec<Option<BlockState>>,
    stop_reason: &mut Option<StopReason>,
) -> Option<StreamEvent> {
    match event {
        SseEvent::ContentBlockStart {
            index,
            content_block,
        } => {
            while blocks.len() <= index {
                blocks.push(None);
            }
            blocks[index] = Some(match content_block {
                ContentBlock::Text { .. } => BlockState::Text {
                    accumulated: String::new(),
                },
                ContentBlock::Thinking { .. } => BlockState::Thinking {
                    accumulated: String::new(),
                },
                ContentBlock::ToolUse { id, name, .. } => BlockState::ToolUse {
                    id,
                    name,
                    input: String::new(),
                },
            });
            None
        }
        SseEvent::ContentBlockDelta { index, delta } => match delta {
            Delta::TextDelta { text } => {
                if let Some(Some(BlockState::Text { accumulated })) = blocks.get_mut(index) {
                    accumulated.push_str(&text);
                }
                Some(StreamEvent::TextDelta { index, delta: text })
            }
            Delta::ThinkingDelta { thinking } => {
                if let Some(Some(BlockState::Thinking { accumulated })) = blocks.get_mut(index) {
                    accumulated.push_str(&thinking);
                }
                Some(StreamEvent::ThinkingDelta {
                    index,
                    delta: thinking,
                })
            }
            Delta::InputJsonDelta { partial_json } => {
                if let Some(Some(BlockState::ToolUse { input, .. })) = blocks.get_mut(index) {
                    input.push_str(&partial_json);
                }
                Some(StreamEvent::ToolDelta {
                    index,
                    delta: partial_json,
                })
            }
            Delta::SignatureDelta { .. }
            | Delta::CitationsDelta {}
            | Delta::ConnectorTextDelta { .. } => None,
        },
        SseEvent::ContentBlockStop { index } => {
            match blocks.get_mut(index).and_then(|b| b.take()) {
                Some(BlockState::Text { accumulated }) => Some(StreamEvent::TextDone {
                    index,
                    text: accumulated,
                }),
                Some(BlockState::Thinking { accumulated }) => Some(StreamEvent::ThinkingDone {
                    index,
                    text: accumulated,
                }),
                Some(BlockState::ToolUse { id, name, input }) => Some(StreamEvent::ToolDone {
                    index,
                    tool_name: name,
                    tool_use_id: id,
                    input_json: input,
                }),
                None => None,
            }
        }
        SseEvent::MessageDelta { delta, usage } => {
            *stop_reason = Some(parse_stop_reason(delta.stop_reason.as_deref()));
            usage.map(|usage| StreamEvent::Usage {
                input_tokens: usage.input_total(),
                output_tokens: usage.output_total(),
                cache_read_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
                cache_write_tokens: usage.cache_creation_input_tokens.unwrap_or_default(),
            })
        }
        SseEvent::MessageStop => Some(StreamEvent::Done {
            stop_reason: stop_reason.take().unwrap_or(StopReason::EndTurn),
        }),
        SseEvent::Error { error } => Some(StreamEvent::Error {
            message: error.message,
        }),
        SseEvent::MessageStart { message } => message.usage.map(|usage| StreamEvent::Usage {
            input_tokens: usage.input_total(),
            output_tokens: usage.output_total(),
            cache_read_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
            cache_write_tokens: usage.cache_creation_input_tokens.unwrap_or_default(),
        }),
        SseEvent::Ping => None,
    }
}

pub fn build_messages(messages: &[ProviderMessage]) -> Value {
    messages
        .iter()
        .map(|m| {
            let role = match m.role {
                ProviderRole::User => "user",
                ProviderRole::Assistant => "assistant",
            };
            let content: Vec<Value> = m
                .content
                .iter()
                .map(|c| match c {
                    ProviderContent::Text(t) => json!({ "type": "text", "text": t }),
                    ProviderContent::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => json!({
                        "type": "tool_result",
                        "tool_use_id": tool_use_id,
                        "content": content,
                        "is_error": is_error,
                    }),
                    ProviderContent::ToolUse { id, name, input } => json!({
                        "type": "tool_use",
                        "id": id,
                        "name": name,
                        "input": input,
                    }),
                })
                .collect();
            json!({ "role": role, "content": content })
        })
        .collect::<Vec<_>>()
        .into()
}

pub fn build_tools(tools: &[ToolDef]) -> Value {
    tools
        .iter()
        .map(|t| {
            json!({
                "name": t.name,
                "description": t.description,
                "input_schema": t.input_schema,
            })
        })
        .collect::<Vec<_>>()
        .into()
}

pub fn into_event_stream(resp: reqwest::Response) -> EventStream {
    let byte_stream = resp
        .bytes_stream()
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e));

    // Tracing parity with the OpenWebUI provider: dump raw SSE bytes at TRACE,
    // log every parsed event type at DEBUG, log finish_reason / errors at INFO.
    // Flip `RUST_LOG=jfc::provider::anthropic_sse=trace` to see raw chunks
    // when debugging upstream SSE weirdness.
    let event_stream = byte_stream
        .eventsource()
        .scan(
            (Vec::<Option<BlockState>>::new(), None::<StopReason>),
            |state, result| {
                let (blocks, stop_reason) = state;
                let out = result.ok().and_then(|ev| {
                    tracing::trace!(
                        target: "jfc::provider::anthropic_sse",
                        event = %ev.event,
                        data = %&ev.data[..ev.data.len().min(400)],
                        "sse raw"
                    );
                    if ev.event == "ping" || ev.data.is_empty() {
                        return None;
                    }
                    if ev.data == "[DONE]" {
                        tracing::debug!(target: "jfc::provider::anthropic_sse", "sse [DONE]");
                        return None;
                    }
                    match serde_json::from_str::<SseEvent>(&ev.data) {
                        Ok(parsed) => {
                            log_parsed_event(&parsed);
                            translate(parsed, blocks, stop_reason).map(Ok)
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "jfc::provider::anthropic_sse",
                                error = %e,
                                data = %&ev.data[..ev.data.len().min(200)],
                                "sse parse error"
                            );
                            Some(Err(anyhow::anyhow!("SSE parse error: {e}")))
                        }
                    }
                });
                futures::future::ready(Some(out))
            },
        )
        .filter_map(|x| futures::future::ready(x));

    Box::pin(event_stream)
}

/// Per-event tracing for the Anthropic SSE pipeline. Mirrors what the OWUI
/// provider logs (`chunk_finish` for stop signals, per-tool synthesis logs)
/// so the two paths read consistently in the log file.
fn log_parsed_event(event: &SseEvent) {
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
                ContentBlock::ToolUse { .. } => "tool_use",
            };
            if let ContentBlock::ToolUse { id, name, .. } = content_block {
                tracing::info!(
                    target: "jfc::provider::anthropic_sse",
                    index,
                    tool_name = %name,
                    tool_use_id = %id,
                    "content_block_start tool_use"
                );
            } else {
                tracing::debug!(
                    target: "jfc::provider::anthropic_sse",
                    index,
                    kind,
                    "content_block_start"
                );
            }
        }
        SseEvent::ContentBlockDelta { index, delta } => {
            let (kind, len) = match delta {
                Delta::TextDelta { text } => ("text", text.len()),
                Delta::ThinkingDelta { thinking } => ("thinking", thinking.len()),
                Delta::InputJsonDelta { partial_json } => ("input_json", partial_json.len()),
                Delta::SignatureDelta { signature } => ("signature", signature.len()),
                Delta::CitationsDelta {} => ("citations", 0),
                Delta::ConnectorTextDelta { connector_text } => {
                    ("connector_text", connector_text.len())
                }
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
        SseEvent::MessageDelta { delta, usage } => {
            tracing::info!(
                target: "jfc::provider::anthropic_sse",
                stop_reason = ?delta.stop_reason,
                input_tokens = usage.as_ref().map(MessageUsage::input_total),
                output_tokens = usage.as_ref().map(MessageUsage::output_total),
                "message_delta"
            );
        }
        SseEvent::MessageStop => {
            tracing::debug!(target: "jfc::provider::anthropic_sse", "message_stop");
        }
        SseEvent::Error { error } => {
            tracing::warn!(
                target: "jfc::provider::anthropic_sse",
                error = %error.message,
                "sse error event"
            );
        }
        SseEvent::Ping => {} // already filtered above by ev.event == "ping"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent, ToolDef,
    };

    fn make_user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(text.to_owned())],
        }
    }

    fn make_assistant_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(text.to_owned())],
        }
    }

    fn empty_state() -> (Vec<Option<BlockState>>, Option<StopReason>) {
        (Vec::new(), None)
    }

    #[test]
    fn parse_stop_reason_all_variants() {
        assert_eq!(parse_stop_reason(Some("end_turn")), StopReason::EndTurn);
        assert_eq!(parse_stop_reason(Some("tool_use")), StopReason::ToolUse);
        assert_eq!(parse_stop_reason(Some("max_tokens")), StopReason::MaxTokens);
        assert_eq!(
            parse_stop_reason(Some("stop_sequence")),
            StopReason::StopSequence
        );
        assert_eq!(
            parse_stop_reason(Some("refusal")),
            StopReason::Other("refusal".into())
        );
        assert_eq!(parse_stop_reason(None), StopReason::EndTurn);
    }

    #[test]
    fn translate_text_block_lifecycle() {
        let (mut blocks, mut sr) = empty_state();
        translate(
            SseEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::Text {
                    text: String::new(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        assert!(matches!(blocks[0], Some(BlockState::Text { .. })));

        let out = translate(
            SseEvent::ContentBlockDelta {
                index: 0,
                delta: Delta::TextDelta {
                    text: "chunk1".into(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        assert!(matches!(out, Some(StreamEvent::TextDelta { delta, .. }) if delta == "chunk1"));

        translate(
            SseEvent::ContentBlockDelta {
                index: 0,
                delta: Delta::TextDelta {
                    text: "chunk2".into(),
                },
            },
            &mut blocks,
            &mut sr,
        );

        let out = translate(
            SseEvent::ContentBlockStop { index: 0 },
            &mut blocks,
            &mut sr,
        );
        assert!(matches!(out, Some(StreamEvent::TextDone { text, .. }) if text == "chunk1chunk2"));
        assert!(blocks[0].is_none());
    }

    #[test]
    fn translate_thinking_delta_accumulates() {
        let (mut blocks, mut sr) = empty_state();
        translate(
            SseEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::Thinking {
                    thinking: String::new(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        let out = translate(
            SseEvent::ContentBlockDelta {
                index: 0,
                delta: Delta::ThinkingDelta {
                    thinking: "thought".into(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        assert!(
            matches!(out, Some(StreamEvent::ThinkingDelta { delta, .. }) if delta == "thought")
        );
    }

    #[test]
    fn translate_tool_use_lifecycle() {
        let (mut blocks, mut sr) = empty_state();
        translate(
            SseEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::ToolUse {
                    id: "tu_1".into(),
                    name: "bash".into(),
                    input: Value::Null,
                },
            },
            &mut blocks,
            &mut sr,
        );
        translate(
            SseEvent::ContentBlockDelta {
                index: 0,
                delta: Delta::InputJsonDelta {
                    partial_json: r#"{"cmd":"ls"}"#.into(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        let out = translate(
            SseEvent::ContentBlockStop { index: 0 },
            &mut blocks,
            &mut sr,
        );
        assert!(
            matches!(out, Some(StreamEvent::ToolDone { tool_name, tool_use_id, input_json, .. })
            if tool_name == "bash" && tool_use_id == "tu_1" && input_json == r#"{"cmd":"ls"}"#)
        );
    }

    #[test]
    fn translate_message_stop_with_reason() {
        let (mut blocks, mut sr) = empty_state();
        translate(
            SseEvent::MessageDelta {
                delta: MessageDeltaData {
                    stop_reason: Some("end_turn".into()),
                },
                usage: None,
            },
            &mut blocks,
            &mut sr,
        );
        let out = translate(SseEvent::MessageStop, &mut blocks, &mut sr);
        assert!(matches!(
            out,
            Some(StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            })
        ));
    }

    #[test]
    fn translate_message_stop_defaults_end_turn() {
        let (mut blocks, mut sr) = empty_state();
        let out = translate(SseEvent::MessageStop, &mut blocks, &mut sr);
        assert!(matches!(
            out,
            Some(StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            })
        ));
    }

    #[test]
    fn translate_error_event() {
        let (mut blocks, mut sr) = empty_state();
        let out = translate(
            SseEvent::Error {
                error: ErrorBody {
                    message: "overloaded".into(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        assert!(matches!(out, Some(StreamEvent::Error { message }) if message == "overloaded"));
    }

    #[test]
    fn translate_ping_and_message_start_emit_nothing() {
        let (mut blocks, mut sr) = empty_state();
        assert!(translate(SseEvent::Ping, &mut blocks, &mut sr).is_none());
        assert!(
            translate(
                SseEvent::MessageStart {
                    message: MessageStart {
                        id: "msg_1".into(),
                        usage: None,
                    },
                },
                &mut blocks,
                &mut sr,
            )
            .is_none()
        );
    }

    #[test]
    fn translate_block_stop_missing_index() {
        let (mut blocks, mut sr) = empty_state();
        assert!(
            translate(
                SseEvent::ContentBlockStop { index: 99 },
                &mut blocks,
                &mut sr
            )
            .is_none()
        );
    }

    #[test]
    fn translate_multi_block_indices_independent() {
        let (mut blocks, mut sr) = empty_state();
        translate(
            SseEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::Text {
                    text: String::new(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        translate(
            SseEvent::ContentBlockStart {
                index: 1,
                content_block: ContentBlock::Thinking {
                    thinking: String::new(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        translate(
            SseEvent::ContentBlockDelta {
                index: 0,
                delta: Delta::TextDelta { text: "a".into() },
            },
            &mut blocks,
            &mut sr,
        );
        translate(
            SseEvent::ContentBlockDelta {
                index: 1,
                delta: Delta::ThinkingDelta {
                    thinking: "t".into(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        let t0 = translate(
            SseEvent::ContentBlockStop { index: 0 },
            &mut blocks,
            &mut sr,
        );
        let t1 = translate(
            SseEvent::ContentBlockStop { index: 1 },
            &mut blocks,
            &mut sr,
        );
        assert!(matches!(t0, Some(StreamEvent::TextDone { text, .. }) if text == "a"));
        assert!(matches!(t1, Some(StreamEvent::ThinkingDone { text, .. }) if text == "t"));
    }

    #[test]
    fn signature_delta_parses_and_ignored() {
        let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"EgYbOHMuAi0"}}"#;
        let event: SseEvent = serde_json::from_str(json).expect("signature_delta must parse");
        let (mut blocks, mut sr) = empty_state();
        blocks.push(Some(BlockState::Thinking {
            accumulated: "thought".into(),
        }));
        assert!(translate(event, &mut blocks, &mut sr).is_none());
    }

    #[test]
    fn citations_delta_parses_and_ignored() {
        let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"citations_delta"}}"#;
        let event: SseEvent = serde_json::from_str(json).expect("citations_delta must parse");
        let (mut blocks, mut sr) = empty_state();
        blocks.push(Some(BlockState::Text {
            accumulated: String::new(),
        }));
        assert!(translate(event, &mut blocks, &mut sr).is_none());
    }

    #[test]
    fn connector_text_delta_parses_and_ignored() {
        let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"connector_text_delta","connector_text":"\n\n"}}"#;
        let event: SseEvent = serde_json::from_str(json).expect("connector_text_delta must parse");
        let (mut blocks, mut sr) = empty_state();
        blocks.push(Some(BlockState::Text {
            accumulated: String::new(),
        }));
        assert!(translate(event, &mut blocks, &mut sr).is_none());
    }

    #[test]
    fn message_delta_usage_emits_usage_event() {
        let json = r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":42}}"#;
        let event: SseEvent = serde_json::from_str(json).expect("message_delta usage must parse");
        let (mut blocks, mut sr) = empty_state();

        assert!(matches!(
            translate(event, &mut blocks, &mut sr),
            Some(StreamEvent::Usage {
                input_tokens: 0,
                output_tokens: 42,
                ..
            })
        ));
        assert_eq!(sr, Some(StopReason::EndTurn));
    }

    #[test]
    fn message_start_usage_includes_cache_tokens() {
        let json = r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":10,"cache_creation_input_tokens":3,"cache_read_input_tokens":7}}}"#;
        let event: SseEvent = serde_json::from_str(json).expect("message_start usage must parse");
        let (mut blocks, mut sr) = empty_state();

        assert!(matches!(
            translate(event, &mut blocks, &mut sr),
            Some(StreamEvent::Usage {
                input_tokens: 20,
                output_tokens: 0,
                ..
            })
        ));
    }

    #[test]
    fn unknown_delta_type_fails_to_parse() {
        let json = r#"{"type":"content_block_delta","index":0,"delta":{"type":"totally_new_delta","data":"x"}}"#;
        assert!(serde_json::from_str::<SseEvent>(json).is_err());
    }

    #[test]
    fn build_messages_roundtrip() {
        let msgs = vec![
            make_user_msg("q1"),
            make_assistant_msg("a1"),
            make_user_msg("q2"),
        ];
        let v = build_messages(&msgs);
        assert_eq!(v[0]["role"], "user");
        assert_eq!(v[0]["content"][0]["text"], "q1");
        assert_eq!(v[1]["role"], "assistant");
        assert_eq!(v[2]["role"], "user");
    }

    #[test]
    fn build_messages_empty() {
        let v = build_messages(&[]);
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[test]
    fn build_messages_tool_result_shape() {
        let msg = ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: "tu_1".into(),
                content: "output".into(),
                is_error: false,
            }],
        };
        let v = build_messages(&[msg]);
        let block = &v[0]["content"][0];
        assert_eq!(block["type"], "tool_result");
        assert_eq!(block["tool_use_id"], "tu_1");
        assert_eq!(block["is_error"], false);
    }

    #[test]
    fn build_messages_tool_use_shape() {
        let msg = ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: "tu_2".into(),
                name: "read_file".into(),
                input: serde_json::json!({"path": "/tmp/x"}),
            }],
        };
        let v = build_messages(&[msg]);
        let block = &v[0]["content"][0];
        assert_eq!(block["type"], "tool_use");
        assert_eq!(block["id"], "tu_2");
        assert_eq!(block["name"], "read_file");
    }

    #[test]
    fn build_tools_shape() {
        let tools = vec![ToolDef {
            name: "bash".into(),
            description: "Execute bash".into(),
            input_schema: serde_json::json!({"type": "object"}),
        }];
        let v = build_tools(&tools);
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["name"], "bash");
    }

    #[test]
    fn build_tools_empty() {
        let v = build_tools(&[]);
        assert_eq!(v.as_array().unwrap().len(), 0);
    }

    #[test]
    fn build_tools_order_preserved() {
        let tools: Vec<ToolDef> = ["alpha", "beta", "gamma"]
            .iter()
            .map(|n| ToolDef {
                name: n.to_string(),
                description: n.to_string(),
                input_schema: serde_json::json!({}),
            })
            .collect();
        let v = build_tools(&tools);
        let arr = v.as_array().unwrap();
        assert_eq!(arr[0]["name"], "alpha");
        assert_eq!(arr[1]["name"], "beta");
        assert_eq!(arr[2]["name"], "gamma");
    }
}
