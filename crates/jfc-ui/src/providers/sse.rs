#![allow(dead_code)]

use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use jfc_provider::{
    EventStream, ProviderContent, ProviderMessage, ProviderRole, ServerToolResultKind, StopReason,
    StreamEvent, ToolDef,
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
        /// Present when Anthropic server-side context management is active.
        #[serde(default)]
        context_management: Option<ContextManagement>,
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
    fn input_tokens(&self) -> u32 {
        self.input_tokens.unwrap_or_default()
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
    /// Server-redacted thinking block — opaque base64 blob, no deltas.
    /// Must be round-tripped verbatim in subsequent requests.
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[allow(dead_code)]
        input: Value,
    },
    /// Anthropic server-side tool invocation (e.g. web_search, code_execution).
    /// These are executed server-side; jfc renders them but does not dispatch
    /// them locally. Shape mirrors `tool_use` but uses `server_tool_use` type.
    ServerToolUse {
        id: String,
        name: String,
        input: Value,
    },
    /// Server-side tool result for `web_search`. Wire shape per cli.js
    /// v142:394261:
    ///
    ///   { "type": "web_search_tool_result",
    ///     "tool_use_id": "srvtoolu_...",
    ///     "content": [ { "title": "...", "url": "..." }, ... ]   // OR
    ///                  { "error_code": "..." } }
    ///
    /// We keep `content` as `Value` so the error variant and any
    /// future field additions round-trip without parse loss.
    WebSearchToolResult {
        tool_use_id: String,
        content: Value,
    },
    /// Server-side tool result for `code_execution`. Wire type
    /// `code_execution_tool_result` per cli.js v142:246154.
    CodeExecutionToolResult {
        tool_use_id: String,
        content: Value,
    },
    /// Server-side tool result for `web_fetch`. Wire type
    /// `web_fetch_tool_result` per cli.js v142:246159.
    WebFetchToolResult {
        tool_use_id: String,
        content: Value,
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

/// Optional server-side context management metadata that Anthropic may attach
/// to a `message_delta` event when it is managing the context window on behalf
/// of the caller. The shape is deliberately left open (`Value`) so that new
/// fields (e.g. `compacted`, `removed_tokens`) don't cause parse failures.
#[derive(Debug, Deserialize)]
pub struct ContextManagement {
    /// True when Anthropic has already compacted earlier turns on the server.
    #[serde(default)]
    pub compacted: bool,
    /// Number of tokens removed by server-side compaction, if reported.
    #[serde(default)]
    pub removed_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct ErrorBody {
    #[serde(default, rename = "type")]
    pub kind: Option<String>,
    pub message: String,
}

pub enum BlockState {
    Text {
        accumulated: String,
    },
    Thinking {
        accumulated: String,
    },
    /// Opaque redacted thinking — no deltas, complete at start.
    /// Must be round-tripped in subsequent requests verbatim.
    RedactedThinking {
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: String,
    },
    /// Server-side tool invocation block (web_search, code_execution, etc.).
    /// Input is pre-populated from the start block and emits a prefixed
    /// ToolDone name so the rendering layer can distinguish server tools
    /// from locally-dispatched ones.
    ServerToolUse {
        id: String,
        name: String,
        input: String,
    },
    /// Server-side tool result block. Anthropic emits the entire
    /// content blob in the start event (cli.js v142:548307 routes the
    /// raw block straight into the result accumulator with no
    /// `input_json_delta` continuation), so we just hold the parsed
    /// JSON until `content_block_stop` releases it as a
    /// `StreamEvent::ServerToolResult`.
    ServerToolResult {
        tool_use_id: String,
        tool_kind: ServerToolResultKind,
        content: Value,
    },
}

pub fn parse_stop_reason(s: Option<&str>) -> StopReason {
    let result = match s {
        Some("end_turn") => StopReason::EndTurn,
        Some("tool_use") => StopReason::ToolUse,
        // Server-side sampling loop hit its iteration cap. The runtime
        // must re-send the conversation without injecting a synthetic
        // user message; the server resumes the loop where it left off.
        // See StopReason::PauseTurn docs and cli.js v142:622686.
        Some("pause_turn") => StopReason::PauseTurn,
        Some("max_tokens") => StopReason::MaxTokens,
        Some("stop_sequence") => StopReason::StopSequence,
        Some(other) => {
            // Unknown stop_reason string. Surface loudly — every
            // historical "stream silently ends" bug has eventually
            // traced back to a new server stop_reason being bucketed
            // into Other(...) and falling through event_loop's
            // dispatch ladder. The warn gives us a one-grep way to
            // catch the next variant (e.g. "refusal", "container_*")
            // before users notice.
            tracing::warn!(
                target: "jfc::provider::sse",
                stop_reason = other,
                "parse_stop_reason: unknown stop_reason string — bucketing into Other(...) \
                 (event_loop will fall into the 'model said its piece' branch); \
                 check cli.js v142 for a new variant we need to map"
            );
            StopReason::Other(other.to_owned())
        }
        None => {
            // Missing stop_reason field. Anthropic sometimes omits it
            // on truncated streams or context_hint short-circuits. The
            // EndTurn default is most-conservative for back-compat
            // (closes the streaming slot cleanly) but the silent fall-
            // through is exactly the class of bug that hid pause_turn
            // for months. Warn loudly so future occurrences are
            // diagnosable from the trace log alone.
            tracing::warn!(
                target: "jfc::provider::sse",
                "parse_stop_reason: missing stop_reason field — defaulting to EndTurn \
                 (this is back-compat; if you see this paired with a stalled stream, \
                 the upstream omitted a real stop_reason we should be handling)"
            );
            StopReason::EndTurn
        }
    };
    tracing::trace!(
        target: "jfc::provider::sse",
        input = ?s,
        result = ?result,
        "parse_stop_reason"
    );
    result
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
                ContentBlock::RedactedThinking { data } => {
                    BlockState::RedactedThinking { data }
                }
                ContentBlock::ToolUse { id, name, .. } => BlockState::ToolUse {
                    id,
                    name,
                    input: String::new(),
                },
                ContentBlock::ServerToolUse { id, name, input } => {
                    // Server-side tools send their full input in the start
                    // block rather than streaming it via InputJsonDelta. We
                    // pre-populate the input string so it's available at stop.
                    let input_str = if input.is_null() {
                        String::new()
                    } else {
                        input.to_string()
                    };
                    BlockState::ServerToolUse {
                        id,
                        name,
                        input: input_str,
                    }
                }
                ContentBlock::WebSearchToolResult {
                    tool_use_id,
                    content,
                } => BlockState::ServerToolResult {
                    tool_use_id,
                    tool_kind: ServerToolResultKind::WebSearch,
                    content,
                },
                ContentBlock::CodeExecutionToolResult {
                    tool_use_id,
                    content,
                } => BlockState::ServerToolResult {
                    tool_use_id,
                    tool_kind: ServerToolResultKind::CodeExecution,
                    content,
                },
                ContentBlock::WebFetchToolResult {
                    tool_use_id,
                    content,
                } => BlockState::ServerToolResult {
                    tool_use_id,
                    tool_kind: ServerToolResultKind::WebFetch,
                    content,
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
                Some(BlockState::RedactedThinking { data }) => {
                    Some(StreamEvent::RedactedThinkingDone { index, data })
                }
                Some(BlockState::ToolUse { id, name, input }) => Some(StreamEvent::ToolDone {
                    index,
                    tool_name: name,
                    tool_use_id: id,
                    input_json: input,
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
                None => None,
            }
        }
        SseEvent::MessageDelta {
            delta,
            usage,
            context_management,
        } => {
            // Log server-side context management metadata when present.
            if let Some(ref cm) = context_management {
                tracing::debug!(
                    target: "jfc::stream",
                    context_management = ?cm,
                    "server-side context management active"
                );
                if cm.compacted {
                    tracing::info!(
                        target: "jfc::stream",
                        removed_tokens = ?cm.removed_tokens,
                        "server compacted context (context_management.compacted=true)"
                    );
                }
            }
            *stop_reason = Some(parse_stop_reason(delta.stop_reason.as_deref()));
            usage.map(|usage| StreamEvent::Usage {
                input_tokens: usage.input_tokens(),
                output_tokens: usage.output_total(),
                cache_read_tokens: usage.cache_read_input_tokens.unwrap_or_default(),
                cache_write_tokens: usage.cache_creation_input_tokens.unwrap_or_default(),
            })
        }
        SseEvent::MessageStop => {
            // Same silent-default trap as parse_stop_reason(None):
            // message_stop without a preceding message_delta means the
            // upstream forgot to tell us why the turn ended. Default to
            // EndTurn for back-compat but log so a stalled stream is
            // diagnosable. Mirrors the warn in parse_stop_reason.
            let reason = match stop_reason.take() {
                Some(r) => r,
                None => {
                    tracing::warn!(
                        target: "jfc::provider::sse",
                        "message_stop arrived without a preceding message_delta \
                         (no stop_reason was set) — defaulting to EndTurn; if the \
                         turn looks truncated, check the raw SSE log for the missing \
                         delta event"
                    );
                    StopReason::EndTurn
                }
            };
            Some(StreamEvent::Done {
                stop_reason: reason,
            })
        }
        SseEvent::Error { error } => {
            let message = match error.kind.as_deref() {
                Some("overloaded_error" | "rate_limit_error" | "api_error") => {
                    format!("{}{}", super::anthropic::AUTO_RETRY_SENTINEL, error.message)
                }
                _ => error.message,
            };
            Some(StreamEvent::Error { message })
        }
        SseEvent::MessageStart { message } => Some(StreamEvent::ResponseMetadata {
            response_id: message.id,
        }),
        SseEvent::Ping => None,
    }
}

/// Anthropic's Messages API requires `tool_use.input` to be a JSON object.
/// Streamed deltas, Generic ToolInput fallbacks, and round-trip edge cases can
/// produce a `Value::String` (stringified JSON) or `Value::Null`. This helper
/// coerces non-object values into valid objects before the request leaves jfc.
///
/// Mirrors the v137 CLI logic at line 434836:
///   if typeof input === "string" → JSON.parse(input) ?? {}
///   if typeof input !== "object" → throw (we default to {} instead)
fn ensure_input_object(v: &serde_json::Value) -> serde_json::Value {
    match v {
        serde_json::Value::Object(_) => v.clone(),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() || trimmed == "null" {
                return serde_json::json!({});
            }
            match serde_json::from_str::<serde_json::Value>(trimmed) {
                Ok(serde_json::Value::Object(map)) => serde_json::Value::Object(map),
                Ok(other) => {
                    // Parsed but not an object (e.g., array, number). Wrap it
                    // so the API gets a valid object.
                    serde_json::json!({ "value": other })
                }
                Err(_) => serde_json::json!({}),
            }
        }
        serde_json::Value::Null => serde_json::json!({}),
        // Array/Number/Bool — shouldn't happen but handle defensively.
        other => serde_json::json!({ "value": other }),
    }
}

pub fn build_messages(messages: &[ProviderMessage]) -> Value {
    let tool_use_count = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter(|c| matches!(c, ProviderContent::ToolUse { .. }))
        .count();
    let tool_result_count = messages
        .iter()
        .flat_map(|m| m.content.iter())
        .filter(|c| matches!(c, ProviderContent::ToolResult { .. }))
        .count();
    tracing::debug!(
        target: "jfc::provider::sse",
        message_count = messages.len(),
        tool_use_count,
        tool_result_count,
        "build_messages"
    );
    let mut out: Vec<Value> = messages
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
                        "input": ensure_input_object(input),
                    }),
                    // Server-side tools round-trip with their original
                    // wire type. Re-emitting them as plain `tool_use`
                    // breaks Anthropic's server-side sampling loop
                    // resumption (cli.js v142:7057, :441090). Anthropic
                    // also accepts `server_tool_use.input` as either a
                    // string OR an object on resend (cli.js v142:441090
                    // tolerates both), so we run the same coercion as
                    // for regular `tool_use` to land on the safe shape.
                    ProviderContent::ServerToolUse { id, name, input } => json!({
                        "type": "server_tool_use",
                        "id": id,
                        "name": name,
                        "input": ensure_input_object(input),
                    }),
                    // Server-side tool results re-emit verbatim with
                    // their original `type` string and content. Per
                    // cli.js v142:441375 these survive the
                    // normalize-for-resend pass unchanged.
                    ProviderContent::ServerToolResult {
                        tool_use_id,
                        tool_kind,
                        content,
                    } => json!({
                        "type": tool_kind.wire_type(),
                        "tool_use_id": tool_use_id,
                        "content": content,
                    }),
                    // Image (PNG/JPEG/GIF/WebP) → `image` block;
                    // PDF → `document` block. Both share the base64
                    // source struct — `to_anthropic_content_block`
                    // owns the type-routing rule.
                    ProviderContent::Attachment(att) => {
                        crate::attachments::to_anthropic_content_block(att)
                    }
                    ProviderContent::RedactedThinking { data } => json!({
                        "type": "redacted_thinking",
                        "data": data,
                    }),
                })
                .collect();
            json!({ "role": role, "content": content })
        })
        .collect();

    // Prompt-caching: place cache_control breakpoints on the last content
    // block of the last 2 user messages. This matches cli.js v142's YB5()
    // strategy — everything before the second-to-last user turn is served
    // from cache on subsequent requests.
    let user_indices: Vec<usize> = out
        .iter()
        .enumerate()
        .filter(|(_, m)| m.get("role").and_then(|r| r.as_str()) == Some("user"))
        .map(|(i, _)| i)
        .collect();
    for &idx in user_indices.iter().rev().take(2) {
        if let Some(content) = out[idx].get_mut("content").and_then(|c| c.as_array_mut()) {
            if let Some(last_block) = content.last_mut() {
                last_block["cache_control"] = json!({ "type": "ephemeral" });
            }
        }
    }

    out.into()
}

