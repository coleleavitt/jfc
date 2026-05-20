//! Antigravity request/response transform for the Code Assist API.
//!
//! Two concerns live here:
//!
//! 1. **Request envelope** ([`build_gemini_request`]): wraps the
//!    `Vec<ProviderMessage>` + `StreamOptions` in the Code Assist payload
//!    Google's `v1internal:streamGenerateContent` expects:
//!
//!    ```jsonc
//!    {
//!      "project":  "...",
//!      "model":    "gemini-3-pro",
//!      "userAgent":"antigravity",
//!      "requestId":"<uuid>",
//!      "request":  { /* GenerateContentRequest: contents/tools/etc. */ }
//!    }
//!    ```
//!
//! 2. **Response parser** ([`parse_sse_chunk`] + [`into_event_stream`]):
//!    consumes Gemini SSE `data: {...}` frames carrying `candidates[].content`
//!    + `usageMetadata` and converts them into jfc's [`StreamEvent`] enum.
//!
//! Both pieces are pure functions (modulo HTTP) so they're unit-testable
//! without a live Google account. The Claude-via-Antigravity request shape
//! (`gemini-claude-*` models) is NOT handled here — those models would need
//! the Anthropic Messages-format envelope from `transform/claude.ts`; this
//! module raises a clear error in `build_gemini_request` if asked for one,
//! and the provider falls back to the auth-only behaviour for them.

use futures::StreamExt;
use jfc_provider::{
    EventStream, ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent,
    StreamOptions, ToolDef,
};
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

/// Translate a model id like `gemini-3-pro` / `gemini-claude-sonnet-4-5` into
/// the Code Assist short name. Mirrors `resolveModelName` in the TS plugin's
/// `request-helpers.ts` for the names we actually expose; unknown ids pass
/// through unchanged so the upstream returns a clean error.
pub fn resolve_model_name(model: &str) -> &str {
    match model {
        // Common short forms used by the upstream plugin.
        "gemini-pro" => "gemini-3-pro",
        "gemini-flash" => "gemini-3-flash",
        // gemini-claude-* and explicit gemini-* pass through verbatim.
        other => other,
    }
}

/// Is this a Claude-via-Gemini model id?
pub fn is_claude_model(model: &str) -> bool {
    model.contains("claude")
}

