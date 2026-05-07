use async_trait::async_trait;
use serde::Deserialize;

use crate::provider::{
    CompletionResponse, EventStream, ModelId, ModelInfo, Provider, ProviderId, ProviderMessage,
    StreamConvention, StreamOptions, TokenUsage,
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
        let api_key = std::env::var("OPENAI_API_KEY").ok()?.trim().to_owned();
        if api_key.is_empty() {
            return None;
        }

        Some(Self::new(
            api_key,
            std::env::var("OPENAI_BASE_URL").unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned()),
        ))
    }

    pub fn new(api_key: impl Into<String>, base_url: impl Into<String>) -> Self {
        Self {
            client: super::http::streaming_client(),
            api_key: api_key.into(),
            base_url: base_url.into().trim_end_matches('/').to_owned(),
        }
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
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
        let body = super::openwebui::build_body(messages, options);
        let url = self.chat_url();
        let send_started = std::time::Instant::now();
        let resp = match super::http::send_with_retry("openai.chat/completions", || {
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
                let cause = super::http::classify_send_error(&e);
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
        super::http::report_first_byte_latency(
            "openai.chat/completions",
            send_started.elapsed(),
        );
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let friendly = super::retry::friendly_error_message(status.as_u16(), &text);
            anyhow::bail!("OpenAI API error {status}: {friendly}\n  raw: {text}");
        }
        Ok(super::openwebui::openai_compatible_event_stream(resp))
    }

    async fn complete(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<CompletionResponse> {
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

fn is_chat_model(id: &str) -> bool {
    id.starts_with("gpt-") || id.starts_with("o1") || id.starts_with("o3") || id.starts_with("o4")
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
    prompt_tokens: usize,
    completion_tokens: usize,
}

impl From<ChatUsage> for TokenUsage {
    fn from(value: ChatUsage) -> Self {
        Self {
            input_tokens: value.prompt_tokens,
            output_tokens: value.completion_tokens,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
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
    }
}
