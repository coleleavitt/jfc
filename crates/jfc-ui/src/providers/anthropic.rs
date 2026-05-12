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
            client: super::http::streaming_client(),
            api_key: api_key.into(),
        }
    }
}

/// Pluck the `error.type` field out of an Anthropic API error body
/// without parsing the whole JSON. The canonical shape is
/// `{"type":"error","error":{"type":"<kind>","message":"…"}}`. We
/// accept any of the documented values: `authentication_error`,
/// `permission_error`, `rate_limit_error`, `overloaded_error`,
/// `api_error`, `invalid_request_error`, `not_found_error`,
/// `request_too_large`. Returns the matched kind as a static str so
/// callers can match against it without a cloned String. None when
/// the body is missing/malformed (e.g. a 503 HTML page from a proxy).
fn anthropic_error_type(body: &str) -> Option<&'static str> {
    let candidates: &[&'static str] = &[
        "authentication_error",
        "permission_error",
        "rate_limit_error",
        "overloaded_error",
        "request_too_large",
        "invalid_request_error",
        "not_found_error",
        "api_error",
    ];
    // Look for `"type":"<kind>"` *inside* the inner error object.
    // The outer `"type":"error"` is always present, so we skip past
    // it by anchoring to `"error":{`.
    let inner_start = body.find("\"error\":{").map(|i| i + "\"error\":{".len())?;
    let inner = &body[inner_start..];
    for kind in candidates {
        let needle = format!("\"type\":\"{kind}\"");
        if inner.contains(&needle) {
            return Some(kind);
        }
    }
    None
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
        // v132 prompt caching: tag the system prompt as `ephemeral` so
        // Anthropic's 5-minute cache kicks in. Multi-turn sessions read
        // the same big system block every turn — caching it cuts input
        // token costs by ~70% on those turns. The v132 SDK does the
        // same; cli.js sets `cache_control: {type:"ephemeral"}` on the
        // last block of system + tools.
        body["system"] = system_blocks(sys);
    }

    if let Some(temp) = opts.temperature {
        body["temperature"] = serde_json::Value::from(temp);
    }
    if let Some(top_p) = opts.top_p {
        body["top_p"] = serde_json::Value::from(top_p);
    }

    if !opts.tools.is_empty() {
        // Tag the LAST tool with cache_control so the entire tools
        // array becomes a cache breakpoint. v132 picks the last tool
        // (vs. first) so callers can prepend ephemeral tools without
        // re-keying the cache.
        let mut tools = sse::build_tools(&opts.tools);
        if let Some(arr) = tools.as_array_mut() {
            if let Some(last) = arr.last_mut() {
                if let Some(obj) = last.as_object_mut() {
                    obj.insert("cache_control".to_owned(), json!({ "type": "ephemeral" }));
                }
            }
        }
        body["tools"] = tools;
    }

    if opts.adaptive_thinking {
        let mut thinking = json!({ "type": "adaptive" });
        if let Some(display) = opts.thinking_display.as_deref() {
            thinking["display"] = json!(display);
        }
        body["thinking"] = thinking;
    } else if let Some(budget) = opts.thinking_budget {
        body["thinking"] = json!({
            "type": "enabled",
            "budget_tokens": budget,
        });
    }
    {
        let mut oc = serde_json::Map::new();
        if let Some(effort) = opts.reasoning_effort.as_deref() {
            oc.insert("effort".into(), json!(effort));
        }
        if let Some(tb) = opts.task_budget_tokens {
            oc.insert("task_budget".into(), json!({"type": "tokens", "total": tb}));
        }
        if !oc.is_empty() {
            body["output_config"] = serde_json::Value::Object(oc);
        }
    }

    body
}

