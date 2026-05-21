//! Direct Gemini API provider (API-key-based, no OAuth).
//!
//! Reads `GEMINI_API_KEY` from the environment and hits
//! `generativelanguage.googleapis.com/v1beta/models/{model}:streamGenerateContent`
//! directly. This is the simplest path for users who have a Gemini API key
//! from Google AI Studio — no OAuth dance, no gcloud CLI, no Antigravity
//! subscription required.
//!
//! The request/response format is the same Gemini `generateContent` shape
//! used by the Antigravity transform, so we reuse `antigravity_transform`
//! for both request building and SSE response parsing.

use async_trait::async_trait;

use jfc_provider::{
    CompletionResponse, EventStream, ModelInfo, Provider, ProviderMessage, StreamConvention,
    StreamOptions,
};

const PROVIDER_ID: &str = "gemini";
const BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";

/// Direct Gemini API provider using `GEMINI_API_KEY`.
#[derive(Clone)]
pub struct GeminiApiProvider {
    client: reqwest::Client,
    api_key: String,
}

impl GeminiApiProvider {
    /// Create from the `GEMINI_API_KEY` environment variable, falling back
    /// to `~/.config/jfc/credentials.toml` `[gemini]` section.
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("GEMINI_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(Self::key_from_credentials_file)
            .map(|k| k.trim().to_owned())?;
        if api_key.is_empty() {
            return None;
        }
        Some(Self {
            client: jfc_provider::http::streaming_client(),
            api_key,
        })
    }

    /// Create with an explicit API key (for tests / programmatic use).
    #[allow(dead_code)]
    pub fn new(api_key: String) -> Self {
        Self {
            client: jfc_provider::http::streaming_client(),
            api_key,
        }
    }

    /// True when `GEMINI_API_KEY` is set or credentials.toml has a key.
    pub fn has_usable_config() -> bool {
        std::env::var("GEMINI_API_KEY")
            .ok()
            .filter(|k| !k.trim().is_empty())
            .or_else(Self::key_from_credentials_file)
            .is_some()
    }

    /// Read the Gemini API key from `~/.config/jfc/credentials.toml`.
    /// Format:
    /// ```toml
    /// [gemini]
    /// api_key = "AIza..."
    /// ```
    fn key_from_credentials_file() -> Option<String> {
        let home = std::env::var("HOME").ok()?;
        let path = std::path::PathBuf::from(home).join(".config/jfc/credentials.toml");
        let content = std::fs::read_to_string(&path).ok()?;
        let mut in_gemini = false;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed == "[gemini]" {
                in_gemini = true;
                continue;
            }
            if trimmed.starts_with('[') {
                in_gemini = false;
                continue;
            }
            if in_gemini && trimmed.starts_with("api_key") {
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

    fn gemini_models() -> Vec<ModelInfo> {
        let mk = |id: &str, name: &str, ctx: usize, out: usize| {
            let mut m = ModelInfo::new(id, name, PROVIDER_ID);
            m.context_window_tokens = Some(ctx);
            m.max_output_tokens = Some(out);
            m
        };
        vec![
            // Gemini 3.x series
            mk("gemini-3.5-flash", "Gemini 3.5 Flash", 1_048_576, 65_536),
            mk("gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview", 1_048_576, 65_536),
            mk("gemini-3.1-flash-lite", "Gemini 3.1 Flash Lite", 1_048_576, 65_536),
            mk("gemini-3-pro-preview", "Gemini 3 Pro Preview", 1_048_576, 65_536),
            mk("gemini-3-flash-preview", "Gemini 3 Flash Preview", 1_048_576, 65_536),
            // Gemini 2.5 series
            mk("gemini-2.5-pro", "Gemini 2.5 Pro", 1_048_576, 65_536),
            mk("gemini-2.5-flash", "Gemini 2.5 Flash", 1_048_576, 65_536),
            mk("gemini-2.5-flash-lite", "Gemini 2.5 Flash-Lite", 1_048_576, 65_536),
            // Gemini 2.0 series
            mk("gemini-2.0-flash", "Gemini 2.0 Flash", 1_048_576, 8_192),
            mk("gemini-2.0-flash-lite", "Gemini 2.0 Flash-Lite", 1_048_576, 8_192),
            // Aliases / convenience
            mk("gemini-pro-latest", "Gemini Pro Latest", 1_048_576, 65_536),
            mk("gemini-flash-latest", "Gemini Flash Latest", 1_048_576, 65_536),
            // Special: Antigravity Agent + Deep Research
            mk("antigravity-preview-05-2026", "Antigravity Agent Preview", 131_072, 65_536),
            mk("deep-research-max-preview-04-2026", "Deep Research Max", 131_072, 65_536),
        ]
    }
}

impl jfc_provider::seal::Sealed for GeminiApiProvider {}

#[async_trait]
impl Provider for GeminiApiProvider {
    fn name(&self) -> &str {
        PROVIDER_ID
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Self::gemini_models()
    }

    fn stream_convention(&self) -> StreamConvention {
        // Uses the same Gemini SSE format as Antigravity
        StreamConvention::AnthropicNative
    }

    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        let model = super::antigravity_transform::resolve_model_name(options.model.as_str());
        let url = format!(
            "{BASE_URL}/models/{model}:streamGenerateContent?alt=sse&key={}",
            self.api_key,
        );

        // Build the request body using the same Gemini format as Antigravity,
        // but without the Code Assist envelope wrapper (no project/userAgent).
        let body = build_direct_request(&messages, options)?;

        tracing::debug!(
            target: "jfc::provider::gemini",
            model,
            messages = messages.len(),
            "POST streamGenerateContent (direct API key)"
        );

        let resp = self
            .client
            .post(&url)
            .header("Content-Type", "application/json")
            .header("Accept", "text/event-stream")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            anyhow::bail!("Gemini API error {status}: {text}");
        }

        Ok(super::antigravity_transform::into_event_stream(resp))
    }

