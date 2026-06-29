use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;

use jfc_provider::{
    EventStream, ModelInfo, Provider, ProviderMessage, StreamConvention, StreamOptions,
};

const PROVIDER_NAME: &str = "openrouter";
const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const DEFAULT_REFERER: &str = "https://github.com/coleam00/jfc";
const DEFAULT_TITLE: &str = "jfc";

pub struct OpenRouterProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    referer: String,
    title: String,
}

impl OpenRouterProvider {
    pub fn from_env() -> Option<Self> {
        let api_key = std::env::var("OPENROUTER_API_KEY")
            .ok()
            .filter(|s| !s.trim().is_empty())?
            .trim()
            .to_owned();
        let base_url = std::env::var("OPENROUTER_BASE_URL")
            .unwrap_or_else(|_| DEFAULT_BASE_URL.to_owned())
            .trim_end_matches('/')
            .to_owned();
        let referer =
            std::env::var("OPENROUTER_REFERER").unwrap_or_else(|_| DEFAULT_REFERER.to_owned());
        let title = std::env::var("OPENROUTER_TITLE").unwrap_or_else(|_| DEFAULT_TITLE.to_owned());

        Some(Self {
            client: jfc_provider::http::streaming_client(),
            api_key,
            base_url,
            referer,
            title,
        })
    }

    fn models_url(&self) -> String {
        format!("{}/models", self.base_url)
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url)
    }

    fn request_headers(&self, builder: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        builder
            .bearer_auth(&self.api_key)
            .header("HTTP-Referer", &self.referer)
            .header("X-Title", &self.title)
            .header("Accept", "application/json")
            .header("Content-Type", "application/json")
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ApiModel>,
}

#[derive(Debug, Deserialize)]
struct ApiModel {
    id: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    context_length: Option<usize>,
    #[serde(default)]
    top_provider: Option<TopProvider>,
    #[serde(default)]
    pricing: Option<Pricing>,
}

#[derive(Debug, Deserialize)]
struct TopProvider {
    #[serde(default)]
    context_length: Option<usize>,
    #[serde(default)]
    max_completion_tokens: Option<usize>,
}

#[derive(Debug, Deserialize)]
struct Pricing {
    #[serde(default)]
    prompt: Option<String>,
    #[serde(default)]
    completion: Option<String>,
}

impl jfc_provider::seal::Sealed for OpenRouterProvider {}

#[async_trait]
impl Provider for OpenRouterProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::OpenAiNative
    }

    fn http_client(&self) -> Option<reqwest::Client> {
        Some(self.client.clone())
    }

    fn warmup_url(&self) -> Option<String> {
        // Extract just the origin from the base URL (handles OPENROUTER_BASE_URL overrides).
        reqwest::Url::parse(&self.base_url)
            .ok()
            .map(|u| u.origin().ascii_serialization())
    }

    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let url = self.models_url();
        let resp = match jfc_provider::http::send_with_retry("openrouter.models", || {
            self.request_headers(self.client.get(&url))
                .timeout(std::time::Duration::from_secs(10))
                .send()
        })
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let cause = jfc_provider::http::classify_send_error(&e);
                return Err(jfc_provider::ProviderError::network(
                    PROVIDER_NAME,
                    format!("request failed: {cause} ({e})"),
                )
                .into());
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(jfc_provider::ProviderError::api_status(
                PROVIDER_NAME,
                status.as_u16(),
                text,
            )
            .into());
        }
        let resp: ModelsResponse = resp.json().await?;

        let mut models: Vec<ModelInfo> = resp
            .data
            .into_iter()
            .map(|m| {
                let display = m.name.unwrap_or_else(|| m.id.clone());
                let context = m
                    .context_length
                    .or_else(|| m.top_provider.as_ref().and_then(|p| p.context_length))
                    .unwrap_or_else(|| {
                        super::openwebui::infer_context_window_from_model_name(
                            &m.id,
                            Some(&display),
                        )
                    });
                let output = m
                    .top_provider
                    .as_ref()
                    .and_then(|p| p.max_completion_tokens);
                let (input_cost, output_cost) = m.pricing.map(parse_costs).unwrap_or((None, None));

                ModelInfo::new(m.id, display, PROVIDER_NAME)
                    .with_context_window_tokens(context)
                    .with_max_output_tokens(output)
                    .with_costs(input_cost, output_cost)
            })
            .collect();
        models.sort_by(|a, b| a.id.as_str().cmp(b.id.as_str()));
        Ok(models)
    }

    #[tracing::instrument(
        target = "jfc::provider::openrouter",
        skip_all,
        fields(model = %options.model, messages = messages.len(), tools = options.tools.len()),
        err,
    )]
    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        let url = self.chat_url();
        let body = openrouter_body(messages, options);
        let send_started = std::time::Instant::now();
        let resp = match jfc_provider::http::send_with_retry("openrouter.chat/completions", || {
            self.request_headers(self.client.post(&url))
                .json(&body)
                .send()
        })
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let cause = jfc_provider::http::classify_send_error(&e);
                return Err(jfc_provider::ProviderError::network(
                    PROVIDER_NAME,
                    format!("request to {url} failed: {cause} ({e})"),
                )
                .into());
            }
        };

        jfc_provider::http::report_first_byte_latency(
            "openrouter.chat/completions",
            send_started.elapsed(),
        );

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(jfc_provider::ProviderError::api_status(
                PROVIDER_NAME,
                status.as_u16(),
                text,
            )
            .into());
        }

        Ok(super::openwebui::openai_compatible_event_stream(resp))
    }
}

fn openrouter_body(messages: Vec<ProviderMessage>, opts: &StreamOptions) -> Value {
    let mut body = super::openwebui::build_body(messages, opts);
    if let Some(obj) = body.as_object_mut() {
        obj.insert(
            "provider".to_owned(),
            serde_json::json!({
                "allow_fallbacks": true,
            }),
        );
    }
    body
}

fn parse_costs(pricing: Pricing) -> (Option<f64>, Option<f64>) {
    // OpenRouter reports per-token USD strings. JFC ModelInfo stores the same
    // per-token unit used by models.dev, so no million-token conversion here.
    (
        pricing.prompt.and_then(|s| s.parse::<f64>().ok()),
        pricing.completion.and_then(|s| s.parse::<f64>().ok()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_costs_accepts_openrouter_decimal_strings() {
        let (input, output) = parse_costs(Pricing {
            prompt: Some("0.000000003".to_owned()),
            completion: Some("0.000000015".to_owned()),
        });
        assert_eq!(input, Some(0.000000003));
        assert_eq!(output, Some(0.000000015));
    }
}
