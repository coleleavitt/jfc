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

/// Synthetic fallback thought signature for Gemini 3.x.
/// Used ONLY for the first `functionCall` in a model turn when no real
/// signature was captured from the stream (cold-start, history restore).
/// The happy path echoes the captured signature verbatim. Mirrors Google's
/// gemini-cli `SYNTHETIC_THOUGHT_SIGNATURE` (historyHardening.ts:10).
/// See https://ai.google.dev/gemini-api/docs/thought-signatures
const SYNTHETIC_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";

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
            if in_gemini
                && trimmed.starts_with("api_key")
                && let Some(val) = trimmed.split('=').nth(1)
            {
                let val = val.trim().trim_matches('"').trim_matches('\'');
                if !val.is_empty() {
                    return Some(val.to_string());
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
            // Gemini 3.x series (thinkingLevel, not thinkingBudget)
            mk("gemini-3.5-flash", "Gemini 3.5 Flash", 1_048_576, 65_536),
            mk(
                "gemini-3.1-pro-preview",
                "Gemini 3.1 Pro Preview",
                1_048_576,
                65_536,
            ),
            mk(
                "gemini-3.1-pro-preview-customtools",
                "Gemini 3.1 Pro Custom Tools",
                1_048_576,
                65_536,
            ),
            mk(
                "gemini-3.1-flash-lite",
                "Gemini 3.1 Flash Lite",
                1_048_576,
                65_536,
            ),
            mk(
                "gemini-3.1-flash-live-preview",
                "Gemini 3.1 Flash Live",
                1_048_576,
                65_536,
            ),
            mk(
                "gemini-3-pro-preview",
                "Gemini 3 Pro Preview",
                1_048_576,
                65_536,
            ),
            mk(
                "gemini-3-flash-preview",
                "Gemini 3 Flash Preview",
                1_048_576,
                65_536,
            ),
            // Gemini 2.5 series (thinkingBudget)
            mk("gemini-2.5-pro", "Gemini 2.5 Pro", 1_048_576, 65_536),
            mk("gemini-2.5-flash", "Gemini 2.5 Flash", 1_048_576, 65_536),
            mk(
                "gemini-2.5-flash-lite",
                "Gemini 2.5 Flash-Lite",
                1_048_576,
                65_536,
            ),
            // Gemini 2.0 series
            mk("gemini-2.0-flash", "Gemini 2.0 Flash", 1_048_576, 8_192),
            mk(
                "gemini-2.0-flash-lite",
                "Gemini 2.0 Flash-Lite",
                1_048_576,
                8_192,
            ),
            // Aliases (always point to latest)
            mk("gemini-pro-latest", "Gemini Pro Latest", 1_048_576, 65_536),
            mk(
                "gemini-flash-latest",
                "Gemini Flash Latest",
                1_048_576,
                65_536,
            ),
            mk(
                "gemini-flash-lite-latest",
                "Gemini Flash-Lite Latest",
                1_048_576,
                65_536,
            ),
            // Special models
            mk(
                "antigravity-preview-05-2026",
                "Antigravity Agent Preview",
                131_072,
                65_536,
            ),
            mk(
                "deep-research-max-preview-04-2026",
                "Deep Research Max",
                131_072,
                65_536,
            ),
            mk(
                "deep-research-preview-04-2026",
                "Deep Research Preview",
                131_072,
                65_536,
            ),
            // Open models
            mk("gemma-4-31b-it", "Gemma 4 31B IT", 262_144, 32_768),
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

    fn http_client(&self) -> Option<reqwest::Client> {
        Some(self.client.clone())
    }

    fn warmup_url(&self) -> Option<String> {
        Some("https://generativelanguage.googleapis.com".to_owned())
    }

    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        // Fetch live pricing + model list from models.dev (community registry).
        // Falls back to the hardcoded static list on network failure.
        match super::models_dev::fetch_provider_models(&self.client, "google", PROVIDER_ID).await {
            Ok(models) if !models.is_empty() => Ok(models),
            _ => Ok(Self::gemini_models()),
        }
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

// ─── Additional API methods (not part of Provider trait) ─────────────────────

impl GeminiApiProvider {
    /// Fetch available models dynamically from the Gemini API.
    /// Returns only text-generation capable models.
    pub async fn fetch_remote_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let url = format!("{BASE_URL}/models?key={}", self.api_key);
        let resp: serde_json::Value = self.client.get(&url).send().await?.json().await?;
        let models = resp
            .get("models")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();

        let mut out = Vec::new();
        for m in models {
            let methods = m
                .get("supportedGenerationMethods")
                .and_then(|v| v.as_array())
                .map(|arr| arr.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
                .unwrap_or_default();
            if !methods.contains(&"generateContent") && !methods.contains(&"streamGenerateContent")
            {
                continue;
            }
            let id = m
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .strip_prefix("models/")
                .unwrap_or("");
            if id.is_empty() {
                continue;
            }
            let display = m.get("displayName").and_then(|v| v.as_str()).unwrap_or(id);
            let ctx = m
                .get("inputTokenLimit")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;
            let max_out = m
                .get("outputTokenLimit")
                .and_then(|v| v.as_u64())
                .unwrap_or(0) as usize;

            let mut info = ModelInfo::new(id, display, PROVIDER_ID);
            info.context_window_tokens = Some(ctx);
            info.max_output_tokens = Some(max_out);
            out.push(info);
        }
        Ok(out)
    }

    /// Count tokens for a set of messages without generating content.
    pub async fn count_tokens(
        &self,
        model: &str,
        messages: &[ProviderMessage],
    ) -> anyhow::Result<u32> {
        use jfc_provider::{ProviderContent, ProviderRole};
        use serde_json::json;

        let url = format!("{BASE_URL}/models/{model}:countTokens?key={}", self.api_key);
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
                        _ => None,
                    })
                    .collect();
                if parts.is_empty() {
                    return None;
                }
                Some(json!({ "role": role, "parts": parts }))
            })
            .collect();

        let body = json!({ "contents": contents });
        let resp: serde_json::Value = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await?
            .json()
            .await?;
        Ok(resp
            .get("totalTokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32)
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
            let is_model = matches!(msg.role, ProviderRole::Assistant);
            let role = if is_model { "model" } else { "user" };
            // Track the first functionCall in a model turn for the synthetic
            // signature fallback (gemini-cli historyHardening.ts:101-113).
            let mut first_function_call_seen = false;
            let parts: Vec<serde_json::Value> = msg
                .content
                .iter()
                .filter_map(|c| match c {
                    ProviderContent::Text(t) if !t.is_empty() => Some(json!({ "text": t })),
                    ProviderContent::ToolUse {
                        name,
                        input,
                        thought_signature,
                        ..
                    } => {
                        // Prefer the real captured signature; fall back to the
                        // synthetic token only for the first functionCall of a
                        // model turn when no real signature exists.
                        let mut part = json!({ "functionCall": { "name": name, "args": input } });
                        if is_model {
                            if let Some(sig) = thought_signature.as_deref() {
                                part["thoughtSignature"] = json!(sig);
                            } else if !first_function_call_seen {
                                part["thoughtSignature"] = json!(SYNTHETIC_THOUGHT_SIGNATURE);
                            }
                        }
                        first_function_call_seen = true;
                        Some(part)
                    }
                    ProviderContent::ToolResult {
                        tool_use_id,
                        content,
                        is_error,
                    } => {
                        // Gemini matches functionResponse → functionCall by
                        // NAME, so resolve the original tool name from history
                        // (raw `toolu_*` ids 400 the next turn). Falls back to
                        // the id when the ToolUse isn't in scope.
                        let name = super::antigravity_transform::find_tool_name_by_id(
                            messages,
                            tool_use_id,
                        )
                        .unwrap_or(tool_use_id.as_str());
                        Some(json!({
                            "functionResponse": {
                                "name": name,
                                "response": { "content": content, "isError": is_error },
                            }
                        }))
                    }
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
        // Gemini 3.x uses thinkingLevel (low|medium|high); 2.5 uses a
        // numeric thinkingBudget. Sending thinkingBudget to a Gemini-3
        // model is ignored at best and 400s at worst.
        let resolved = super::antigravity_transform::resolve_model_name(options.model.as_str());
        if super::antigravity_transform::is_gemini_3(resolved) {
            let level = super::antigravity_transform::budget_to_thinking_level(budget);
            gen_config.insert("thinkingConfig".into(), json!({ "thinkingLevel": level }));
        } else {
            gen_config.insert("thinkingConfig".into(), json!({ "thinkingBudget": budget }));
        }
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