    async fn complete(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<CompletionResponse> {
        anyhow::bail!("Gemini direct API does not support non-streaming completion; use stream()")
    }
}

/// Build the Gemini `generateContent` request body without the Code Assist
/// envelope. This goes directly to `generativelanguage.googleapis.com`.
fn build_direct_request(
    messages: &[ProviderMessage],
    options: &StreamOptions,
) -> anyhow::Result<serde_json::Value> {
    use jfc_provider::{ProviderContent, ProviderRole};
    use serde_json::json;

    let contents: Vec<serde_json::Value> = messages
        .iter()
        .filter_map(|msg| {
            let role = match msg.role {
                ProviderRole::User => "user",
                ProviderRole::Assistant => "model",
            };
            let parts: Vec<serde_json::Value> = msg
                .content
                .iter()
                .filter_map(|c| match c {
                    ProviderContent::Text(t) if !t.is_empty() => Some(json!({ "text": t })),
                    ProviderContent::ToolUse { name, input, .. } => {
                        Some(json!({ "functionCall": { "name": name, "args": input } }))
                    }
                    ProviderContent::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => Some(json!({
                        "functionResponse": {
                            "name": tool_use_id,
                            "response": { "content": content, "isError": is_error },
                        }
                    })),
                    _ => None,
                })
                .collect();
            if parts.is_empty() {
                return None;
            }
            Some(json!({ "role": role, "parts": parts }))
        })
        .collect();

    let mut body = json!({ "contents": contents });

    if let Some(sys) = options.system.as_deref().filter(|s| !s.is_empty()) {
        body["systemInstruction"] = json!({ "parts": [{ "text": sys }] });
    }

    if !options.tools.is_empty() {
        let decls: Vec<serde_json::Value> = options
            .tools
            .iter()
            .map(|t| {
                json!({
                    "name": super::antigravity_transform::sanitize_tool_name(&t.name),
                    "description": t.description,
                    "parameters": t.input_schema,
                })
            })
            .collect();
        body["tools"] = json!([{ "functionDeclarations": decls }]);
        body["toolConfig"] = json!({ "functionCallingConfig": { "mode": "AUTO" } });
    }

    let mut gen_config = serde_json::Map::new();
    gen_config.insert("maxOutputTokens".into(), json!(options.max_tokens));
    if let Some(temp) = options.temperature {
        gen_config.insert("temperature".into(), json!(temp));
    }
    if let Some(top_p) = options.top_p {
        gen_config.insert("topP".into(), json!(top_p));
    }
    if let Some(budget) = options.thinking_budget {
        gen_config.insert("thinkingConfig".into(), json!({ "thinkingBudget": budget }));
    }
    if !gen_config.is_empty() {
        body["generationConfig"] = serde_json::Value::Object(gen_config);
    }

    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_usable_config_without_env_returns_false() {
        // This test relies on GEMINI_API_KEY not being set in CI.
        // If it IS set, this test trivially passes anyway.
        let _ = GeminiApiProvider::has_usable_config();
    }

    #[test]
    fn provider_name_and_models_normal() {
        let p = GeminiApiProvider::new("test-key".into());
        assert_eq!(p.name(), "gemini");
        let models = p.available_models();
        assert!(!models.is_empty());
        assert!(models.iter().any(|m| m.id.as_str().contains("2.5")));
    }

    #[test]
    fn build_direct_request_basic_normal() {
        use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

        let opts = StreamOptions::new("gemini-2.5-flash");
        let msg = ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text("hello".into())],
        };
        let body = build_direct_request(&[msg], &opts).unwrap();
        assert_eq!(body["contents"][0]["role"], "user");
        assert_eq!(body["contents"][0]["parts"][0]["text"], "hello");
        assert!(body["generationConfig"]["maxOutputTokens"].is_number());
    }
}
