use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use jfc_provider::{
    CompletionResponse, EventStream, ModelId, ModelInfo, Provider, ProviderContent, ProviderId,
    ProviderMessage, ProviderRole, StopReason, StreamConvention, StreamEvent, StreamOptions,
    TokenUsage, ToolDef,
};

const PROVIDER_ID: &str = "openai";
const DEFAULT_BASE_URL: &str = "https://api.openai.com/v1";

#[derive(Clone)]
pub struct OpenAIProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl OpenAIProvider {
    pub fn from_env() -> Option<Self> {
        // Try env var first (standard).
        let api_key = std::env::var("OPENAI_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            // Fall back to ~/.config/jfc/credentials.toml
            .or_else(Self::key_from_credentials_file)
            .map(|k| k.trim().to_owned())?;

        if api_key.is_empty() {
            return None;
        }

        Some(Self::new(
            api_key,
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned()),
        ))
    }

    /// Read the OpenAI API key from `~/.config/jfc/credentials.toml`
    /// if it exists. Format:
    /// ```toml
    /// [openai]
    /// api_key = "sk-..."
    /// ```
    fn key_from_credentials_file() -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        let path = std::path::PathBuf::from(home).join(".config/jfc/credentials.toml");
        let content = std::fs::read_to_string(&path).ok()?;
        // Minimal TOML parsing — just find [openai] section's api_key.
        let mut in_openai = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "[openai]" {
                in_openai = true;
                continue;
            }
            if trimmed.starts_with('[') {
                in_openai = false;
                continue;
            }
            if in_openai && trimmed.starts_with("api_key") {
                // Parse: api_key = "value"
                if let Some(val) = trimmed.split('=').nth(1) {
                    let val = val.trim().trim_matches('"').trim_matches('\'');
                    if !val.is_empty() {
                        return Some(val.to_string());
                    }
                }
            }
        }
        None
    }

    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: jfc_provider::http::streaming_client(),
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn responses_url(&self) -> String {
        format!("{}/responses", self.base_url)
    }

    fn models_url(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn fallback_models() -> Vec<ModelInfo> {
        [
            ("gpt-5.1", "GPT-5.1", Some(400_000), Some(128_000)),
            ("gpt-5", "GPT-5", Some(400_000), Some(128_000)),
            ("gpt-5-mini", "GPT-5 Mini", Some(400_000), Some(128_000)),
            ("gpt-5-nano", "GPT-5 Nano", Some(400_000), Some(128_000)),
            ("gpt-4.1", "GPT-4.1", Some(1_000_000), Some(32_768)),
            (
                "gpt-4.1-mini",
                "GPT-4.1 Mini",
                Some(1_000_000),
                Some(32_768),
            ),
            ("o3", "o3", Some(200_000), Some(100_000)),
            ("o4-mini", "o4 Mini", Some(200_000), Some(100_000)),
        ]
        .into_iter()
        .map(|(id, name, context, output)| {
            ModelInfo::new(ModelId::new(id), name, ProviderId::new(PROVIDER_ID))
                .with_context_window_tokens(context)
                .with_max_output_tokens(output)
        })
        .collect()
    }
}

impl jfc_provider::seal::Sealed for OpenAIProvider {}

#[async_trait]
impl Provider for OpenAIProvider {
    fn name(&self) -> &str {
        PROVIDER_ID
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Self::fallback_models()
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::OpenAiNative
    }

    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let resp = self
            .client
            .get(self.models_url())
            .bearer_auth(&self.api_key)
            .send()
            .await?
            .error_for_status()?;

        let body: ModelsResponse = resp.json().await?;
        let mut models: Vec<ModelInfo> = body
            .data
            .into_iter()
            .filter(|m| is_chat_model(&m.id))
            .map(|m| {
                ModelInfo::new(
                    ModelId::new(m.id.clone()),
                    m.id,
                    ProviderId::new(PROVIDER_ID),
                )
            })
            .collect();