/// Build the Code Assist `streamGenerateContent` request body for a
/// Gemini-native model. Returns `Err` for Claude-via-Gemini models — those
/// route through a different transform that isn't ported yet.
pub fn build_gemini_request(
    project_id: &str,
    messages: &[ProviderMessage],
    options: &StreamOptions,
) -> anyhow::Result<Value> {
    let model = resolve_model_name(options.model.as_str()).to_owned();
    if is_claude_model(&model) {
        anyhow::bail!(
            "Antigravity Claude-via-Gemini models ({model}) require the \
             Anthropic request transform, which isn't ported yet"
        );
    }

    let contents = messages.iter().filter_map(message_to_content).collect::<Vec<_>>();

    let mut request = json!({ "contents": contents });
    if let Some(sys) = options.system.as_deref().filter(|s| !s.is_empty()) {
        request["systemInstruction"] = json!({
            "parts": [ { "text": sys } ],
        });
    }
    if !options.tools.is_empty() {
        request["tools"] = json!([
            { "functionDeclarations": options.tools.iter().map(tool_to_decl).collect::<Vec<_>>() }
        ]);
    }

    let mut generation_config = serde_json::Map::new();
    generation_config.insert("maxOutputTokens".into(), json!(options.max_tokens));
    if let Some(temp) = options.temperature {
        generation_config.insert("temperature".into(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        generation_config.insert("topP".into(), json!(top_p));
    }
    if let Some(budget) = options.thinking_budget {
        generation_config.insert(
            "thinkingConfig".into(),
            json!({ "thinkingBudget": budget }),
        );
    }
    if !generation_config.is_empty() {
        request["generationConfig"] = Value::Object(generation_config);
    }

    Ok(json!({
        "project": project_id,
        "model": model,
        "userAgent": "antigravity",
        "requestType": "agent",
        "requestId": Uuid::new_v4().to_string(),
        "request": request,
    }))
}

fn message_to_content(message: &ProviderMessage) -> Option<Value> {
    let role = match message.role {
        ProviderRole::User => "user",
        ProviderRole::Assistant => "model",
    };
    let parts: Vec<Value> = message
        .content
        .iter()
        .filter_map(content_to_part)
        .collect();
    if parts.is_empty() {
        return None;
    }
    Some(json!({ "role": role, "parts": parts }))
}

fn content_to_part(content: &ProviderContent) -> Option<Value> {
    match content {
        ProviderContent::Text(text) if text.is_empty() => None,
        ProviderContent::Text(text) => Some(json!({ "text": text })),
        ProviderContent::ToolUse { id: _, name, input } => Some(json!({
            "functionCall": { "name": name, "args": input },
        })),
        ProviderContent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } => Some(json!({
            "functionResponse": {
                "name": tool_use_id,
                "response": { "content": content, "isError": is_error },
            },
        })),
        // Gemini doesn't have a direct analog for Anthropic-style
        // server_tool_use / redacted thinking blocks — drop them so the
        // request stays well-formed. The renderer keeps them in the local
        // transcript regardless.
        _ => None,
    }
}

fn tool_to_decl(tool: &ToolDef) -> Value {
    json!({
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
    })
}

// ─── SSE response parsing ────────────────────────────────────────────────────

/// One parsed Gemini SSE frame — the shape we care about from
/// `candidates[].content.parts[]` plus optional usage metadata.
#[derive(Debug, Deserialize, Default)]
struct GeminiSseFrame {
    #[serde(default)]
    candidates: Vec<GeminiCandidate>,
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<GeminiUsage>,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiCandidate {
    #[serde(default)]
    content: Option<GeminiContent>,
    #[serde(default, rename = "finishReason")]
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiContent {
    #[serde(default)]
    parts: Vec<GeminiPart>,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiPart {
    #[serde(default)]
    text: Option<String>,
    #[serde(default)]
    thought: Option<bool>,
    #[serde(default, rename = "functionCall")]
    function_call: Option<GeminiFunctionCall>,
}

#[derive(Debug, Deserialize)]
struct GeminiFunctionCall {
    name: String,
    #[serde(default)]
    args: Value,
}

#[derive(Debug, Deserialize, Default)]
struct GeminiUsage {
    #[serde(default, rename = "promptTokenCount")]
    prompt_tokens: Option<u32>,
    #[serde(default, rename = "candidatesTokenCount")]
    candidates_tokens: Option<u32>,
    #[serde(default, rename = "cachedContentTokenCount")]
    cached_tokens: Option<u32>,
}

/// Per-stream cursor state needed to emit `TextDelta`s with monotonically
/// increasing indices, and to convert Gemini `finishReason` strings into a
/// jfc-side [`StopReason`] at end-of-stream.
#[derive(Debug, Default)]
struct ParserState {
    text_index: usize,
    text_started: bool,
    tool_index: usize,
}

impl ParserState {
    fn next_text_idx(&mut self) -> usize {
        if !self.text_started {
            self.text_started = true;
            self.text_index
        } else {
            self.text_index
        }
    }
    fn fresh_tool_idx(&mut self) -> usize {
        let n = self.tool_index;
        self.tool_index += 1;
        n
    }
}

/// Parse a single decoded SSE frame body (the part after `data: `) into a
/// flat sequence of [`StreamEvent`]s. Pure; tested without a live endpoint.
fn parse_frame(json: &str, state: &mut ParserState) -> Vec<StreamEvent> {
    let trimmed = json.trim();
    if trimmed.is_empty() || trimmed == "[DONE]" {
        return Vec::new();
    }
    let frame: GeminiSseFrame = match serde_json::from_str(trimmed) {
        Ok(f) => f,
        Err(err) => {
            return vec![StreamEvent::Error {
                message: format!("malformed Gemini SSE frame: {err}"),
            }];
        }
    };

    let mut out = Vec::new();
    for candidate in frame.candidates {
        emit_candidate_events(candidate, state, &mut out);
    }
    if let Some(usage) = frame.usage_metadata {
        out.push(StreamEvent::Usage {
            input_tokens: usage.prompt_tokens.unwrap_or(0),
            output_tokens: usage.candidates_tokens.unwrap_or(0),
            cache_read_tokens: usage.cached_tokens.unwrap_or(0),
            cache_write_tokens: 0,
        });
    }
    out
}

fn emit_candidate_events(
    candidate: GeminiCandidate,
    state: &mut ParserState,
    out: &mut Vec<StreamEvent>,
) {
    if let Some(content) = candidate.content {
        for part in content.parts {
            if let Some(ev) = part_to_event(part, state) {
                out.push(ev);
            }
        }
    }
    if let Some(reason) = candidate.finish_reason {
        out.push(StreamEvent::Done {
            stop_reason: map_finish_reason(&reason),
        });
    }
}

fn part_to_event(part: GeminiPart, state: &mut ParserState) -> Option<StreamEvent> {
    if let Some(call) = part.function_call {
        let idx = state.fresh_tool_idx();
        let input_json = serde_json::to_string(&call.args).unwrap_or_else(|_| "{}".into());
        return Some(StreamEvent::ToolDone {
            index: idx,
            tool_name: call.name,
            tool_use_id: format!("toolu_{}", Uuid::new_v4().simple()),
            input_json,
        });
    }
    let text = part.text?;
    if text.is_empty() {
        return None;
    }
    if part.thought == Some(true) {
        Some(StreamEvent::ThinkingDelta {
            index: state.text_index,
            delta: text,
        })
    } else {
        let idx = state.next_text_idx();
        Some(StreamEvent::TextDelta { index: idx, delta: text })
    }
}

fn map_finish_reason(reason: &str) -> StopReason {
    match reason {
        "STOP" | "" => StopReason::EndTurn,
        "MAX_TOKENS" => StopReason::MaxTokens,
        "TOOL_CODE" | "FUNCTION_CALL" | "TOOL_USE" => StopReason::ToolUse,
        "STOP_SEQUENCE" => StopReason::StopSequence,
        other => StopReason::Other(other.to_owned()),
    }
}

/// Wrap a `reqwest::Response` SSE stream as an [`EventStream`] of jfc
/// [`StreamEvent`]s by piping each parsed frame through [`parse_frame`].
/// Reuses the byte-level SSE parser from `jfc_anthropic_sdk::sse` so we
/// don't reinvent line-buffering or split-across-chunks handling.
pub fn into_event_stream(resp: reqwest::Response) -> EventStream {
    let raw = jfc_anthropic_sdk::sse::response_event_stream(resp);
    let stream = raw
        .scan(ParserState::default(), |state, frame| {
            let events = match frame {
                Ok(frame) if frame.data.is_empty() || frame.data == "[DONE]" => Vec::new(),
                Ok(frame) => parse_frame(&frame.data, state),
                Err(err) => vec![StreamEvent::Error {
                    message: format!("Antigravity SSE transport error: {err}"),
                }],
            };
            futures::future::ready(Some(events))
        })
        .flat_map(futures::stream::iter)
        .map(Ok);
    stream.boxed()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

    fn user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(text.into())],
        }
    }

    #[test]
    fn build_gemini_request_wraps_with_project_and_model_normal() {
        let mut opts = StreamOptions::new("gemini-3-pro");
        opts.system = Some("be helpful".into());
        let body =
            build_gemini_request("proj-123", &[user_msg("hi")], &opts).expect("gemini req");
        assert_eq!(body["project"], "proj-123");
        assert_eq!(body["model"], "gemini-3-pro");
        assert_eq!(body["userAgent"], "antigravity");
        assert_eq!(body["request"]["contents"][0]["role"], "user");
        assert_eq!(body["request"]["contents"][0]["parts"][0]["text"], "hi");
        assert_eq!(
            body["request"]["systemInstruction"]["parts"][0]["text"],
            "be helpful"
        );
    }

    #[test]
    fn build_gemini_request_resolves_short_aliases_normal() {
        let opts = StreamOptions::new("gemini-pro");
        let body = build_gemini_request("p", &[user_msg("x")], &opts).unwrap();
        assert_eq!(body["model"], "gemini-3-pro");
    }

    #[test]
    fn build_gemini_request_rejects_claude_models_robust() {
        let opts = StreamOptions::new("gemini-claude-sonnet-4-5");
        let err = build_gemini_request("p", &[user_msg("x")], &opts).unwrap_err();
        assert!(err.to_string().contains("Anthropic request transform"));
    }

    #[test]
    fn build_gemini_request_omits_empty_text_and_handles_tool_use_normal() {
        let opts = StreamOptions::new("gemini-3-pro");
        let msg = ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![
                ProviderContent::Text("".into()),
                ProviderContent::ToolUse {
                    id: "x".into(),
                    name: "search".into(),
                    input: json!({ "q": "rust" }),
                },
            ],
        };
        let body = build_gemini_request("p", &[msg], &opts).unwrap();
        let parts = &body["request"]["contents"][0]["parts"];
        assert_eq!(parts.as_array().unwrap().len(), 1);
        assert_eq!(parts[0]["functionCall"]["name"], "search");
    }

    #[test]
    fn parse_frame_handles_text_delta_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"candidates":[{"content":{"parts":[{"text":"hello"}]}}]}"#,
            &mut state,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::TextDelta { delta, .. } => assert_eq!(delta, "hello"),
            other => panic!("expected TextDelta, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_handles_function_call_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"candidates":[{"content":{"parts":[{"functionCall":{"name":"search","args":{"q":"x"}}}]}}]}"#,
            &mut state,
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            StreamEvent::ToolDone {
                tool_name,
                input_json,
                ..
            } => {
                assert_eq!(tool_name, "search");
                assert!(input_json.contains("\"q\""));
            }
            other => panic!("expected ToolDone, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_handles_thinking_delta_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"candidates":[{"content":{"parts":[{"text":"thought","thought":true}]}}]}"#,
            &mut state,
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::ThinkingDelta { .. }));
    }

    #[test]
    fn parse_frame_emits_done_on_finish_reason_normal() {
        let mut state = ParserState::default();
        let events =
            parse_frame(r#"{"candidates":[{"finishReason":"STOP"}]}"#, &mut state);
        assert!(events.iter().any(
            |e| matches!(e, StreamEvent::Done { stop_reason: StopReason::EndTurn })
        ));
    }

    #[test]
    fn parse_frame_emits_usage_normal() {
        let mut state = ParserState::default();
        let events = parse_frame(
            r#"{"usageMetadata":{"promptTokenCount":42,"candidatesTokenCount":7}}"#,
            &mut state,
        );
        match events.last().expect("usage event") {
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                ..
            } => {
                assert_eq!(*input_tokens, 42);
                assert_eq!(*output_tokens, 7);
            }
            other => panic!("expected Usage, got {other:?}"),
        }
    }

    #[test]
    fn parse_frame_reports_malformed_json_as_error_event_robust() {
        let mut state = ParserState::default();
        let events = parse_frame("{not json", &mut state);
        assert_eq!(events.len(), 1);
        assert!(matches!(events[0], StreamEvent::Error { .. }));
    }
}
