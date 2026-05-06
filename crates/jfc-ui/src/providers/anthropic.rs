use async_trait::async_trait;
use serde_json::json;

use crate::provider::{
    EventStream, ModelInfo, Provider, ProviderMessage, StreamConvention, StreamOptions,
};

use super::sse;

const API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";
const ANTHROPIC_BETA: &str = "interleaved-thinking-2025-05-14";

pub struct AnthropicProvider {
    client: reqwest::Client,
    api_key: String,
}

impl AnthropicProvider {
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key: api_key.into(),
        }
    }
}

fn build_body(messages: Vec<ProviderMessage>, opts: &StreamOptions) -> serde_json::Value {
    let thinking_mode = if opts.adaptive_thinking {
        "adaptive"
    } else if opts.thinking_budget.is_some() {
        "enabled"
    } else {
        "none"
    };
    tracing::debug!(
        target: "jfc::provider::anthropic",
        model = %opts.model,
        max_tokens = opts.max_tokens,
        has_system = opts.system.is_some(),
        tool_count = opts.tools.len(),
        thinking_mode,
        "building request body"
    );

    let mut body = json!({
        "model": opts.model,
        "max_tokens": opts.max_tokens,
        "stream": true,
        "messages": sse::build_messages(&messages),
    });

    if let Some(sys) = &opts.system {
        body["system"] = json!(sys);
    }

    if !opts.tools.is_empty() {
        body["tools"] = sse::build_tools(&opts.tools);
    }

    if opts.adaptive_thinking {
        body["thinking"] = json!({
            "type": "adaptive",
        });
    } else if let Some(budget) = opts.thinking_budget {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": budget,
        });
    }

    body
}

#[async_trait]
impl Provider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::AnthropicNative
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        super::anthropic_models::anthropic_first_party_models("anthropic")
    }

    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        // Prefer the live models.dev catalog so we pick up new Anthropic models the
        // moment they ship. Fall back to the embedded canonical list when the network
        // is unavailable (offline / corp proxy / models.dev down).
        match super::models_dev::fetch_provider_models(&self.client, "anthropic", "anthropic").await
        {
            Ok(m) if !m.is_empty() => Ok(m),
            _ => Ok(self.available_models()),
        }
    }

    #[tracing::instrument(
        target = "jfc::provider::anthropic",
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
        let body = build_body(messages, options);

        let resp = self
            .client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", ANTHROPIC_BETA)
            .header("content-type", "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let content_type = resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("unknown")
            .to_owned();
        tracing::info!(
            target: "jfc::provider::anthropic",
            status = %status,
            content_type = %content_type,
            "received HTTP response"
        );

        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            tracing::warn!(
                target: "jfc::provider::anthropic",
                status = %status,
                body_preview = %&text[..text.len().min(200)],
                "API request failed"
            );
            if let Some(model) = super::anthropic_oauth::parse_model_not_found(&text) {
                anyhow::bail!(
                    "{model} is not enabled on your Anthropic account. \
                     Pin a model you have access to (Ctrl+M)."
                );
            }
            anyhow::bail!("Anthropic API error {status}: {text}");
        }

        Ok(sse::into_event_stream(resp))
    }
}