        models.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        if models.is_empty() {
            Ok(Self::fallback_models())
        } else {
            Ok(models)
        }
    }

    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        if model_uses_responses(options.model.as_str()) {
            let body = build_responses_body(messages, options, true);
            let url = self.responses_url();
            let send_started = std::time::Instant::now();
            let resp = match jfc_provider::http::send_with_retry("openai.responses", || {
                self.client
                    .post(&url)
                    .bearer_auth(&self.api_key)
                    .json(&body)
                    .send()
            })
            .await
            {
                Ok(r) => r,
                Err(e) => {
                    let cause = jfc_provider::http::classify_send_error(&e);
                    tracing::warn!(
                        target: "jfc::provider::openai",
                        url = %url,
                        error = %e,
                        cause = cause,
                        "POST responses failed before response (after retries)"
                    );
                    anyhow::bail!("OpenAI request failed: {cause} ({e})");
                }
            };
            jfc_provider::http::report_first_byte_latency(
                "openai.responses",
                send_started.elapsed(),
            );
            if !resp.status().is_success() {
                let status = resp.status();
                let text = resp.text().await.unwrap_or_default();
                let friendly = jfc_provider::retry::friendly_error_message(status.as_u16(), &text);
                anyhow::bail!("OpenAI API error {status}: {friendly}\n  raw: {text}");
            }
            return Ok(responses_event_stream(resp));
        }

        let body = super::openwebui::build_body(messages, options);
        let url = self.chat_url();
        let send_started = std::time::Instant::now();
        let resp = match jfc_provider::http::send_with_retry("openai.chat/completions", || {
            self.client
                .post(&url)
                .bearer_auth(&self.api_key)
                .json(&body)
                .send()
        })
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let cause = jfc_provider::http::classify_send_error(&e);
                tracing::warn!(
                    target: "jfc::provider::openai",
                    url = %url,
                    error = %e,
                    cause = cause,
                    "POST chat/completions failed before response (after retries)"
                );
                anyhow::bail!("OpenAI request failed: {cause} ({e})");
            }
        };
        jfc_provider::http::report_first_byte_latency(
            "openai.chat/completions",
            send_started.elapsed(),
        );
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let friendly = jfc_provider::retry::friendly_error_message(status.as_u16(), &text);
            anyhow::bail!("OpenAI API error {status}: {friendly}\n  raw: {text}");
        }
        Ok(super::openwebui::openai_compatible_event_stream(resp))
    }

    async fn complete(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<CompletionResponse> {
        if model_uses_responses(options.model.as_str()) {
            let resp = self
                .client
                .post(self.responses_url())
                .bearer_auth(&self.api_key)
                .json(&build_responses_body(messages, options, false))
                .send()
                .await?
                .error_for_status()?;
            let body: Value = resp.json().await?;

            return Ok(CompletionResponse {
                content: response_output_text(&body),
                usage: response_usage(&body).unwrap_or_default(),
            });
        }

        let mut body = super::openwebui::build_body(messages, options);
        if let Some(obj) = body.as_object_mut() {
            obj.insert("stream".to_owned(), serde_json::Value::Bool(false));
        }

        let resp = self
            .client
            .post(self.chat_url())
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await?
            .error_for_status()?;
        let body: ChatCompletion = resp.json().await?;

        Ok(CompletionResponse {
            content: body
                .choices
                .first()
                .and_then(|choice| choice.message.content.clone())
                .unwrap_or_default(),
            usage: body.usage.unwrap_or_default().into(),
        })
    }
}

