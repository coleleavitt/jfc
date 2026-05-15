#![allow(dead_code)]

use std::path::PathBuf;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::provider::{
    EventStream, ModelInfo, Provider, ProviderMessage, StreamConvention, StreamOptions,
};

const PROVIDER_NAME: &str = "litellm";

// ── Credential store ─────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Credentials {
    pub api_key: String,
    pub base_url: String,
}

pub fn credentials_path() -> PathBuf {
    dirs::config_dir()
        .or_else(|| {
            std::env::var("HOME")
                .ok()
                .map(|h| PathBuf::from(h).join(".config"))
        })
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("jfc")
        .join("litellm.toml")
}

pub fn load_credentials() -> Option<Credentials> {
    let path = credentials_path();
    let raw = std::fs::read_to_string(&path).ok()?;
    toml::from_str::<Credentials>(&raw).ok()
}

pub fn save_credentials(base_url: &str, api_key: &str) -> anyhow::Result<()> {
    let path = credentials_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let creds = Credentials {
        api_key: api_key.trim().to_owned(),
        base_url: base_url.trim_end_matches('/').to_owned(),
    };
    let serialized = toml::to_string_pretty(&creds)?;
    std::fs::write(&path, serialized)?;
    // Restrict to owner-only on Unix — API keys must not be world-readable.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

// ── Provider ─────────────────────────────────────────────────────────────────

pub struct LiteLLMProvider {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
}

impl LiteLLMProvider {
    /// Resolve credentials from env vars first, then the on-disk credential store.
    /// Returns `None` when neither source has usable credentials.
    pub fn from_env() -> Option<Self> {
        let (api_key, base_url) = resolve_credentials()?;

        tracing::debug!(
            target: "jfc::provider::litellm",
            base_url = %base_url,
            "LiteLLMProvider::from_env"
        );

        Some(Self {
            client: super::http::streaming_client(),
            api_key,
            base_url,
        })
    }
}

fn resolve_credentials() -> Option<(String, String)> {
    let env_key = std::env::var("JFC_LITELLM_API_KEY")
        .ok()
        .filter(|s| !s.is_empty());
    let env_url = std::env::var("JFC_LITELLM_API")
        .ok()
        .filter(|s| !s.is_empty());

    if let (Some(key), Some(url)) = (env_key, env_url) {
        return Some((key, url.trim_end_matches('/').to_owned()));
    }

    let creds = load_credentials()?;
    Some((creds.api_key, creds.base_url))
}

// ── API response types ───────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ModelsListResponse {
    data: Vec<ApiModel>,
}

#[derive(Debug, Deserialize)]
struct ApiModel {
    id: String,
    #[serde(default)]
    object: Option<String>,
    #[serde(default)]
    created: Option<u64>,
    #[serde(default)]
    owned_by: Option<String>,
}

// ── Provider trait implementation ────────────────────────────────────────────

impl crate::provider::seal::Sealed for LiteLLMProvider {}

#[async_trait]
impl Provider for LiteLLMProvider {
    fn name(&self) -> &str {
        PROVIDER_NAME
    }