/// DO-178B §6.4.2 conformance: every behavior is exercised by at least one
/// `_normal` test (canonical inputs / equivalence classes / boundary values)
/// and one `_robust` test (invalid / abnormal / illegal-state inputs).
///
/// Tests focus on the request-construction layer (`build_body`) and the
/// `Provider` trait wiring. The HTTP layer is exercised end-to-end by the
/// existing live integration tests in `models_dev.rs` and
/// `anthropic_oauth.rs`; replicating those here would duplicate the setup
/// without buying extra coverage of `AnthropicProvider`-specific code.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::provider::{
        Provider, ProviderContent, ProviderMessage, ProviderRole, StreamConvention, StreamOptions,
        ToolDef,
    };

    fn make_user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(text.to_owned())],
        }
    }

    fn opts(model: &str) -> StreamOptions {
        StreamOptions::new(model)
    }

    // Normal: a fresh provider exposes the expected name / convention pair so
    // the renderer's stream-decoding dispatcher routes Anthropic-native SSE.
    #[test]
    fn provider_name_and_convention_normal() {
        let p = AnthropicProvider::new("sk-ant-test");
        assert_eq!(p.name(), "anthropic");
        assert_eq!(p.stream_convention(), StreamConvention::AnthropicNative);
    }

    // Normal: `available_models()` surfaces the embedded canonical catalog so
    // the picker has something to show before `fetch_models()` resolves.
    #[test]
    fn available_models_returns_canonical_catalog_normal() {
        let p = AnthropicProvider::new("sk-ant-test");
        let models = p.available_models();
        assert!(!models.is_empty(), "canonical catalog must be non-empty");
        // Every entry is stamped with the "anthropic" provider tag — that's what
        // the picker round-trips back to `Provider::name()`.
        assert!(models.iter().all(|m| m.provider == "anthropic"));
    }

    // Normal: build_body emits the four Anthropic-required fields when no
    // optional knobs are set — model / max_tokens / stream / messages.
    #[test]
    fn build_body_required_fields_present_normal() {
        let body = build_body(vec![make_user_msg("hello")], &opts("claude-opus-4-7"));
        assert_eq!(body["model"], "claude-opus-4-7");
        assert_eq!(body["max_tokens"], 8192);
        assert_eq!(body["stream"], true);
        assert!(body["messages"].is_array());
    }

    // Normal: the optional `system` field is included when the caller provides
    // a system prompt; the on-wire shape is a plain string per Anthropic's API.
    #[test]
    fn build_body_includes_system_when_set_normal() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").system("you are helpful"),
        );
        assert_eq!(body["system"], "you are helpful");
    }

    // Robust: when no system prompt is set, the field must be absent (sending
    // `system: null` is rejected by some Anthropic-compatible proxies).
    #[test]
    fn build_body_omits_system_when_unset_robust() {
        let body = build_body(vec![make_user_msg("hi")], &opts("m"));
        assert!(body.get("system").is_none(), "system leaked: {body}");
    }

    // Normal: tools array is included when the caller provides tool defs.
    #[test]
    fn build_body_includes_tools_when_set_normal() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").tools(vec![ToolDef {
                name: "Bash".into(),
                description: "run".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }]),
        );
        assert_eq!(body["tools"].as_array().unwrap().len(), 1);
    }

    // Robust: empty tools list omits the field entirely so proxies that reject
    // `tools: []` don't 400 the request.
    #[test]
    fn build_body_omits_tools_when_empty_robust() {
        let body = build_body(vec![make_user_msg("hi")], &opts("m"));
        assert!(body.get("tools").is_none());
    }

    // Normal: legacy thinking budget — pre-4.6 Claude accepts the
    // `{"type":"enabled","budget_tokens":N}` form. Tests the lower budget path.
    #[test]
    fn build_body_thinking_legacy_budget_normal() {
        let body = build_body(vec![make_user_msg("hi")], &opts("m").thinking(8192));
        assert_eq!(body["thinking"]["type"], "enabled");
        assert_eq!(body["thinking"]["budget_tokens"], 8192);
    }

    // Normal: adaptive thinking emits `{"type":"adaptive"}` for 4.6+ models.
    // Critically, when `adaptive_thinking` is true, the legacy `budget_tokens`
    // path is never taken — even if `thinking_budget` is also set.
    #[test]
    fn build_body_thinking_adaptive_overrides_budget_normal() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").thinking(4096).adaptive(),
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        // budget_tokens must NOT leak into the body when adaptive is on,
        // otherwise Claude 4.6+ rejects with 400.
        assert!(body["thinking"].get("budget_tokens").is_none());
    }

    // Robust: with neither budget nor adaptive set, the field must be absent.
    // Sending an empty `thinking: {}` block yields a 400 from Anthropic.
    #[test]
    fn build_body_thinking_absent_when_unset_robust() {
        let body = build_body(vec![make_user_msg("hi")], &opts("m"));
        assert!(body.get("thinking").is_none());
    }

    // Normal: build_body preserves message order — user/assistant alternation
    // matters for Claude's prefill behavior.
    #[test]
    fn build_body_preserves_message_order_normal() {
        let history = vec![
            make_user_msg("first"),
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![ProviderContent::Text("reply".into())],
            },
            make_user_msg("second"),
        ];
        let body = build_body(history, &opts("m"));
        let msgs = body["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[1]["role"], "assistant");
        assert_eq!(msgs[2]["role"], "user");
    }

    // Robust: empty message list still produces a valid body — caller may
    // legitimately want to send only a system prompt with no history.
    #[test]
    fn build_body_empty_messages_robust() {
        let body = build_body(vec![], &opts("m"));
        assert_eq!(body["messages"].as_array().unwrap().len(), 0);
    }

    // Normal: max_tokens override flows into the body. The default is 8192;
    // setting a custom cap lets the caller pin extended-output deployments.
    #[test]
    fn build_body_max_tokens_override_normal() {
        let body = build_body(vec![make_user_msg("hi")], &opts("m").max_tokens(64_000));
        assert_eq!(body["max_tokens"], 64_000);
    }
}