/// Returns true if `id` is a known chat-capable model that works with
/// either `/v1/chat/completions` or `/v1/responses`. Filters out
/// embeddings, audio, legacy completions-only, and fine-tuned models
/// that the model picker shouldn't show.
fn is_chat_model(id: &str) -> bool {
    let id = id.to_ascii_lowercase();
    // Reject known non-chat prefixes/patterns.
    if id.starts_with("text-embedding")
        || id.starts_with("whisper")
        || id.starts_with("tts")
        || id.starts_with("dall-e")
        || id.starts_with("davinci")
        || id.starts_with("babbage")
        || id.starts_with("curie")
        || id.starts_with("ada")
        || id.contains("instruct")
        || id.starts_with("ft:")
        || id.starts_with("canary-")
        || id.starts_with("codex-")
    {
        return false;
    }
    id.starts_with("gpt-")
        || id.starts_with("o1")
        || id.starts_with("o3")
        || id.starts_with("o4")
        || id.starts_with("chatgpt")
}

fn model_uses_responses(id: &str) -> bool {
    let id = id
        .rsplit('/')
        .next()
        .unwrap_or(id)
        .trim()
        .to_ascii_lowercase();
    id.starts_with("gpt-5") || id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4")
}

pub(crate) fn build_responses_body(
    messages: Vec<ProviderMessage>,
    options: &StreamOptions,
    stream: bool,
) -> Value {
    let mut body = json!({
        "model": options.model.as_str(),
        "input": responses_input(messages),
        "stream": stream,
        "store": false,
    });

    if let Some(system) = &options.system {
        body["instructions"] = json!(system);
    }

    body["max_output_tokens"] = json!(options.max_tokens);

    if !options.tools.is_empty() {
        body["tools"] = json!(options.tools.iter().map(responses_tool).collect::<Vec<_>>());
        body["tool_choice"] = json!("auto");
        body["parallel_tool_calls"] = json!(true);
    }

    if let Some(ref effort) = options.reasoning_effort {
        body["reasoning"] = json!({ "effort": effort });
        body["include"] = json!(["reasoning.encrypted_content"]);
    }
    if let Some(temp) = options.temperature {
        body["temperature"] = Value::from(temp);
    }
    if let Some(top_p) = options.top_p {
        body["top_p"] = Value::from(top_p);
    }
    for (key, value) in &options.provider_options {
        body[key] = value.clone();
    }

    body
}

fn responses_input(messages: Vec<ProviderMessage>) -> Vec<Value> {
    messages
        .into_iter()
        .flat_map(|message| message_items(message.role, message.content))
        .collect()
}