pub fn build_tools(tools: &[ToolDef]) -> Value {
    tracing::trace!(
        target: "jfc::provider::sse",
        tool_count = tools.len(),
        "build_tools"
    );
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
    // Tracing parity with the OpenWebUI provider: dump raw SSE bytes at TRACE,
    // log every parsed event type at DEBUG, log finish_reason / errors at INFO.
    // Flip `RUST_LOG=jfc::provider::anthropic_sse=trace` to see raw chunks
    // when debugging upstream SSE weirdness.
    let event_stream = jfc_anthropic_sdk::sse::response_event_stream(resp)
        .scan(
            (Vec::<Option<BlockState>>::new(), None::<StopReason>),
            |state, result| {
                let (blocks, stop_reason) = state;
                let out = match result {
                    Ok(ev) => {
                        tracing::trace!(
                            target: "jfc::provider::anthropic_sse",
                            event = %ev.event,
                            data = %&ev.data[..ev.data.len().min(400)],
                            "sse raw"
                        );
                        if ev.event == "ping" || ev.data.is_empty() {
                            return futures::future::ready(Some(None));
                        }
                        if ev.data == "[DONE]" {
                            tracing::debug!(target: "jfc::provider::anthropic_sse", "sse [DONE]");
                            return futures::future::ready(Some(None));
                        }
                        // `context_hint` is a special SSE event type (not a JSON
                        // `type` field) that Anthropic sends when the model is
                        // approaching its context limit. Mirrors v132 cli.js line
                        // 471490: treat it the same as a prompt_too_long rejection
                        // so the main loop fires auto-compaction.
                        if ev.event == "context_hint" || ev.data.contains("\"context_hint\"") {
                            tracing::info!(
                                target: "jfc::provider::anthropic_sse",
                                event = %ev.event,
                                data = %&ev.data[..ev.data.len().min(200)],
                                "context_hint received — signalling auto-compact"
                            );
                            return futures::future::ready(Some(Some(Ok(StreamEvent::Error {
                                message: format!(
                                    "auto-compact: context_hint from server ({})",
                                    &ev.data[..ev.data.len().min(120)]
                                ),
                            }))));
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
                    }
                    Err(e) => Some(Err(anyhow::anyhow!("SSE stream parse error: {e}"))),
                };
                futures::future::ready(Some(out))
            },
        )
        .filter_map(futures::future::ready);

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
                ContentBlock::RedactedThinking { .. } => "redacted_thinking",
                ContentBlock::ToolUse { .. } => "tool_use",
                ContentBlock::ServerToolUse { .. } => "server_tool_use",
                ContentBlock::WebSearchToolResult { .. } => "web_search_tool_result",
                ContentBlock::CodeExecutionToolResult { .. } => "code_execution_tool_result",
                ContentBlock::WebFetchToolResult { .. } => "web_fetch_tool_result",
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
                ContentBlock::WebSearchToolResult { tool_use_id, .. }
                | ContentBlock::CodeExecutionToolResult { tool_use_id, .. }
                | ContentBlock::WebFetchToolResult { tool_use_id, .. } => {
                    tracing::info!(
                        target: "jfc::provider::anthropic_sse",
                        index,
                        kind,
                        tool_use_id = %tool_use_id,
                        "content_block_start server_tool_result"
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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_provider::{
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
        // pause_turn must NOT bucket into Other(...) — that drops it into
        // event_loop's "model said its piece" else branch and silently ends
        // the agentic loop. See StopReason::PauseTurn docs.
        assert_eq!(parse_stop_reason(Some("pause_turn")), StopReason::PauseTurn);
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
                context_management: None,
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

    // Robust: `parse_stop_reason(None)` still falls back to EndTurn for
    // back-compat with truncated/short-circuited streams, but the
    // behavior is documented + warn-logged so the silent fallback
    // doesn't hide a future variant the way it hid pause_turn for
    // months. This test pins the contract: missing field → EndTurn,
    // NOT panic, NOT Other(""), NOT Other("null").
    #[test]
    fn parse_stop_reason_none_falls_back_to_end_turn_robust() {
        assert_eq!(parse_stop_reason(None), StopReason::EndTurn);
    }

    // Robust: an unknown variant string buckets into Other(...) and is
    // expected to surface a warn in the trace log. We can't easily
    // capture the tracing event from a unit test without a
    // subscriber-capture rig, but we DO pin that the variant is
    // preserved verbatim so the user can grep their logs for the
    // exact string Anthropic sent.
    #[test]
    fn parse_stop_reason_unknown_string_preserves_variant_robust() {
        assert_eq!(
            parse_stop_reason(Some("refusal")),
            StopReason::Other("refusal".into())
        );
        assert_eq!(
            parse_stop_reason(Some("container_oom")),
            StopReason::Other("container_oom".into())
        );
        // Empty string is its own degenerate case — preserved (NOT
        // collapsed to EndTurn) so it shows up in logs as
        // `Other("")` which is grep-able.
        assert_eq!(parse_stop_reason(Some("")), StopReason::Other("".into()));
    }

    // Normal: a message_delta with stop_reason="pause_turn" followed by
    // message_stop produces a Done{PauseTurn} — NOT Other("pause_turn"),
    // which would silently fall through event_loop's dispatch ladder into
    // the "model said its piece" branch and end the agentic loop. See
    // StopReason::PauseTurn docs.
    #[test]
    fn translate_message_stop_with_pause_turn_normal() {
        let (mut blocks, mut sr) = empty_state();
        translate(
            SseEvent::MessageDelta {
                delta: MessageDeltaData {
                    stop_reason: Some("pause_turn".into()),
                },
                usage: None,
                context_management: None,
            },
            &mut blocks,
            &mut sr,
        );
        let out = translate(SseEvent::MessageStop, &mut blocks, &mut sr);
        assert!(matches!(
            out,
            Some(StreamEvent::Done {
                stop_reason: StopReason::PauseTurn
            })
        ));
    }

    #[test]
    fn translate_error_event() {
        let (mut blocks, mut sr) = empty_state();
        let out = translate(
            SseEvent::Error {
                error: ErrorBody {
                    kind: None,
                    message: "overloaded".into(),
                },
            },
            &mut blocks,
            &mut sr,
        );
        assert!(matches!(out, Some(StreamEvent::Error { message }) if message == "overloaded"));
    }

    #[test]
    fn translate_transient_error_event_requests_auto_retry() {
        let (mut blocks, mut sr) = empty_state();
        for kind in ["overloaded_error", "rate_limit_error", "api_error"] {
            let out = translate(
                SseEvent::Error {
                    error: ErrorBody {
                        kind: Some(kind.into()),
                        message: "transient".into(),
                    },
                },
                &mut blocks,
                &mut sr,
            );
            assert!(
                matches!(out, Some(StreamEvent::Error { message }) if message.starts_with(crate::providers::anthropic::AUTO_RETRY_SENTINEL)),
                "{kind}"
            );
        }
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
    fn message_start_usage_keeps_cache_tokens_separate() {
        let json = r#"{"type":"message_start","message":{"id":"msg_1","usage":{"input_tokens":10,"cache_creation_input_tokens":3,"cache_read_input_tokens":7}}}"#;
        let event: SseEvent = serde_json::from_str(json).expect("message_start usage must parse");
        let (mut blocks, mut sr) = empty_state();

        assert!(matches!(
            translate(event, &mut blocks, &mut sr),
            Some(StreamEvent::Usage {
                input_tokens: 10,
                cache_read_tokens: 7,
                cache_write_tokens: 3,
                output_tokens: 0,
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

    #[test]
    fn ensure_input_object_passes_objects_through() {
        let obj = serde_json::json!({"path": "/tmp", "recursive": true});
        let result = ensure_input_object(&obj);
        assert_eq!(result, obj);
    }

    #[test]
    fn ensure_input_object_parses_stringified_json() {
        let s = serde_json::Value::String(r#"{"path":"/tmp"}"#.to_owned());
        let result = ensure_input_object(&s);
        assert_eq!(result, serde_json::json!({"path": "/tmp"}));
    }

    #[test]
    fn ensure_input_object_empty_string_becomes_empty_object() {
        let s = serde_json::Value::String("".to_owned());
        assert_eq!(ensure_input_object(&s), serde_json::json!({}));
    }

    #[test]
    fn ensure_input_object_null_string_becomes_empty_object() {
        let s = serde_json::Value::String("null".to_owned());
        assert_eq!(ensure_input_object(&s), serde_json::json!({}));
    }

    #[test]
    fn ensure_input_object_null_value_becomes_empty_object() {
        assert_eq!(
            ensure_input_object(&serde_json::Value::Null),
            serde_json::json!({})
        );
    }

    #[test]
    fn ensure_input_object_unparseable_string_becomes_empty_object() {
        let s = serde_json::Value::String("not json at all".to_owned());
        assert_eq!(ensure_input_object(&s), serde_json::json!({}));
    }

    #[test]
    fn ensure_input_object_string_array_gets_wrapped() {
        let s = serde_json::Value::String("[1, 2, 3]".to_owned());
        let result = ensure_input_object(&s);
        assert_eq!(result, serde_json::json!({"value": [1, 2, 3]}));
    }

    // ─── server_tool_use tests ────────────────────────────────────────────────

    #[test]
    fn server_tool_use_content_block_parses() {
        let json = r#"{"type":"content_block_start","index":0,"content_block":{"type":"server_tool_use","id":"srvtool_1","name":"web_search","input":{"query":"rust async"}}}"#;
        let event: SseEvent = serde_json::from_str(json).expect("server_tool_use must parse");
        assert!(matches!(
            event,
            SseEvent::ContentBlockStart {
                content_block: ContentBlock::ServerToolUse { .. },
                ..
            }
        ));
    }

    #[test]
    fn server_tool_use_block_emits_tool_done_with_prefix() {
        let (mut blocks, mut sr) = empty_state();
        translate(
            SseEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::ServerToolUse {
                    id: "srvtool_1".into(),
                    name: "web_search".into(),
                    input: serde_json::json!({"query": "rust async"}),
                },
            },
            &mut blocks,
            &mut sr,
        );
        // Server tools pre-populate input at start, not via deltas
        assert!(matches!(blocks[0], Some(BlockState::ServerToolUse { .. })));

        let out = translate(
            SseEvent::ContentBlockStop { index: 0 },
            &mut blocks,
            &mut sr,
        );
        // ToolDone is emitted with "server_tool_use:" prefix so stream.rs
        // can route to a non-dispatch path.
        assert!(
            matches!(out, Some(StreamEvent::ToolDone { ref tool_name, ref tool_use_id, .. })
                if tool_name == "server_tool_use:web_search" && tool_use_id == "srvtool_1"),
            "expected ToolDone with server_tool_use: prefix, got: {out:?}"
        );
        assert!(blocks[0].is_none());
    }

    #[test]
    fn server_tool_use_null_input_produces_empty_string() {
        let (mut blocks, mut sr) = empty_state();
        translate(
            SseEvent::ContentBlockStart {
                index: 0,
                content_block: ContentBlock::ServerToolUse {
                    id: "srvtool_2".into(),
                    name: "code_execution".into(),
                    input: serde_json::Value::Null,
                },
            },
            &mut blocks,
            &mut sr,
        );
        if let Some(Some(BlockState::ServerToolUse { input, .. })) = blocks.get(0) {
            assert!(input.is_empty(), "null input should become empty string");
        } else {
            panic!("expected ServerToolUse block state");
        }
    }

    #[test]
    fn server_tool_use_from_name_routes_to_server_variant() {
        use crate::types::ToolKind;
        assert!(
            matches!(
                ToolKind::from_name("server_tool_use:web_search"),
                ToolKind::ServerWebSearch
            ),
            "server_tool_use:web_search should map to ServerWebSearch"
        );
        assert!(
            matches!(
                ToolKind::from_name("server_tool_use:code_execution"),
                ToolKind::ServerCodeExecution
            ),
            "server_tool_use:code_execution should map to ServerCodeExecution"
        );
        assert!(
            matches!(
                ToolKind::from_name("server_tool_use:unknown_future_tool"),
                ToolKind::Generic(_)
            ),
            "unknown server tool should fall through to Generic"
        );
    }
}