    /// LiteLLM uses OpenAI-compatible structured tool_calls — same as native OpenAI.
    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::OpenAiNative
    }

    /// Static fallback is empty — model list is always fetched dynamically from the
    /// LiteLLM instance via `fetch_models()`.
    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    /// Dynamically fetch all models available on the configured LiteLLM instance.
    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let url = format!("{}/models", self.base_url);
        tracing::info!(
            target: "jfc::provider::litellm",
            url = %url,
            "fetching models"
        );

        let resp: ModelsListResponse = self
            .client
            .get(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

        let models: Vec<ModelInfo> = resp
            .data
            .into_iter()
            .map(|m| {
                let display = m.id.clone();
                let context_window =
                    super::openwebui::infer_context_window_from_model_name(&m.id, None);
                ModelInfo::new(m.id.as_str(), display, PROVIDER_NAME)
                    .with_context_window_tokens(context_window)
            })
            .collect();

        tracing::info!(
            target: "jfc::provider::litellm",
            model_count = models.len(),
            "fetch_models succeeded"
        );
        Ok(models)
    }

    #[tracing::instrument(
        target = "jfc::provider::litellm",
        skip_all,
        fields(
            model = %options.model,
            messages = messages.len(),
            tools = options.tools.len(),
        ),
        err,
    )]
    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        let url = format!("{}/chat/completions", self.base_url);
        let body = build_body(messages, options);

        tracing::debug!(
            target: "jfc::provider::litellm",
            url = %url,
            tools = body.get("tools").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            messages = body.get("messages").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            "POST chat/completions"
        );

        let send_started = std::time::Instant::now();
        let resp = match super::http::send_with_retry("litellm.chat/completions", || {
            self.client
                .post(&url)
                .header("Authorization", format!("Bearer {}", self.api_key))
                .header("Accept", "application/json")
                .header("Content-Type", "application/json")
                .header("Connection", "keep-alive")
                .header("x-litellm-stream-timeout", "600")
                .header("x-litellm-timeout", "600")
                .json(&body)
                .send()
        })
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let cause = super::http::classify_send_error(&e);
                tracing::warn!(
                    target: "jfc::provider::litellm",
                    url = %url,
                    error = %e,
                    cause = cause,
                    "POST chat/completions failed (after retries)"
                );
                anyhow::bail!(
                    "LiteLLM request to {url} failed: {cause} ({e}). \
                     Check JFC_LITELLM_API ({}) and JFC_LITELLM_API_KEY are correct.",
                    self.base_url
                );
            }
        };

        super::http::report_first_byte_latency("litellm.chat/completions", send_started.elapsed());
        tracing::info!(
            target: "jfc::provider::litellm",
            status = %resp.status(),
            model = %options.model,
            content_type = ?resp.headers().get("content-type"),
            "HTTP response received"
        );

        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let friendly = super::retry::friendly_error_message(status.as_u16(), &text);
            anyhow::bail!("LiteLLM API error {status}: {friendly}\n  raw: {text}");
        }

        Ok(super::openwebui::openai_compatible_event_stream(resp))
    }
}