fn message_items(role: ProviderRole, content: Vec<ProviderContent>) -> Vec<Value> {
    let text = content
        .iter()
        .filter_map(|part| match part {
            ProviderContent::Text(text) => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");

    let mut items = if text.is_empty() {
        Vec::new()
    } else {
        vec![json!({
            "type": "message",
            "role": response_role(role),
            "content": [{
                "type": if matches!(role, ProviderRole::Assistant) { "output_text" } else { "input_text" },
                "text": text,
            }],
        })]
    };

    items.extend(content.into_iter().filter_map(|part| match part {
        ProviderContent::ToolUse { id, name, input } => Some(json!({
            "type": "function_call",
            "call_id": id,
            "name": name,
            "arguments": input.to_string(),
        })),
        ProviderContent::ToolResult {
            tool_use_id,
            content,
            ..
        } => Some(json!({
            "type": "function_call_output",
            "call_id": tool_use_id,
            "output": content,
        })),
        _ => None,
    }));

    items
}

fn response_role(role: ProviderRole) -> &'static str {
    match role {
        ProviderRole::User => "user",
        ProviderRole::Assistant => "assistant",
    }
}

fn responses_tool(tool: &ToolDef) -> Value {
    // Responses API uses flat format (NOT the nested `function:{}` wrapper
    // that chat completions uses). See:
    // https://platform.openai.com/docs/api-reference/responses/create
    json!({
        "type": "function",
        "name": tool.name,
        "description": tool.description,
        "parameters": tool.input_schema,
        "strict": false,
    })
}

pub(crate) fn responses_event_stream(resp: reqwest::Response) -> EventStream {
    let event_stream = jfc_anthropic_sdk::sse::response_event_stream(resp)
        .scan((), |_, result| {
            let emitted = match result {
                Ok(event) => responses_events_from_sse(&event.data),
                Err(e) => vec![Err(anyhow::anyhow!("OpenAI SSE stream parse error: {e}"))],
            };
            futures::future::ready(Some(emitted))
        })
        .flat_map(futures::stream::iter);

    Box::pin(event_stream)
}

fn responses_events_from_sse(data: &str) -> Vec<anyhow::Result<StreamEvent>> {
    if data.trim() == "[DONE]" || data.is_empty() {
        return Vec::new();
    }

    let value = match serde_json::from_str::<Value>(data) {
        Ok(value) => value,
        Err(e) => return vec![Err(anyhow::anyhow!("OpenAI SSE JSON parse error: {e}"))],
    };

    match value
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default()
    {
        "response.created" | "response.in_progress" => value
            .get("response")
            .and_then(|response| response.get("id"))
            .and_then(Value::as_str)
            .map(|id| {
                vec![Ok(StreamEvent::ResponseMetadata {
                    response_id: id.to_owned(),
                    input_tokens: None,
                })]
            })
            .unwrap_or_default(),
        "response.output_text.delta" => value
            .get("delta")
            .and_then(Value::as_str)
            .map(|delta| {
                vec![Ok(StreamEvent::TextDelta {
                    index: output_index(&value),
                    delta: delta.to_string(),
                })]
            })
            .unwrap_or_default(),
        "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => value
            .get("delta")
            .and_then(Value::as_str)
            .map(|delta| {
                vec![Ok(StreamEvent::ThinkingDelta {
                    index: output_index(&value),
                    delta: delta.to_string(),
                })]
            })
            .unwrap_or_default(),
        "response.function_call_arguments.delta" => value
            .get("delta")
            .and_then(Value::as_str)
            .map(|delta| {
                vec![Ok(StreamEvent::ToolDelta {
                    index: output_index(&value),
                    delta: delta.to_string(),
                })]
            })
            .unwrap_or_default(),
        "response.output_item.done" => value
            .get("item")
            .filter(|item| item.get("type").and_then(Value::as_str) == Some("function_call"))
            .map(|item| {
                vec![
                    Ok(StreamEvent::ToolDone {
                        index: output_index(&value),
                        tool_name: item
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                        tool_use_id: item
                            .get("call_id")
                            .and_then(Value::as_str)
                            .or_else(|| item.get("id").and_then(Value::as_str))
                            .unwrap_or_default()
                            .to_string(),
                        input_json: item
                            .get("arguments")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_string(),
                    }),
                    Ok(StreamEvent::Done {
                        stop_reason: StopReason::ToolUse,
                    }),
                ]
            })
            .unwrap_or_default(),
        "response.completed" => {
            let mut events = value
                .get("response")
                .and_then(response_usage)
                .map(|usage| {
                    vec![Ok(StreamEvent::Usage {
                        input_tokens: usage.input_tokens as u32,
                        output_tokens: usage.output_tokens as u32,
                        cache_read_tokens: usage.cache_read_tokens as u32,
                        cache_write_tokens: usage.cache_creation_tokens as u32,
                    })]
                })
                .unwrap_or_default();
            if let Some(response_id) = value
                .get("response")
                .and_then(|response| response.get("id"))
                .and_then(Value::as_str)
            {
                events.push(Ok(StreamEvent::ResponseMetadata {
                    input_tokens: None,
                    response_id: response_id.to_owned(),
                }));
            }
            events.push(Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            }));
            events
        }
        "response.failed" | "error" => vec![Ok(StreamEvent::Error {
            message: response_error_message(&value),
        })],
        _ => Vec::new(),
    }
}