fn system_blocks(system: &str) -> serde_json::Value {
    let Some(index) = system.find("\n\n## Current diagnostics") else {
        return json!([{ "type": "text", "text": system, "cache_control": { "type": "ephemeral" } }]);
    };

    let stable = system[..index].trim_end();
    let volatile = system[index..].trim_start();
    let mut blocks = Vec::new();
    if !stable.is_empty() {
        blocks.push(json!({
            "type": "text",
            "text": stable,
            "cache_control": { "type": "ephemeral" },
        }));
    }
    if !volatile.is_empty() {
        blocks.push(json!({ "type": "text", "text": volatile }));
    }
    json!(blocks)
}

impl crate::provider::seal::Sealed for AnthropicProvider {}

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

        // Build beta header: append fast-mode and/or task-budgets betas as needed.
        let mut betas = ANTHROPIC_BETA.to_owned();
        if options.fast_mode {
            betas.push_str(",fast-mode-2026-02-01");
        }
        if options.task_budget_tokens.is_some() {
            betas.push_str(",task-budgets-2026-03-13");
        }
        let beta_header = betas;

        let send_started = std::time::Instant::now();
        let resp = match super::http::send_with_retry("anthropic.messages", || {
            self.client
                .post(API_URL)
                .header("x-api-key", &self.api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("anthropic-beta", beta_header.as_str())
                .header("content-type", "application/json")
                .json(&body)
                .send()
        })
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let cause = super::http::classify_send_error(&e);
                tracing::warn!(
                    target: "jfc::provider::anthropic",
                    error = %e,
                    cause = cause,
                    "POST messages failed before response (after retries)"
                );
                anyhow::bail!(
                    "Anthropic request failed: {cause} ({e}). \
                     If this persists, check your network and try again — \
                     long thinking turns can stall briefly under proxies."
                );
            }
        };

        super::http::report_first_byte_latency("anthropic.messages", send_started.elapsed());
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
            // Map the body's `error.type` against the canonical strings
            // v132 (`extracted_2.1.132/src/entrypoints/cli.js`) recognises:
            // overloaded_error / rate_limit_error / api_error /
            // authentication_error / permission_error / invalid_request_error
            // / not_found_error / request_too_large. Surfacing the
            // semantic kind first gives the user a one-line cause
            // before we dump the raw body.
            let kind = anthropic_error_type(&text);
            let friendly = super::retry::friendly_error_message(status.as_u16(), &text);
            match kind {
                Some("authentication_error") => anyhow::bail!(
                    "Authentication failed — check your API key or token. \
                     {friendly}"
                ),
                Some("permission_error") => anyhow::bail!(
                    "Permission denied — your account may not have access \
                     to this model. {friendly}"
                ),
                Some("rate_limit_error") => {
                    anyhow::bail!("Rate limited — wait a moment and retry. {friendly}")
                }
                Some("overloaded_error") => anyhow::bail!(
                    "Anthropic is overloaded ({status}). Try again in a \
                     few seconds. {friendly}"
                ),
                Some("request_too_large") => anyhow::bail!(
                    "Request too large — auto-compaction should kick in. \
                     {friendly}"
                ),
                Some("invalid_request_error") => {
                    anyhow::bail!("Invalid request: {friendly}\n  raw: {text}")
                }
                Some("not_found_error") => anyhow::bail!("Model or endpoint not found: {friendly}"),
                _ => anyhow::bail!("Anthropic API error {status}: {friendly}\n  raw: {text}"),
            }
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

    // Normal: anthropic_error_type recognises the canonical shape
    // and every documented `error.type` value v132 cli.js looks for.
    #[test]
    fn anthropic_error_type_recognises_all_kinds_normal() {
        for kind in &[
            "authentication_error",
            "permission_error",
            "rate_limit_error",
            "overloaded_error",
            "api_error",
            "invalid_request_error",
            "not_found_error",
            "request_too_large",
        ] {
            let body = format!(
                "{{\"type\":\"error\",\"error\":{{\"type\":\"{kind}\",\"message\":\"x\"}}}}"
            );
            assert_eq!(anthropic_error_type(&body), Some(*kind), "{kind}");
        }
    }

    // Robust: an unknown kind, malformed body, or HTML proxy page
    // returns None so the dispatcher falls back to the generic
    // status-code handler.
    #[test]
    fn anthropic_error_type_returns_none_when_missing_robust() {
        assert_eq!(anthropic_error_type(""), None);
        assert_eq!(anthropic_error_type("<html>503</html>"), None);
        assert_eq!(
            anthropic_error_type(
                "{\"type\":\"error\",\"error\":{\"type\":\"future_kind\",\"message\":\"\"}}"
            ),
            None
        );
        // Outer `"type":"error"` alone — no inner error object — None.
        assert_eq!(anthropic_error_type("{\"type\":\"error\"}"), None);
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
    // a system prompt; the on-wire shape is text blocks so cache_control can be
    // applied to the stable prefix.
    #[test]
    fn build_body_includes_system_when_set_normal() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").system("you are helpful"),
        );
        assert_eq!(body["system"][0]["text"], "you are helpful");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    }

    #[test]
    fn build_body_splits_volatile_diagnostics_from_cached_system() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").system("stable instructions\n\n## Current diagnostics\n\nvolatile"),
        );
        assert_eq!(body["system"].as_array().unwrap().len(), 2);
        assert_eq!(body["system"][0]["text"], "stable instructions");
        assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
        assert_eq!(
            body["system"][1]["text"],
            "## Current diagnostics\n\nvolatile"
        );
        assert!(body["system"][1].get("cache_control").is_none());
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

    #[test]
    fn build_body_reasoning_effort_uses_output_config_normal() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").reasoning_effort("xhigh"),
        );
        assert_eq!(body["output_config"]["effort"], "xhigh");
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

    // Normal: `.thinking_display("summarized")` injects `display: "summarized"` into
    // the adaptive thinking block so Opus 4.7 returns thinking text to the caller.
    #[test]
    fn build_body_thinking_display_summarized_normal() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").adaptive().thinking_display("summarized"),
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert_eq!(body["thinking"]["display"], "summarized");
    }

    // Robust: without `.thinking_display()` the `display` key must be absent —
    // Anthropic defaults to `"omitted"` server-side and sending an explicit
    // `display: null` could be rejected by strict validators.
    #[test]
    fn build_body_thinking_display_absent_when_unset_robust() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").adaptive(),
        );
        assert_eq!(body["thinking"]["type"], "adaptive");
        assert!(
            body["thinking"].get("display").is_none(),
            "display key must be absent when thinking_display is not set, got: {}",
            body["thinking"]
        );
    }

    // Normal: task_budget(50_000) produces the correct output_config shape
    // with type "tokens" and total 50000 as required by the API beta spec.
    #[test]
    fn build_body_task_budget_produces_output_config_normal() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").task_budget(50_000),
        );
        assert_eq!(body["output_config"]["task_budget"]["type"], "tokens");
        assert_eq!(body["output_config"]["task_budget"]["total"], 50_000u64);
    }

    // Robust: task_budget(5_000) is below the API minimum of 20_000 and must
    // be clamped up. Sending a sub-minimum value would be rejected by the API.
    #[test]
    fn build_body_task_budget_clamped_to_minimum_robust() {
        let o = opts("m").task_budget(5_000);
        // StreamOptions builder clamps to 20_000.
        assert_eq!(o.task_budget_tokens, Some(20_000));
        let body = build_body(vec![make_user_msg("hi")], &o);
        assert_eq!(body["output_config"]["task_budget"]["total"], 20_000u64);
    }

    // Normal: when both reasoning_effort and task_budget are set, they must
    // both appear in a single output_config object (not overwrite each other).
    #[test]
    fn build_body_effort_and_task_budget_coexist_in_output_config_normal() {
        let body = build_body(
            vec![make_user_msg("hi")],
            &opts("m").reasoning_effort("high").task_budget(30_000),
        );
        assert_eq!(body["output_config"]["effort"], "high");
        assert_eq!(body["output_config"]["task_budget"]["type"], "tokens");
        assert_eq!(body["output_config"]["task_budget"]["total"], 30_000u64);
    }
}