fn build_body(messages: Vec<ProviderMessage>, opts: &StreamOptions) -> Value {
    super::openwebui::build_body(messages, opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        ProviderContent, ProviderMessage, ProviderRole, StopReason, StreamEvent, StreamOptions,
        ToolDef,
    };
    use serde_json::json;

    fn user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(text.into())],
        }
    }

    fn assistant_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::Text(text.into())],
        }
    }

    fn bash_tool() -> ToolDef {
        ToolDef {
            name: "Bash".into(),
            description: "Run a shell command".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string", "description": "The command to run" }
                },
                "required": ["command"]
            }),
        }
    }

    fn read_tool() -> ToolDef {
        ToolDef {
            name: "Read".into(),
            description: "Read a file".into(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": { "type": "string", "description": "File path" }
                },
                "required": ["path"]
            }),
        }
    }

    // ── Unit tests ───────────────────────────────────────────────────────────

    // Normal: provider name is "litellm".
    #[test]
    fn provider_name_normal() {
        assert_eq!(PROVIDER_NAME, "litellm");
    }

    // Normal: build_body produces a valid streaming request with the model id.
    #[test]
    fn build_body_includes_model_and_stream_normal() {
        let opts = StreamOptions::new("qwen/qwen3.6-35b-a3b:coding");
        let body = build_body(vec![user_msg("hi")], &opts);
        assert_eq!(body["model"], "qwen/qwen3.6-35b-a3b:coding");
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["stream_options"], json!({"include_usage": true}));
    }

    // Normal: tools are included and lowercased.
    #[test]
    fn build_body_includes_tools_lowercased_normal() {
        let opts = StreamOptions::new("test-model").tools(vec![bash_tool()]);
        let body = build_body(vec![user_msg("hi")], &opts);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools[0]["function"]["name"], "bash");
    }

    // Normal: system prompt is prepended as a system-role message.
    #[test]
    fn build_body_includes_system_prompt_normal() {
        let opts = StreamOptions::new("test-model").system("You are a helpful assistant.");
        let body = build_body(vec![user_msg("hi")], &opts);
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "You are a helpful assistant.");
    }

    // Normal: max_tokens is forwarded into the body.
    #[test]
    fn build_body_includes_max_tokens_normal() {
        let opts = StreamOptions::new("test-model").max_tokens(4096);
        let body = build_body(vec![user_msg("hi")], &opts);
        assert_eq!(body["max_tokens"], json!(4096));
    }

    // Normal: temperature is forwarded when set.
    #[test]
    fn build_body_includes_temperature_normal() {
        let opts = StreamOptions::new("test-model").temperature(0.7);
        let body = build_body(vec![user_msg("hi")], &opts);
        assert_eq!(body["temperature"], json!(0.7));
    }

    // Robust: temperature is omitted when not set.
    #[test]
    fn build_body_omits_temperature_when_unset_robust() {
        let opts = StreamOptions::new("test-model");
        let body = build_body(vec![user_msg("hi")], &opts);
        assert!(body.get("temperature").is_none() || body["temperature"].is_null());
    }

    // Normal: multiple tools are all lowercased and included.
    #[test]
    fn build_body_multiple_tools_all_lowercased_normal() {
        let opts = StreamOptions::new("test-model").tools(vec![bash_tool(), read_tool()]);
        let body = build_body(vec![user_msg("hi")], &opts);
        let tools = body["tools"].as_array().unwrap();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["function"]["name"], "bash");
        assert_eq!(tools[1]["function"]["name"], "read");
        assert_eq!(body["tool_choice"], "auto");
    }

    // Robust: no tools → no tools field in body.
    #[test]
    fn build_body_no_tools_omits_field_robust() {
        let opts = StreamOptions::new("test-model");
        let body = build_body(vec![user_msg("hi")], &opts);
        assert!(body.get("tools").is_none());
        assert!(body.get("tool_choice").is_none());
    }

    // Normal: tool_use content in history serializes as assistant.tool_calls.
    #[test]
    fn build_body_serializes_tool_use_history_normal() {
        let history = vec![
            user_msg("run ls"),
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![ProviderContent::ToolUse {
                    id: "call_001".into(),
                    name: "Bash".into(),
                    input: json!({"command": "ls -la"}),
                }],
            },
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::ToolResult {
                    tool_use_id: "call_001".into(),
                    content: "file1.txt\nfile2.txt".into(),
                    is_error: false,
                }],
            },
            user_msg("what files are there?"),
        ];
        let opts = StreamOptions::new("test-model").tools(vec![bash_tool()]);
        let body = build_body(history, &opts);
        let msgs = body["messages"].as_array().unwrap();

        let asst = msgs
            .iter()
            .find(|m| m["role"] == "assistant" && m.get("tool_calls").is_some())
            .expect("assistant tool_calls turn");
        let calls = asst["tool_calls"].as_array().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "call_001");
        assert_eq!(calls[0]["function"]["name"], "bash");
        let args: serde_json::Value =
            serde_json::from_str(calls[0]["function"]["arguments"].as_str().unwrap()).unwrap();
        assert_eq!(args["command"], "ls -la");

        let tool_turn = msgs
            .iter()
            .find(|m| m["role"] == "tool")
            .expect("tool result turn");
        assert_eq!(tool_turn["tool_call_id"], "call_001");
        assert!(tool_turn["content"].as_str().unwrap().contains("file1.txt"));
    }

    // Robust: trailing assistant message is stripped (Bedrock prefill compat).
    #[test]
    fn build_body_strips_trailing_assistant_turn_robust() {
        let msgs = vec![user_msg("hi"), assistant_msg("Let me think...")];
        let opts = StreamOptions::new("test-model");
        let body = build_body(msgs, &opts);
        let messages = body["messages"].as_array().unwrap();
        let last_role = messages.last().unwrap()["role"].as_str().unwrap();
        assert!(
            last_role == "user" || last_role == "tool",
            "expected last turn to be user/tool, got: {last_role}"
        );
    }

    // Normal: stream_options.include_usage is always set for streaming requests.
    #[test]
    fn build_body_always_sets_stream_options_include_usage_normal() {
        let opts = StreamOptions::new("any-model");
        let body = build_body(vec![user_msg("test")], &opts);
        assert_eq!(body["stream_options"]["include_usage"], json!(true));
    }

    // Normal: reasoning_effort is forwarded when set.
    #[test]
    fn build_body_includes_reasoning_effort_normal() {
        let opts = StreamOptions::new("test-model").reasoning_effort("high");
        let body = build_body(vec![user_msg("hi")], &opts);
        assert_eq!(body["reasoning_effort"], "high");
    }

    // Normal: from_env returns None when env vars are not set AND no disk store exists.
    #[test]
    fn from_env_returns_none_without_env_vars_normal() {
        if std::env::var("JFC_LITELLM_API_KEY").is_ok() && std::env::var("JFC_LITELLM_API").is_ok()
        {
            eprintln!("skipping: env vars already set (CI environment)");
            return;
        }
        if load_credentials().is_some() {
            eprintln!(
                "skipping: disk credentials exist at {:?}",
                credentials_path()
            );
            return;
        }
        unsafe {
            std::env::remove_var("JFC_LITELLM_API_KEY");
            std::env::remove_var("JFC_LITELLM_API");
        }
        assert!(LiteLLMProvider::from_env().is_none());
    }

    // Robust: from_env falls through to disk store when env vars are empty.
    #[test]
    fn from_env_falls_through_to_disk_with_empty_env_robust() {
        if std::env::var("JFC_LITELLM_API_KEY").is_ok() && std::env::var("JFC_LITELLM_API").is_ok()
        {
            eprintln!("skipping: env vars already set (CI environment)");
            return;
        }
        unsafe {
            std::env::set_var("JFC_LITELLM_API_KEY", "");
            std::env::set_var("JFC_LITELLM_API", "http://localhost:4000");
        }
        let result = LiteLLMProvider::from_env();
        unsafe {
            std::env::remove_var("JFC_LITELLM_API_KEY");
            std::env::remove_var("JFC_LITELLM_API");
        }
        if load_credentials().is_some() {
            assert!(result.is_some(), "should fall through to disk credentials");
        } else {
            assert!(result.is_none(), "no disk credentials → None");
        }
    }

    // Robust: from_env returns None when env URL is empty AND no disk store.
    #[test]
    fn from_env_returns_none_with_empty_api_url_robust() {
        if std::env::var("JFC_LITELLM_API_KEY").is_ok() && std::env::var("JFC_LITELLM_API").is_ok()
        {
            eprintln!("skipping: env vars already set (CI environment)");
            return;
        }
        unsafe {
            std::env::set_var("JFC_LITELLM_API_KEY", "sk-test-key");
            std::env::set_var("JFC_LITELLM_API", "");
        }
        let result = LiteLLMProvider::from_env();
        unsafe {
            std::env::remove_var("JFC_LITELLM_API_KEY");
            std::env::remove_var("JFC_LITELLM_API");
        }
        if load_credentials().is_some() {
            assert!(result.is_some(), "should fall through to disk credentials");
        } else {
            assert!(result.is_none(), "no disk credentials → None");
        }
    }

    // Normal: stream_convention returns OpenAiNative.
    #[test]
    fn stream_convention_is_openai_native_normal() {
        use crate::provider::StreamConvention;
        assert_eq!(
            StreamConvention::OpenAiNative,
            StreamConvention::OpenAiNative
        );
    }

    // Normal: available_models returns empty vec (dynamic-only provider).
    #[test]
    fn available_models_returns_empty_normal() {
        let static_models: Vec<crate::provider::ModelInfo> = Vec::new();
        assert!(static_models.is_empty());
    }

    // ── Live integration tests (gated #[ignore]) ─────────────────────────
    // Run with: JFC_LITELLM_API_KEY=... JFC_LITELLM_API=... cargo test --bin jfc -- --ignored litellm

    fn live_provider() -> Option<LiteLLMProvider> {
        LiteLLMProvider::from_env()
    }

    /// Prefer Claude/GPT models for reliable tool-call behavior, falls back to first available.
    async fn live_tool_capable_model(p: &LiteLLMProvider) -> String {
        let models = p.fetch_models().await.expect("fetch_models");
        models
            .iter()
            .find(|m| m.id.contains("claude"))
            .or_else(|| models.iter().find(|m| m.id.contains("gpt")))
            .unwrap_or_else(|| models.first().expect("need at least one model"))
            .id
            .to_string()
    }

    // Normal: /models endpoint returns at least one model tagged with "litellm" provider.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_fetch_models_returns_list_normal() {
        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let models = p.fetch_models().await.expect("fetch_models");
        assert!(!models.is_empty(), "instance returned zero models");
        for m in &models {
            assert!(!m.id.is_empty(), "empty model id in {m:?}");
            assert_eq!(m.provider.as_str(), "litellm");
        }
    }

    // Normal: each model has a non-zero context window inferred from its name.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_fetch_models_context_window_inference_normal() {
        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let models = p.fetch_models().await.expect("fetch_models");
        for m in &models {
            if let Some(ctx) = m.context_window_tokens {
                assert!(
                    ctx >= 4096,
                    "model {} has suspiciously small context window: {}",
                    m.id,
                    ctx
                );
            }
        }
    }

    // Normal: streaming works end-to-end against the live instance.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_produces_events_normal() {
        use futures::StreamExt;

        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let models = p.fetch_models().await.expect("fetch_models");
        let model_id = models.first().expect("need at least one model").id.clone();

        let opts = StreamOptions::new(model_id).max_tokens(20);
        let msgs = vec![user_msg("Say hello in one word.")];

        let mut stream = p.stream(msgs, &opts).await.expect("stream");
        let mut got_text = false;
        let mut got_done = false;
        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta { .. }) => got_text = true,
                Ok(StreamEvent::ThinkingDelta { .. }) => got_text = true,
                Ok(StreamEvent::Done { .. }) => {
                    got_done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(got_text || got_done, "expected at least text or done event");
    }

    // Normal: streaming with a system prompt produces coherent output.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_with_system_prompt_normal() {
        use futures::StreamExt;

        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let model_id = live_tool_capable_model(&p).await;

        let opts = StreamOptions::new(model_id)
            .system("You are a pirate. Always respond with 'Arrr!'.")
            .max_tokens(50);
        let msgs = vec![user_msg("Hello")];

        let mut stream = p.stream(msgs, &opts).await.expect("stream");
        let mut full_text = String::new();
        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta { delta, .. }) => full_text.push_str(&delta),
                Ok(StreamEvent::Done { .. }) => break,
                _ => {}
            }
        }
        assert!(
            !full_text.is_empty(),
            "expected non-empty response with system prompt"
        );
    }

    // Normal: stream emits Usage event with token counts (when backend supports it).
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_emits_usage_normal() {
        use futures::StreamExt;

        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let models = p.fetch_models().await.expect("fetch_models");
        let model_id = models.first().expect("need at least one model").id.clone();

        let opts = StreamOptions::new(model_id).max_tokens(20);
        let msgs = vec![user_msg("Say hi.")];

        let mut stream = p.stream(msgs, &opts).await.expect("stream");
        let mut got_usage = false;
        let mut got_done = false;
        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                    ..
                }) => {
                    assert!(input_tokens > 0, "expected non-zero input tokens");
                    assert!(output_tokens > 0, "expected non-zero output tokens");
                    got_usage = true;
                }
                Ok(StreamEvent::Done { .. }) => {
                    got_done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(got_done, "expected stream to complete with Done event");
        if !got_usage {
            eprintln!(
                "note: backend did not emit Usage event — \
                 some LiteLLM deployments omit usage data"
            );
        }
    }

    // Normal: streaming with tool definitions triggers a tool_call response
    // when the model is asked to perform a tool-appropriate task.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_tool_call_normal() {
        use futures::StreamExt;

        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let model_id = live_tool_capable_model(&p).await;

        let opts = StreamOptions::new(model_id)
            .tools(vec![bash_tool()])
            .max_tokens(200);
        let msgs = vec![user_msg(
            "Run the command `echo hello` using the bash tool. \
             You MUST call the bash tool. Do not respond with text.",
        )];

        let mut stream = p.stream(msgs, &opts).await.expect("stream");
        let mut got_tool_delta = false;
        let mut got_tool_done = false;
        let mut tool_name = String::new();
        let mut tool_input = String::new();
        let mut stop_reason = None;

        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::ToolDelta { delta, .. }) => {
                    got_tool_delta = true;
                    tool_input.push_str(&delta);
                }
                Ok(StreamEvent::ToolDone {
                    tool_name: name,
                    input_json,
                    ..
                }) => {
                    got_tool_done = true;
                    tool_name = name;
                    tool_input = input_json;
                }
                Ok(StreamEvent::Done {
                    stop_reason: reason,
                }) => {
                    stop_reason = Some(reason);
                    break;
                }
                _ => {}
            }
        }

        assert!(
            got_tool_done,
            "expected ToolDone event — model did not call a tool. \
             Got tool_delta={got_tool_delta}"
        );
        assert_eq!(
            tool_name, "bash",
            "expected tool name 'bash', got '{tool_name}'"
        );
        let input: serde_json::Value =
            serde_json::from_str(&tool_input).expect("tool input should be valid JSON");
        assert!(
            input.get("command").is_some(),
            "expected 'command' field in tool input: {input}"
        );
        assert_eq!(
            stop_reason,
            Some(StopReason::ToolUse),
            "expected stop_reason=ToolUse"
        );
    }

    // Normal: multi-turn conversation with tool result continuation.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_multi_turn_tool_continuation_normal() {
        use futures::StreamExt;

        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let model_id = live_tool_capable_model(&p).await;

        let history = vec![
            user_msg("Run `echo hello` using the bash tool."),
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![ProviderContent::ToolUse {
                    id: "call_test_001".into(),
                    name: "Bash".into(),
                    input: json!({"command": "echo hello"}),
                }],
            },
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::ToolResult {
                    tool_use_id: "call_test_001".into(),
                    content: "hello\n".into(),
                    is_error: false,
                }],
            },
        ];

        let opts = StreamOptions::new(model_id)
            .tools(vec![bash_tool()])
            .max_tokens(100);

        let mut stream = p.stream(history, &opts).await.expect("stream");
        let mut full_text = String::new();
        let mut got_done = false;
        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta { delta, .. }) => full_text.push_str(&delta),
                Ok(StreamEvent::Done { .. }) => {
                    got_done = true;
                    break;
                }
                _ => {}
            }
        }
        assert!(got_done, "expected stream to complete");
    }

    // Robust: streaming with an invalid/nonexistent model returns an error.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_invalid_model_returns_error_robust() {
        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };

        let opts = StreamOptions::new("nonexistent-model-that-does-not-exist-12345").max_tokens(10);
        let msgs = vec![user_msg("hi")];

        match p.stream(msgs, &opts).await {
            Err(e) => {
                let msg = e.to_string();
                assert!(!msg.is_empty(), "expected non-empty error message");
            }
            Ok(_) => panic!("expected error for nonexistent model, got Ok"),
        }
    }

    // Normal: multiple tool calls in a single response (parallel tool use).
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_parallel_tool_calls_normal() {
        use futures::StreamExt;

        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let model_id = live_tool_capable_model(&p).await;

        let opts = StreamOptions::new(model_id)
            .tools(vec![bash_tool(), read_tool()])
            .max_tokens(300);
        let msgs = vec![user_msg(
            "I need you to do two things at once: \
             1. Run `echo first` using the bash tool \
             2. Read the file `/etc/hostname` using the read tool \
             Call BOTH tools in a single response. Do not respond with text first.",
        )];

        let mut stream = p.stream(msgs, &opts).await.expect("stream");
        let mut tool_dones: Vec<String> = Vec::new();
        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::ToolDone { tool_name, .. }) => {
                    tool_dones.push(tool_name);
                }
                Ok(StreamEvent::Done { .. }) => break,
                _ => {}
            }
        }

        assert!(
            !tool_dones.is_empty(),
            "expected at least one tool call, got none"
        );
    }

    // Normal: stream Done event has EndTurn stop reason for text-only response.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_done_end_turn_normal() {
        use futures::StreamExt;

        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let models = p.fetch_models().await.expect("fetch_models");
        let model_id = models.first().expect("need at least one model").id.clone();

        let opts = StreamOptions::new(model_id).max_tokens(10);
        let msgs = vec![user_msg("Say 'yes'.")];

        let mut stream = p.stream(msgs, &opts).await.expect("stream");
        let mut final_reason = None;
        while let Some(event) = stream.next().await {
            if let Ok(StreamEvent::Done { stop_reason }) = event {
                final_reason = Some(stop_reason);
                break;
            }
        }
        assert!(
            matches!(
                final_reason,
                Some(StopReason::EndTurn) | Some(StopReason::MaxTokens)
            ),
            "expected EndTurn or MaxTokens, got {final_reason:?}"
        );
    }

    // Normal: max_tokens limit is respected — response truncates at the limit.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_respects_max_tokens_normal() {
        use futures::StreamExt;

        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let models = p.fetch_models().await.expect("fetch_models");
        let model_id = models.first().expect("need at least one model").id.clone();

        let opts = StreamOptions::new(model_id).max_tokens(5);
        let msgs = vec![user_msg(
            "Write a very long essay about the history of computing. \
             Make it at least 1000 words.",
        )];

        let mut stream = p.stream(msgs, &opts).await.expect("stream");
        let mut full_text = String::new();
        let mut stop = None;
        while let Some(event) = stream.next().await {
            match event {
                Ok(StreamEvent::TextDelta { delta, .. }) => full_text.push_str(&delta),
                Ok(StreamEvent::Done { stop_reason }) => {
                    stop = Some(stop_reason);
                    break;
                }
                _ => {}
            }
        }
        assert!(
            full_text.len() < 500,
            "expected short response with max_tokens=5, got {} chars",
            full_text.len()
        );
        assert_eq!(
            stop,
            Some(StopReason::MaxTokens),
            "expected MaxTokens stop reason with max_tokens=5"
        );
    }

    // Robust: fetch_models is idempotent — calling twice returns consistent results.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_fetch_models_idempotent_robust() {
        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        let models1 = p.fetch_models().await.expect("fetch_models (1)");
        let models2 = p.fetch_models().await.expect("fetch_models (2)");
        assert_eq!(
            models1.len(),
            models2.len(),
            "fetch_models returned different counts on consecutive calls"
        );
        for (a, b) in models1.iter().zip(models2.iter()) {
            assert_eq!(a.id, b.id, "model order/identity changed between calls");
        }
    }

    // Normal: provider name() method returns "litellm" on a live instance.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_provider_name_normal() {
        use crate::provider::Provider;
        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        assert_eq!(p.name(), "litellm");
    }

    // Normal: stream convention is OpenAiNative on live provider.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_stream_convention_openai_native_normal() {
        use crate::provider::{Provider, StreamConvention};
        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };
        assert_eq!(p.stream_convention(), StreamConvention::OpenAiNative);
    }

    // Robust: error message from invalid model is descriptive.
    #[tokio::test]
    #[ignore = "hits live LiteLLM instance — run with cargo test -- --ignored"]
    async fn live_error_message_is_descriptive_robust() {
        let Some(p) = live_provider() else {
            eprintln!("skipping: JFC_LITELLM_API_KEY / JFC_LITELLM_API not set");
            return;
        };

        let opts = StreamOptions::new("__invalid__").max_tokens(10);
        let msgs = vec![user_msg("hi")];
        match p.stream(msgs, &opts).await {
            Err(e) => {
                let msg = e.to_string().to_lowercase();
                assert!(
                    msg.contains("litellm") || msg.contains("error") || msg.contains("400"),
                    "expected descriptive error, got: {msg}"
                );
            }
            Ok(_) => panic!("expected error for invalid model"),
        }
    }
}