fn output_index(value: &Value) -> usize {
    value
        .get("output_index")
        .and_then(Value::as_u64)
        .unwrap_or_default() as usize
}

fn response_error_message(value: &Value) -> String {
    value
        .pointer("/response/error/message")
        .or_else(|| value.pointer("/error/message"))
        .or_else(|| value.get("message"))
        .and_then(Value::as_str)
        .unwrap_or("OpenAI Responses API error")
        .to_string()
}

pub(crate) fn response_output_text(value: &Value) -> String {
    value
        .get("output")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .flat_map(|item| {
            item.get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
        })
        .filter_map(|content| content.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("")
}

pub(crate) fn response_usage(value: &Value) -> Option<TokenUsage> {
    let usage = value.get("usage")?;
    Some(
        ChatUsage {
            prompt_tokens: usage
                .get("input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            completion_tokens: usage
                .get("output_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            prompt_tokens_details: usage
                .get("input_tokens_details")
                .cloned()
                .and_then(|value| serde_json::from_value(value).ok()),
            cache_creation_input_tokens: usage
                .get("cache_creation_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            cache_read_input_tokens: usage
                .get("cache_read_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
            cache_write_input_tokens: usage
                .get("cache_write_input_tokens")
                .and_then(Value::as_u64)
                .unwrap_or_default() as usize,
        }
        .into(),
    )
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<OpenAIModel>,
}

#[derive(Debug, Deserialize)]
struct OpenAIModel {
    id: String,
}

#[derive(Debug, Deserialize)]
struct ChatCompletion {
    choices: Vec<ChatChoice>,
    usage: Option<ChatUsage>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
struct ChatUsage {
    #[serde(default)]
    prompt_tokens: usize,
    #[serde(default)]
    completion_tokens: usize,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    cache_creation_input_tokens: usize,
    #[serde(default)]
    cache_read_input_tokens: usize,
    #[serde(default)]
    cache_write_input_tokens: usize,
}

impl ChatUsage {
    fn raw_input_tokens(&self) -> usize {
        self.prompt_tokens
            .saturating_sub(self.cache_read_tokens())
            .saturating_sub(self.cache_creation_tokens())
    }

    fn cache_read_tokens(&self) -> usize {
        self.cache_read_input_tokens.max(
            self.prompt_tokens_details
                .as_ref()
                .map_or(0, |d| d.cached_tokens),
        )
    }

    fn cache_creation_tokens(&self) -> usize {
        self.cache_creation_input_tokens
            .max(self.cache_write_input_tokens)
            .max(
                self.prompt_tokens_details
                    .as_ref()
                    .map_or(0, |d| d.cache_creation_input_tokens),
            )
    }
}

#[derive(Debug, Default, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: usize,
    #[serde(default)]
    cache_creation_input_tokens: usize,
}

impl From<ChatUsage> for TokenUsage {
    fn from(value: ChatUsage) -> Self {
        Self {
            input_tokens: value.raw_input_tokens(),
            output_tokens: value.completion_tokens,
            cache_read_tokens: value.cache_read_tokens(),
            cache_creation_tokens: value.cache_creation_tokens(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_chat_models_normal() {
        assert!(is_chat_model("gpt-5.1"));
        assert!(is_chat_model("o4-mini"));
        assert!(!is_chat_model("text-embedding-3-large"));
        assert!(!is_chat_model("whisper-1"));
    }

    #[test]
    fn trims_base_url_normal() {
        let provider = OpenAIProvider::new("key", "https://example.test/v1/");
        assert_eq!(
            provider.chat_url(),
            "https://example.test/v1/chat/completions"
        );
        assert_eq!(
            provider.responses_url(),
            "https://example.test/v1/responses"
        );
    }

    #[test]
    fn routes_gpt5_class_models_to_responses() {
        assert!(model_uses_responses("gpt-5.5"));
        assert!(model_uses_responses("openai/gpt-5.1"));
        assert!(model_uses_responses("o4-mini"));
        assert!(!model_uses_responses("gpt-4.1"));
        assert!(!model_uses_responses("gpt-4o"));
    }

    #[test]
    fn builds_responses_body_with_instructions_tools_and_input() {
        let mut options = StreamOptions::new("gpt-5.5")
            .system("be direct")
            .reasoning_effort("high")
            .temperature(0.2)
            .top_p(0.9)
            .tools(vec![ToolDef {
                name: "inspect".to_string(),
                description: "inspect files".to_string(),
                input_schema: json!({ "type": "object" }),
            }]);
        options
            .provider_options
            .insert("metadata".to_string(), json!({ "source": "test" }));

        let body = build_responses_body(
            vec![
                ProviderMessage {
                    role: ProviderRole::User,
                    content: vec![ProviderContent::Text("hello".to_string())],
                },
                ProviderMessage {
                    role: ProviderRole::Assistant,
                    content: vec![ProviderContent::ToolUse {
                        id: "call_1".to_string(),
                        name: "inspect".to_string(),
                        input: json!({ "path": "Cargo.toml" }),
                    }],
                },
                ProviderMessage {
                    role: ProviderRole::User,
                    content: vec![ProviderContent::ToolResult {
                        tool_use_id: "call_1".to_string(),
                        content: "ok".to_string(),
                        is_error: false,
                    }],
                },
            ],
            &options,
            true,
        );

        assert_eq!(body["model"], "gpt-5.5");
        assert_eq!(body["instructions"], "be direct");
        assert_eq!(body["input"][0]["content"][0]["type"], "input_text");
        assert_eq!(body["input"][1]["type"], "function_call");
        assert_eq!(body["input"][2]["type"], "function_call_output");
        assert_eq!(body["tools"][0]["type"], "function");
        assert_eq!(body["tool_choice"], "auto");
        assert_eq!(body["reasoning"]["effort"], "high");
        assert_eq!(body["include"], json!(["reasoning.encrypted_content"]));
        assert_eq!(body["temperature"], 0.2);
        assert_eq!(body["top_p"], 0.9);
        assert_eq!(body["metadata"]["source"], "test");
        assert_eq!(body["stream"], true);
    }

    #[test]
    fn parses_responses_stream_events() {
        let text = responses_events_from_sse(
            r#"{"type":"response.output_text.delta","output_index":0,"delta":"hi"}"#,
        );
        let tool = responses_events_from_sse(
            r#"{"type":"response.output_item.done","output_index":1,"item":{"type":"function_call","call_id":"call_1","name":"inspect","arguments":"{\"path\":\"Cargo.toml\"}"}}"#,
        );
        let done = responses_events_from_sse(
            r#"{"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":2,"input_tokens_details":{"cached_tokens":7}}}}"#,
        );

        assert!(
            matches!(text[0].as_ref().unwrap(), StreamEvent::TextDelta { delta, .. } if delta == "hi")
        );
        assert!(
            matches!(tool[0].as_ref().unwrap(), StreamEvent::ToolDone { tool_name, tool_use_id, .. } if tool_name == "inspect" && tool_use_id == "call_1")
        );
        assert!(matches!(
            tool[1].as_ref().unwrap(),
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse
            }
        ));
        assert!(matches!(
            done[0].as_ref().unwrap(),
            StreamEvent::Usage {
                input_tokens: 3,
                output_tokens: 2,
                cache_read_tokens: 7,
                ..
            }
        ));
        assert!(matches!(
            done[1].as_ref().unwrap(),
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            }
        ));
    }

    #[test]
    fn responses_stream_malformed_json_surfaces_error_robust() {
        let events = responses_events_from_sse("{not json");
        assert_eq!(events.len(), 1);
        assert!(
            events[0]
                .as_ref()
                .unwrap_err()
                .to_string()
                .contains("JSON parse error")
        );
    }
}
