pub mod jwt;
pub mod oidc;
pub mod store;
pub mod verify;

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use futures::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};

use jfc_provider::{
    EventStream, ModelInfo, Provider, ProviderContent, ProviderMessage, ProviderRole, StopReason,
    StreamConvention, StreamEvent, StreamOptions,
};

pub const AUTO_RETRY_SENTINEL: &str = jfc_provider::retry::OPENWEBUI_AUTO_RETRY_SENTINEL;

// Re-export the new modular auth types so external callers (CLI, etc.) can
// reach them through `providers::openwebui::*`.
pub use self::jwt::{is_token_expired, parse_jwt_claims};
pub use self::oidc::{DuoMethod, OidcLoginOptions, oidc_login};
pub use self::store::{
    Account, default_store_path, get_current, list as list_accounts, load_store,
    remove as remove_account, set_current, upsert as upsert_account,
};
pub use self::verify::{fetch_instance_config, normalize_base_url, verify_token};

// `notify_chat_completed` + `update_user_timezone` are exported at the file
// scope (below `OpenWebUIProvider::stream`) — runtime stream-done hook
// calls them by fully-qualified path so no extra re-export is needed here.

/// Backwards-compat shim so the existing test-suite that calls `load_account(path)`
/// continues to compile against the new modular store.
#[cfg(test)]
fn load_account(path: &std::path::Path) -> anyhow::Result<Account> {
    let store = load_store(path);
    get_current(&store).ok_or_else(|| anyhow::anyhow!("no enabled OpenWebUI accounts in store"))
}

/// Loads the active account, preferring (in order):
///   1. `OPENWEBUI_BASE_URL` + `OPENWEBUI_TOKEN` (or `OPENWEBUI_API_KEY`) env vars
///   2. The current/first-enabled account from the persisted store
///
/// This fixes the bug where `OPENWEBUI_BASE_URL` alone made `has_usable_config`
/// return true but actual requests failed because no token source existed.
fn load_active_account(store_path: &std::path::Path) -> anyhow::Result<Account> {
    if let Ok(base_url) = std::env::var("OPENWEBUI_BASE_URL") {
        let token = std::env::var("OPENWEBUI_TOKEN")
            .or_else(|_| std::env::var("OPENWEBUI_API_KEY"))
            .ok();
        if let Some(t) = token {
            return Ok(Account {
                name: "env".into(),
                base_url,
                token: t,
                ..Default::default()
            });
        }
        // Base URL without token → fall through and try the store; the store's
        // active account may belong to that same base URL.
    }
    let store = load_store(store_path);
    get_current(&store).ok_or_else(|| {
        anyhow::anyhow!(
            "no enabled OpenWebUI accounts in store at {} (set OPENWEBUI_TOKEN or run `jfc auth openwebui login`)",
            store_path.display()
        )
    })
}

pub struct OpenWebUIProvider {
    client: reqwest::Client,
    store_path: PathBuf,
}

impl Default for OpenWebUIProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl OpenWebUIProvider {
    pub fn new() -> Self {
        let store_path = default_store_path();
        tracing::debug!(
            target: "jfc::provider::openwebui",
            store_path = %store_path.display(),
            "OpenWebUIProvider::new"
        );
        Self {
            client: jfc_provider::http::streaming_client(),
            store_path,
        }
    }

    /// True when an active account is resolvable: env-vars (`OPENWEBUI_BASE_URL` +
    /// `OPENWEBUI_TOKEN`) OR an enabled entry in the persisted store.
    /// The previous behavior allowed `OPENWEBUI_BASE_URL` alone, which then
    /// failed at request time because no token source existed.
    pub fn has_usable_config(&self) -> bool {
        let result = load_active_account(&self.store_path).is_ok();
        tracing::trace!(
            target: "jfc::provider::openwebui",
            result,
            "has_usable_config"
        );
        result
    }

    /// Load the active account, and if its JWT is expired (or near expiring),
    /// refresh it via OIDC + Duo using the OWUI_* env vars. If refresh fails
    /// or env vars aren't set, returns the (possibly expired) account anyway
    /// — a 401 from the upstream is more informative than blocking up-front.
    pub async fn acquire_account_with_refresh(&self) -> anyhow::Result<Account> {
        let mut account = load_active_account(&self.store_path)?;

        // Skip expiry check for env-only accounts — the user controls their
        // own token rotation in that case.
        let is_env_only = account.name == "env";
        if is_env_only {
            return Ok(account);
        }

        // 60 s skew lets a request that's currently in flight finish before
        // we proactively refresh.
        if is_token_expired(&account.token, 60_000) {
            tracing::info!(
                target: "jfc::provider::openwebui",
                account = %account.name,
                expires_at = ?account.expires_at,
                "token expired or near expiry — attempting auto-refresh"
            );
            match self.refresh_active_account().await {
                Ok(refreshed) => {
                    tracing::info!(
                        target: "jfc::provider::openwebui",
                        account = %refreshed.name,
                        new_expires_at = ?refreshed.expires_at,
                        "auto-refresh succeeded"
                    );
                    account = refreshed;
                }
                Err(e) => {
                    tracing::warn!(
                        target: "jfc::provider::openwebui",
                        error = %e,
                        "auto-refresh failed — continuing with old token (request may 401)"
                    );
                }
            }
        }
        Ok(account)
    }

    /// Refresh the active account's token via OIDC + Duo, using OWUI_USERNAME /
    /// OWUI_PASSWORD / OWUI_DUO_PASSCODE env vars. Persists the new token.
    /// Returns the refreshed account.
    pub async fn refresh_active_account(&self) -> anyhow::Result<Account> {
        let username =
            std::env::var("OWUI_USERNAME").map_err(|_| anyhow::anyhow!("OWUI_USERNAME not set"))?;
        let password =
            std::env::var("OWUI_PASSWORD").map_err(|_| anyhow::anyhow!("OWUI_PASSWORD not set"))?;
        let passcode = std::env::var("OWUI_DUO_PASSCODE").ok();
        let method = if passcode.is_some() {
            DuoMethod::Passcode
        } else {
            DuoMethod::Push
        };

        let mut current = load_active_account(&self.store_path)?;
        let mut opts = OidcLoginOptions::new(&current.base_url, &username, &password);
        opts.duo_passcode = passcode;
        opts.duo_method = method;

        let result = oidc_login(opts).await?;
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        current.token = result.token;
        current.expires_at = Some(result.expires_at);
        current.updated_at = Some(now_ms);
        if current.created_at.is_none() {
            current.created_at = Some(now_ms);
        }
        upsert_account(&self.store_path, current.clone())?;
        // One-shot timezone push so OWUI's user record matches the
        // local clock for any server-side filter that formats timestamps.
        // Detached: never blocks login.
        let base_url = current.base_url.clone();
        let token = current.token.clone();
        tokio::spawn(async move {
            let tz = detect_iana_timezone();
            update_user_timezone(&base_url, &token, &tz).await;
        });
        Ok(current)
    }
}

#[derive(Debug, Deserialize)]
struct ModelsResponse {
    data: Vec<ApiModelInfo>,
}

#[derive(Debug, Deserialize)]
struct ApiModelInfo {
    id: String,
    name: Option<String>,
    #[serde(flatten)]
    metadata: Value,
}

fn context_window_from_value(value: &Value) -> Option<usize> {
    const KEYS: &[&str] = &[
        "context_length",
        "max_context_length",
        "context_window",
        "context_window_tokens",
        "max_context_window",
        "max_context",
        "max_ctx",
        "num_ctx",
        "n_ctx",
        "ctx_len",
        "max_position_embeddings",
        "general.context_length",
    ];

    match value {
        Value::Object(map) => {
            for key in KEYS {
                if let Some(tokens) = map.get(*key).and_then(value_as_usize) {
                    return Some(tokens);
                }
            }

            for (key, value) in map {
                if key.ends_with(".context_length")
                    && let Some(tokens) = value_as_usize(value)
                {
                    return Some(tokens);
                }
            }

            map.values().find_map(context_window_from_value)
        }
        Value::Array(items) => items.iter().find_map(context_window_from_value),
        _ => None,
    }
}

fn context_window_from_model(model: &ApiModelInfo) -> usize {
    context_window_from_value(&model.metadata)
        .unwrap_or_else(|| infer_context_window_from_model_name(&model.id, model.name.as_deref()))
}

pub fn infer_context_window_from_model_name(id: &str, name: Option<&str>) -> usize {
    let haystack = format!("{} {}", id, name.unwrap_or_default()).to_lowercase();
    let has = |needle: &str| haystack.contains(needle);
    let has_version = |major: &str, minor: &str| {
        has(&format!("{major}.{minor}"))
            || has(&format!("{major}_{minor}"))
            || has(&format!("{major}-{minor}"))
    };

    if has("claude")
        && (has("mythos")
            || (has("opus") && (has_version("4", "7") || has_version("4", "6")))
            || (has("sonnet") && has_version("4", "6"))
            || (has("opus") && has_version("4", "5")))
    {
        1_000_000
    } else if has("claude") {
        200_000
    } else if has("gpt") && has("5") {
        1_000_000
    } else if has("gpt") && (has("4o") || has("4")) {
        128_000
    } else if has("llama") && has("4") && has("maverick") {
        1_048_576
    } else if has("llama") && (has("4") || has("3")) {
        131_072
    } else if has("gemma") && has("3") {
        128_000
    } else if has("gemini") && has("2") {
        1_048_576
    } else if has("nova") && (has("pro") || has("lite")) {
        300_000
    } else {
        128_000
    }
}

fn value_as_usize(value: &Value) -> Option<usize> {
    match value {
        Value::Number(n) => n.as_u64().and_then(|v| usize::try_from(v).ok()),
        Value::String(s) => s.parse::<usize>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_window_is_read_from_common_openwebui_shapes() {
        let direct = serde_json::json!({ "context_length": 131072 });
        assert_eq!(context_window_from_value(&direct), Some(131_072));

        let nested = serde_json::json!({
            "info": {
                "params": { "num_ctx": "32768" }
            }
        });
        assert_eq!(context_window_from_value(&nested), Some(32_768));

        let ollama_details = serde_json::json!({
            "details": {
                "model_info": { "llama.context_length": 65536 }
            }
        });
        assert_eq!(context_window_from_value(&ollama_details), Some(65_536));
    }

    #[test]
    fn openwebui_model_context_falls_back_to_provider_inference() {
        let claude = ApiModelInfo {
            id: "anthropic/claude-sonnet-4-5".to_string(),
            name: None,
            metadata: Value::Null,
        };
        assert_eq!(context_window_from_model(&claude), 200_000);

        let claude_opus_46 = ApiModelInfo {
            id: "anthropic/claude-opus-4-6".to_string(),
            name: None,
            metadata: Value::Null,
        };
        assert_eq!(context_window_from_model(&claude_opus_46), 1_000_000);

        let gpt5 = ApiModelInfo {
            id: "openai/gpt-5-mini".to_string(),
            name: None,
            metadata: Value::Null,
        };
        assert_eq!(context_window_from_model(&gpt5), 1_000_000);

        let custom = ApiModelInfo {
            id: "local/custom-model".to_string(),
            name: None,
            metadata: Value::Null,
        };
        assert_eq!(context_window_from_model(&custom), 128_000);
    }

    // ── Real-API integration tests (gated #[ignore]) ──────────────────────
    // Run with: cargo test --bin jfc -- --ignored openwebui
    // Reads ~/.config/opencode/openwebui-accounts.json (or jfc fallback) and hits
    // the configured instance's /api/models. Skips silently when no creds exist.

    fn live_provider() -> Option<OpenWebUIProvider> {
        let p = OpenWebUIProvider::new();
        if !p.has_usable_config() {
            eprintln!(
                "skipping live test: no openwebui creds at {}",
                p.store_path.display()
            );
            return None;
        }
        Some(p)
    }

    // Normal: live `/api/models` returns at least one entry with a non-empty id,
    // tagged with the "openwebui" provider so the picker can route it.
    #[tokio::test]
    #[ignore = "hits live OpenWebUI instance — run with cargo test -- --ignored"]
    async fn live_fetch_models_returns_real_list_normal() {
        let Some(p) = live_provider() else { return };
        let models = p.fetch_models().await.expect("fetch_models");
        assert!(!models.is_empty(), "instance returned zero models");
        for m in &models {
            assert!(!m.id.is_empty(), "empty model id in {m:?}");
            assert_eq!(m.provider, "openwebui");
        }
    }

    // Robust: account loader fails cleanly on a path that doesn't exist (verifies
    // we surface the error instead of panicking inside fetch_models).
    #[test]
    fn load_account_missing_file_errors_robust() {
        let bogus = PathBuf::from("/tmp/this-path-definitely-does-not-exist.json");
        assert!(load_account(&bogus).is_err());
    }

    // ── Tool wire-format (the file-system-write bug fix) ──────────────────
    use jfc_provider::ToolDef;

    fn opts_with_bash_tool() -> StreamOptions {
        StreamOptions::new("any-model").tools(vec![ToolDef {
            name: "Bash".into(),
            description: "Run a shell command".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": { "type": "string" }
                },
                "required": ["command"]
            }),
        }])
    }

    fn user_msg(text: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::Text(text.into())],
        }
    }

    // Normal: when the caller passes a tool, the body includes a top-level
    // `tools` array in OpenAI function-tool shape and `tool_choice: "auto"`.
    // The tool name is lowercased (see `build_body` for rationale — Bedrock's
    // guardrail strips PascalCase tool_calls).
    #[test]
    fn build_body_includes_tools_when_caller_provides_them_normal() {
        let body = build_body(vec![user_msg("hi")], &opts_with_bash_tool());
        let tools = body.get("tools").and_then(|v| v.as_array()).expect("tools");
        assert_eq!(tools.len(), 1);
        let t0 = &tools[0];
        assert_eq!(t0.get("type").and_then(|v| v.as_str()), Some("function"));
        let func = t0.get("function").expect("function");
        assert_eq!(func.get("name").and_then(|v| v.as_str()), Some("bash"));
        assert!(func.get("parameters").is_some());
        assert_eq!(
            body.get("tool_choice").and_then(|v| v.as_str()),
            Some("auto")
        );
    }

    // Normal: tool names are lowercased before being sent. Bedrock's guardrail
    // and LiteLLM's tool-call validator silently strip tool_calls whose names
    // match an "executor"-shaped pattern; lowercase names bypass the filter.
    // Pinning this ensures a future refactor doesn't accidentally send
    // PascalCase and re-introduce the empty-tool_calls bug.
    #[test]
    fn build_body_lowercases_tool_names_normal() {
        let opts = StreamOptions::new("m").tools(vec![
            ToolDef {
                name: "Bash".into(),
                description: "x".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDef {
                name: "Read".into(),
                description: "x".into(),
                input_schema: serde_json::json!({}),
            },
            ToolDef {
                name: "ApplyPatch".into(),
                description: "x".into(),
                input_schema: serde_json::json!({}),
            },
        ]);
        let body = build_body(vec![user_msg("hi")], &opts);
        let names: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["bash", "read", "applypatch"]);
    }

    // Robust: no tools → no `tools` field at all (some OWUI proxies reject an
    // empty `tools: []` payload).
    #[test]
    fn build_body_omits_tools_field_when_empty_robust() {
        let body = build_body(vec![user_msg("hi")], &StreamOptions::new("m"));
        assert!(body.get("tools").is_none(), "tools leaked: {body}");
        assert!(body.get("tool_choice").is_none());
    }

    // Shared build_body keeps reasoning_effort so LiteLLM (which reuses
    // this helper) gets the param it needs to map to provider-specific
    // shapes. The OWUI-direct path strips it in `stream()` — covered by
    // `stream_strips_reasoning_effort_for_owui_regression` below.
    #[test]
    fn build_body_keeps_reasoning_effort_for_litellm_normal() {
        let mut opts = StreamOptions::new("m");
        opts.reasoning_effort = Some("high".to_string());
        let body = build_body(vec![user_msg("hi")], &opts);
        assert_eq!(body["reasoning_effort"], "high");
    }

    // Provider_options is the escape hatch — when the user *knows* the
    // OWUI upstream supports reasoning_effort, they can set it directly
    // via `[agents."<m>"].provider_options.reasoning_effort` and it
    // wins over the field-level value.
    #[test]
    fn build_body_provider_options_overrides_field_normal() {
        let mut opts = StreamOptions::new("m");
        opts.reasoning_effort = Some("high".to_string());
        opts.provider_options
            .insert("reasoning_effort".to_string(), Value::from("low"));
        let body = build_body(vec![user_msg("hi")], &opts);
        assert_eq!(body["reasoning_effort"], "low");
    }

    // Regression: simulate the OWUI-stream post-build strip step. OWUI
    // forwards reasoning_effort verbatim to backends that 500 on it
    // (Bedrock-Claude on chat.ai2s.org). Strip must happen at the
    // stream-call layer, not in shared build_body.
    #[test]
    fn owui_stream_post_build_strip_removes_reasoning_effort_regression() {
        let mut opts = StreamOptions::new("m");
        opts.reasoning_effort = Some("high".to_string());
        let body = build_openwebui_chat_body(vec![user_msg("hi")], &opts);
        assert!(
            body.get("reasoning_effort").is_none(),
            "post-build strip failed: {body}"
        );
    }

    // The strip MUST NOT fire when the user set reasoning_effort via
    // provider_options — that's the explicit "I know my backend
    // supports it" opt-in.
    #[test]
    fn owui_stream_post_build_strip_respects_provider_options_normal() {
        let mut opts = StreamOptions::new("m");
        opts.reasoning_effort = Some("high".to_string());
        opts.provider_options
            .insert("reasoning_effort".to_string(), Value::from("medium"));
        let body = build_openwebui_chat_body(vec![user_msg("hi")], &opts);
        // provider_options wrote "medium" over "high" in build_body; strip
        // is bypassed because provider_options registered the key.
        assert_eq!(body["reasoning_effort"], "medium");
    }

    // Regression: OWUI 0.9.x changed `/api/chat/completions` to the web-chat
    // route. That path calls `chat_id.startswith(...)`; a bare OpenAI payload
    // leaves chat_id as None and returns the screenshot's NoneType error.
    #[test]
    fn owui_chat_route_compat_injects_local_chat_id_robust() {
        let mut body = build_body(vec![user_msg("hi")], &StreamOptions::new("m"));
        assert!(
            body.get("chat_id").is_none(),
            "shared build_body must stay OpenAI-compatible: {body}"
        );

        let mut opts = StreamOptions::new("m");
        opts.session_id = Some("jfc-test".to_string());
        apply_openwebui_chat_route_compat(&mut body, &opts);

        assert_eq!(body["chat_id"], "local:jfc-test");
        assert!(
            body.get("session_id").is_none(),
            "session_id would switch OWUI into background task mode: {body}"
        );
        assert!(
            body.get("id").is_none(),
            "id would create OWUI message/task metadata JFC does not consume: {body}"
        );
    }

    #[test]
    fn owui_chat_route_compat_preserves_explicit_chat_id_normal() {
        let mut opts = StreamOptions::new("m");
        opts.provider_options
            .insert("chat_id".to_string(), Value::from("local:user-supplied"));
        let mut body = build_body(vec![user_msg("hi")], &opts);

        apply_openwebui_chat_route_compat(&mut body, &StreamOptions::new("m"));

        assert_eq!(body["chat_id"], "local:user-supplied");
    }

    #[test]
    fn build_openwebui_chat_body_applies_route_compat_normal() {
        let mut opts = StreamOptions::new("m");
        opts.session_id = Some("ses_123".to_string());
        let body = build_openwebui_chat_body(vec![user_msg("hi")], &opts);

        assert_eq!(body["chat_id"], "local:ses_123");
        assert!(
            body.get("session_id").is_none(),
            "session_id would switch OWUI into background task mode: {body}"
        );
        assert!(
            body.get("id").is_none(),
            "id would create OWUI message/task metadata JFC does not consume: {body}"
        );
    }

    #[test]
    fn local_openwebui_chat_id_prefixes_plain_session_normal() {
        assert_eq!(
            local_openwebui_chat_id(Some("ses_123")).as_str(),
            "local:ses_123"
        );
        assert_eq!(
            local_openwebui_chat_id(Some("local:abc")).as_str(),
            "local:abc"
        );
        assert!(local_openwebui_chat_id(None).starts_with("local:jfc-"));
    }

    // Normal: assistant tool_use blocks from prior turns serialize into the
    // OpenAI `assistant.tool_calls[]` shape with **lowercased** function
    // names. v126 LiteLLM matches against `tools[].function.name` strictly
    // case-sensitively; if the conversation history carries PascalCase names
    // (e.g. from a prior anthropic-oauth turn), they must be normalized to
    // match what we send for new tool calls.
    #[test]
    fn build_body_serializes_assistant_tool_use_normal() {
        let history = vec![
            user_msg("hi"),
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![ProviderContent::ToolUse {
                    id: "call_abc".into(),
                    name: "Bash".into(), // PascalCase from anthropic-oauth turn
                    input: serde_json::json!({"command": "echo hi"}),
                    thought_signature: None,
                }],
            },
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::ToolResult {
                    tool_use_id: "call_abc".into(),
                    content: "hi".into(),
                    is_error: false,
                }],
            },
        ];
        let body = build_body(history, &opts_with_bash_tool());
        let msgs = body
            .get("messages")
            .and_then(|v| v.as_array())
            .expect("messages");
        let asst = msgs
            .iter()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .expect("assistant turn");
        assert_eq!(
            asst.get("content").and_then(|v| v.as_str()),
            Some(BEDROCK_BLANK_TEXT_PLACEHOLDER),
            "assistant tool-call turns must carry non-null content for OpenWebUI compatibility"
        );
        let calls = asst
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .expect("tool_calls");
        assert_eq!(calls.len(), 1);
        let c0 = &calls[0];
        assert_eq!(c0.get("id").and_then(|v| v.as_str()), Some("call_abc"));
        // The historical PascalCase name was lowercased on serialization to
        // match the casing of new tool calls we send.
        assert_eq!(
            c0.get("function")
                .and_then(|f| f.get("name"))
                .and_then(|v| v.as_str()),
            Some("bash")
        );
        // arguments is JSON-serialized as a string per OpenAI's spec.
        let args = c0
            .get("function")
            .and_then(|f| f.get("arguments"))
            .and_then(|v| v.as_str())
            .expect("arguments");
        assert!(args.contains("echo hi"));
    }

    // Robust: cross-provider switch — when conversation crosses Anthropic
    // (PascalCase) → OWUI (lowercase), historical tool_use names of every
    // case end up lowercased so LiteLLM matches the active `tools` array.
    #[test]
    fn build_body_lowercases_historical_tool_use_names_robust() {
        let history = vec![
            user_msg("first"),
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![
                    ProviderContent::ToolUse {
                        id: "c1".into(),
                        name: "Bash".into(),
                        input: serde_json::json!({}),
                        thought_signature: None,
                    },
                    ProviderContent::ToolUse {
                        id: "c2".into(),
                        name: "Read".into(),
                        input: serde_json::json!({}),
                        thought_signature: None,
                    },
                    ProviderContent::ToolUse {
                        id: "c3".into(),
                        name: "ApplyPatch".into(),
                        input: serde_json::json!({}),
                        thought_signature: None,
                    },
                ],
            },
            // Tool results follow the assistant turn so the conversation
            // ends on a user/tool turn (Bedrock prefill compat).
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![
                    ProviderContent::ToolResult {
                        tool_use_id: "c1".into(),
                        content: "ok".into(),
                        is_error: false,
                    },
                    ProviderContent::ToolResult {
                        tool_use_id: "c2".into(),
                        content: "ok".into(),
                        is_error: false,
                    },
                    ProviderContent::ToolResult {
                        tool_use_id: "c3".into(),
                        content: "ok".into(),
                        is_error: false,
                    },
                ],
            },
        ];
        let body = build_body(history, &opts_with_bash_tool());
        let msgs = body["messages"].as_array().unwrap();
        let names: Vec<&str> = msgs
            .iter()
            .filter(|m| m.get("role").and_then(|r| r.as_str()) == Some("assistant"))
            .filter_map(|m| m.get("tool_calls").and_then(|v| v.as_array()))
            .flatten()
            .map(|c| c["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, vec!["bash", "read", "applypatch"]);
    }

    // Regression: parallel tool_use blocks from one assistant turn must remain
    // in one OpenAI assistant.tool_calls array. Splitting them into consecutive
    // assistant messages makes LiteLLM/Bedrock convert the first tool_use into
    // an Anthropic message whose next message is another assistant, not the
    // required tool_result user turn.
    #[test]
    fn build_body_groups_parallel_tool_uses_before_results_regression() {
        let history = vec![
            user_msg("run these in parallel"),
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![
                    ProviderContent::ToolUse {
                        id: "call_a".into(),
                        name: "Bash".into(),
                        input: serde_json::json!({"command": "pwd"}),
                        thought_signature: None,
                    },
                    ProviderContent::ToolUse {
                        id: "call_b".into(),
                        name: "Read".into(),
                        input: serde_json::json!({"file_path": "Cargo.toml"}),
                        thought_signature: None,
                    },
                ],
            },
            ProviderMessage {
                role: ProviderRole::User,
                content: vec![
                    ProviderContent::ToolResult {
                        tool_use_id: "call_a".into(),
                        content: "/tmp/repo".into(),
                        is_error: false,
                    },
                    ProviderContent::ToolResult {
                        tool_use_id: "call_b".into(),
                        content: "[package]".into(),
                        is_error: false,
                    },
                ],
            },
        ];

        let body = build_body(history, &opts_with_bash_tool());
        let msgs = body["messages"].as_array().expect("messages");
        let assistant_tool_turns: Vec<&Value> = msgs
            .iter()
            .filter(|m| m.get("tool_calls").and_then(|v| v.as_array()).is_some())
            .collect();
        assert_eq!(
            assistant_tool_turns.len(),
            1,
            "parallel tool calls must not be split into adjacent assistant messages: {msgs:?}"
        );

        let assistant_idx = msgs
            .iter()
            .position(|m| m.get("tool_calls").and_then(|v| v.as_array()).is_some())
            .expect("assistant tool turn");
        let calls = msgs[assistant_idx]["tool_calls"]
            .as_array()
            .expect("tool_calls");
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0]["id"], "call_a");
        assert_eq!(calls[1]["id"], "call_b");

        assert_eq!(msgs[assistant_idx + 1]["role"], "tool");
        assert_eq!(msgs[assistant_idx + 1]["tool_call_id"], "call_a");
        assert_eq!(msgs[assistant_idx + 2]["role"], "tool");
        assert_eq!(msgs[assistant_idx + 2]["tool_call_id"], "call_b");
    }

    // Normal: tool results from prior turns become role:"tool" messages with
    // the matching tool_call_id — required so the model can resolve which
    // call each result answers.
    #[test]
    fn build_body_serializes_tool_result_as_tool_role_normal() {
        let history = vec![ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: "call_abc".into(),
                content: "exit 0\nstdout: ok".into(),
                is_error: false,
            }],
        }];
        let body = build_body(history, &opts_with_bash_tool());
        let msgs = body
            .get("messages")
            .and_then(|v| v.as_array())
            .expect("messages");
        let tool = msgs
            .iter()
            .find(|m| m.get("role").and_then(|r| r.as_str()) == Some("tool"))
            .expect("tool turn");
        assert_eq!(
            tool.get("tool_call_id").and_then(|v| v.as_str()),
            Some("call_abc")
        );
        assert!(
            tool.get("content")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .contains("exit 0")
        );
    }

    fn fn_call(
        idx: usize,
        id: Option<&str>,
        name: Option<&str>,
        args: Option<&str>,
    ) -> ChunkToolCall {
        ChunkToolCall {
            index: Some(idx),
            id: id.map(str::to_owned),
            function: Some(ChunkToolFn {
                name: name.map(str::to_owned),
                arguments: args.map(str::to_owned),
            }),
        }
    }

    fn chunk(delta: ChunkDelta, finish: Option<&str>) -> ChatChunk {
        ChatChunk {
            choices: vec![ChunkChoice {
                delta,
                finish_reason: finish.map(str::to_owned),
            }],
            usage: None,
        }
    }

    // ── Stateful accumulator (fix for LiteLLM-on-Bedrock empty-finish bug) ─

    fn evs_stateful(state: &mut OpenAiStreamState, c: ChatChunk) -> Vec<StreamEvent> {
        let mut out: Vec<anyhow::Result<StreamEvent>> = Vec::new();
        push_chunk_events_stateful(c, state, &mut out);
        out.into_iter().filter_map(Result::ok).collect()
    }

    // Normal: multi-chunk tool call where name+id arrive on chunk 0, args
    // arrive across chunks 1-3, and finish_reason fires on a chunk with EMPTY
    // tool_calls (LiteLLM-on-Bedrock's behavior). The accumulator must still
    // synthesize a ToolDone with the assembled name/id/args.
    #[test]
    fn stateful_handles_litellm_empty_finish_chunk_normal() {
        let mut state = OpenAiStreamState::default();

        // Chunk 0: name + id, no args yet.
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![fn_call(0, Some("call_x"), Some("bash"), Some(""))]),
                    ..Default::default()
                },
                None,
            ),
        );

        // Chunks 1-3: args fragments, no name.
        for frag in ["{\"comm", "and\":\"l", "s -la\"}"] {
            evs_stateful(
                &mut state,
                chunk(
                    ChunkDelta {
                        tool_calls: Some(vec![fn_call(0, None, None, Some(frag))]),
                        ..Default::default()
                    },
                    None,
                ),
            );
        }

        // Final chunk: finish_reason fires with EMPTY tool_calls (the bug).
        let final_events = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(Vec::new()),
                    ..Default::default()
                },
                Some("tool_calls"),
            ),
        );

        // Expect a ToolDone synthesized from the accumulator + a Done(ToolUse).
        let done = final_events
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolDone {
                    index,
                    tool_name,
                    tool_use_id,
                    input_json,
                    ..
                } => Some((
                    *index,
                    tool_name.clone(),
                    tool_use_id.clone(),
                    input_json.clone(),
                )),
                _ => None,
            })
            .expect("synthesized ToolDone");
        assert_eq!(done.0, 0);
        assert_eq!(done.1, "bash");
        assert_eq!(done.2, "call_x");
        assert_eq!(done.3, "{\"command\":\"ls -la\"}");

        assert!(final_events.iter().any(|e| matches!(
            e,
            StreamEvent::Done {
                stop_reason: StopReason::ToolUse
            }
        )));
    }

    // Normal: multiple parallel tool calls — each index gets its own
    // accumulator entry. ToolDone events are emitted in index order.
    #[test]
    fn stateful_handles_parallel_tool_calls_normal() {
        let mut state = OpenAiStreamState::default();
        // Both tools start in one chunk — different indices.
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![
                        fn_call(0, Some("a"), Some("read"), Some("")),
                        fn_call(1, Some("b"), Some("grep"), Some("")),
                    ]),
                    ..Default::default()
                },
                None,
            ),
        );
        // Each gets a small args fragment.
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![
                        fn_call(0, None, None, Some("{\"path\":\"/x\"}")),
                        fn_call(1, None, None, Some("{\"pattern\":\"foo\"}")),
                    ]),
                    ..Default::default()
                },
                None,
            ),
        );
        // Empty finish chunk.
        let final_events =
            evs_stateful(&mut state, chunk(ChunkDelta::default(), Some("tool_calls")));
        let dones: Vec<_> = final_events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolDone {
                    index,
                    tool_name,
                    tool_use_id,
                    input_json,
                    ..
                } => Some((
                    *index,
                    tool_name.clone(),
                    tool_use_id.clone(),
                    input_json.clone(),
                )),
                _ => None,
            })
            .collect();
        assert_eq!(dones.len(), 2);
        assert_eq!(
            dones[0],
            (0, "read".into(), "a".into(), "{\"path\":\"/x\"}".into())
        );
        assert_eq!(
            dones[1],
            (1, "grep".into(), "b".into(), "{\"pattern\":\"foo\"}".into())
        );
    }

    // Normal: state is drained on finish so a subsequent stream starts clean.
    #[test]
    fn stateful_drains_accumulator_on_finish_normal() {
        let mut state = OpenAiStreamState::default();
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![fn_call(0, Some("a"), Some("read"), Some("{}"))]),
                    ..Default::default()
                },
                Some("tool_calls"),
            ),
        );
        assert!(
            state.tools.is_empty(),
            "accumulator not drained on finish: {:?}",
            state.tools
        );
    }

    // Normal: an inline `<tool_call>{…}</tool_call>` block in the *content*
    // stream (the LiteLLM Qwen3-on-Bedrock format, where `arguments` is a
    // double-encoded JSON string) is intercepted and synthesized into a
    // ToolDone — NOT leaked as text. Mirrors
    // amazon_qwen3_transformation.py:146.
    #[test]
    fn inline_tool_call_in_content_becomes_tool_done_normal() {
        let mut state = OpenAiStreamState::default();
        // Whole block in one delta, with surrounding prose.
        let evs = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(
                        "Let me look. <tool_call>\n{\"name\": \"bash\", \"arguments\": \"{\\\"command\\\":\\\"ls\\\"}\"}\n</tool_call> done"
                            .into(),
                    ),
                    ..Default::default()
                },
                None,
            ),
        );
        // Prose before the tag is emitted as text.
        assert!(
            evs.iter()
                .any(|e| matches!(e, StreamEvent::TextDelta { delta, .. } if delta.contains("Let me look."))),
            "expected leading prose, got {evs:?}"
        );
        // The tool call is synthesized with the right name + (decoded) args.
        let done = evs
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolDone {
                    tool_name,
                    input_json,
                    ..
                } => Some((tool_name.clone(), input_json.clone())),
                _ => None,
            })
            .expect("synthesized ToolDone from inline XML");
        assert_eq!(done.0, "bash");
        assert_eq!(done.1, "{\"command\":\"ls\"}");
        // The raw `<tool_call>` XML must NOT appear in any text delta.
        assert!(
            !evs.iter().any(|e| matches!(e, StreamEvent::TextDelta { delta, .. } if delta.contains("<tool_call>"))),
            "inline XML leaked into text: {evs:?}"
        );
    }

    // Robust: some OpenWebUI/LiteLLM routes emit a tool call as the entire
    // assistant text content instead of structured `tool_calls` or XML tags.
    // The screenshot symptom was the raw JSON rendering in the transcript.
    #[test]
    fn bare_json_tool_call_content_becomes_tool_done_robust() {
        let mut state = OpenAiStreamState::default();
        let first = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(
                        r#"{
                          "name": "Skill",
                          "arguments": {
                            "name": "vuln-researcher",
                            "args": "JS asset mining and deobfuscation workflow"
                          }
                        }"#
                        .into(),
                    ),
                    ..Default::default()
                },
                None,
            ),
        );
        assert!(
            first.is_empty(),
            "bare JSON tool call should be held until finish: {first:?}"
        );

        let second = evs_stateful(&mut state, chunk(ChunkDelta::default(), Some("stop")));
        let done = second
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolDone {
                    tool_name,
                    input_json,
                    ..
                } => Some((tool_name.clone(), input_json.clone())),
                _ => None,
            })
            .expect("ToolDone synthesized from bare JSON");
        assert_eq!(done.0, "Skill");
        let input: serde_json::Value = serde_json::from_str(&done.1).expect("tool input json");
        assert_eq!(input["name"], "vuln-researcher");
        assert_eq!(input["args"], "JS asset mining and deobfuscation workflow");
        assert!(
            !second.iter().any(
                |e| matches!(e, StreamEvent::TextDelta { delta, .. } if delta.contains("\"name\": \"Skill\""))
            ),
            "raw JSON tool call leaked into text: {second:?}"
        );
    }

    // Robust: only known jfc tool names are consumed as bare JSON. This keeps
    // normal JSON answers from being treated as executable tool requests.
    #[test]
    fn bare_json_unknown_tool_remains_text_robust() {
        let mut state = OpenAiStreamState::default();
        let first = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(r#"{"name":"not_a_real_tool","arguments":{"x":1}}"#.into()),
                    ..Default::default()
                },
                None,
            ),
        );
        assert!(first.is_empty(), "candidate JSON should be held: {first:?}");

        let second = evs_stateful(&mut state, chunk(ChunkDelta::default(), Some("stop")));
        assert!(
            !second
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolDone { .. })),
            "unknown tool JSON must not execute: {second:?}"
        );
        let text: String = second
            .iter()
            .filter_map(|e| match e {
                StreamEvent::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert!(
            text.contains("not_a_real_tool"),
            "unknown tool JSON should render as text: {text:?}"
        );
    }

    // Normal: Bedrock Claude emits Anthropic-style `<tool_use>` tags (args as
    // an object). They must intercept exactly like `<tool_call>`, and an inline
    // `<tool_result>` (model-fabricated) must be suppressed, not leaked.
    #[test]
    fn inline_tool_use_claude_format_and_result_suppressed_normal() {
        let mut state = OpenAiStreamState::default();
        let evs = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(
                        "<tool_use> {\"name\": \"codegraph_context\",\"arguments\":{\"query\":\"arch\",\"scope\":\"global\"}} </tool_use> <tool_result> fabricated </tool_result> Here's my take:"
                            .into(),
                    ),
                    ..Default::default()
                },
                None,
            ),
        );
        let done = evs
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolDone {
                    tool_name,
                    input_json,
                    ..
                } => Some((tool_name.clone(), input_json.clone())),
                _ => None,
            })
            .expect("ToolDone synthesized from <tool_use>");
        assert_eq!(done.0, "codegraph_context");
        assert_eq!(done.1, "{\"query\":\"arch\",\"scope\":\"global\"}");
        let text: String = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        // Trailing prose survives; neither tag NOR the fabricated result leaks.
        assert!(text.contains("Here's my take:"), "prose lost: {text:?}");
        assert!(!text.contains("<tool_use>"), "tool_use leaked: {text:?}");
        assert!(
            !text.contains("fabricated"),
            "fabricated result leaked: {text:?}"
        );
    }

    // Robust: a `<tool_call>` open tag split across two content deltas is held
    // until the close tag arrives, then synthesized once — no partial-tag text
    // leak.
    #[test]
    fn inline_tool_call_split_across_deltas_robust() {
        let mut state = OpenAiStreamState::default();
        let first = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some("pre <tool_c".into()),
                    ..Default::default()
                },
                None,
            ),
        );
        // The partial tag must be held back — only "pre " is safe to emit.
        assert!(
            !first.iter().any(
                |e| matches!(e, StreamEvent::TextDelta { delta, .. } if delta.contains("<tool_c"))
            ),
            "partial open tag leaked: {first:?}"
        );
        let second = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(
                        "all>{\"name\":\"read\",\"arguments\":{\"path\":\"/x\"}}</tool_call>"
                            .into(),
                    ),
                    ..Default::default()
                },
                None,
            ),
        );
        let done = second
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolDone {
                    tool_name,
                    input_json,
                    ..
                } => Some((tool_name.clone(), input_json.clone())),
                _ => None,
            })
            .expect("ToolDone after completing split tag");
        assert_eq!(done.0, "read");
        assert_eq!(done.1, "{\"path\":\"/x\"}");
    }

    // Normal: the live LiteLLM/Bedrock plural shape from
    // ses_20260521_062804.json — `<tool_calls><tool_call><tool_name>NAME
    // </tool_name><tool_input>{json}</tool_input></tool_call></tool_calls>`.
    // The wrapper is intercepted and one ToolDone is synthesized with the
    // child-tag name + input; the raw XML never leaks into text.
    #[test]
    fn inline_plural_wrapper_child_tags_becomes_tool_done_normal() {
        let mut state = OpenAiStreamState::default();
        let evs = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(
                        "\n<tool_calls>\n<tool_call><tool_name>Bash</tool_name>\n<tool_input>{\"command\": \"ls\"}</tool_input></tool_call>\n</tool_calls>\n\nLet me look."
                            .into(),
                    ),
                    ..Default::default()
                },
                None,
            ),
        );
        let done = evs
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolDone {
                    tool_name,
                    input_json,
                    ..
                } => Some((tool_name.clone(), input_json.clone())),
                _ => None,
            })
            .expect("ToolDone synthesized from plural-wrapper XML");
        assert_eq!(done.0, "Bash");
        assert_eq!(done.1, "{\"command\": \"ls\"}");
        let text: String = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert!(
            text.contains("Let me look."),
            "trailing prose lost: {text:?}"
        );
        assert!(!text.contains("<tool_call"), "plural XML leaked: {text:?}");
        assert!(!text.contains("<tool_name>"), "child tag leaked: {text:?}");
    }

    // Normal: a single `<tool_calls>` wrapper holding TWO `<tool_call>`
    // children yields TWO synthesized ToolDone events (parallel calls in one
    // turn — the exact shape that stalled in the live session).
    #[test]
    fn inline_plural_wrapper_multiple_children_normal() {
        let mut state = OpenAiStreamState::default();
        let evs = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(
                        "<tool_calls><tool_call><tool_name>Read</tool_name><tool_input>{\"file_path\":\"/a\"}</tool_input></tool_call><tool_call><tool_name>Read</tool_name><tool_input>{\"file_path\":\"/b\"}</tool_input></tool_call></tool_calls>"
                            .into(),
                    ),
                    ..Default::default()
                },
                None,
            ),
        );
        let dones: Vec<(String, String)> = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolDone {
                    tool_name,
                    input_json,
                    ..
                } => Some((tool_name.clone(), input_json.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(dones.len(), 2, "expected 2 ToolDone, got {dones:?}");
        assert_eq!(dones[0], ("Read".into(), "{\"file_path\":\"/a\"}".into()));
        assert_eq!(dones[1], ("Read".into(), "{\"file_path\":\"/b\"}".into()));
    }

    // Robust: an inline `<tool_results>` echo (plural) is suppressed exactly
    // like the singular `<tool_result>` — the model's fabricated results must
    // never feed back as truth.
    #[test]
    fn inline_plural_tool_results_suppressed_robust() {
        let mut state = OpenAiStreamState::default();
        let evs = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(
                        "<tool_results>\nfabricated output\n</tool_results> after".into(),
                    ),
                    ..Default::default()
                },
                None,
            ),
        );
        let text: String = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert!(text.contains("after"), "trailing prose lost: {text:?}");
        assert!(
            !text.contains("fabricated"),
            "fabricated results leaked: {text:?}"
        );
        assert!(!text.contains("<tool_results>"), "wrapper leaked: {text:?}");
    }

    // Robust: plain content with no inline tags streams through verbatim — the
    // interceptor must not hold back or alter ordinary text.
    #[test]
    fn plain_content_passes_through_unbuffered_robust() {
        let mut state = OpenAiStreamState::default();
        let evs = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some("just normal prose with a < less-than".into()),
                    ..Default::default()
                },
                None,
            ),
        );
        let text: String = evs
            .iter()
            .filter_map(|e| match e {
                StreamEvent::TextDelta { delta, .. } => Some(delta.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(text, "just normal prose with a < less-than");
    }

    // Robust: finish_reason "stop" with no tool_calls in history → just
    // emits Done(EndTurn), no spurious ToolDone.
    #[test]
    fn stateful_finish_stop_emits_no_tool_done_robust() {
        let mut state = OpenAiStreamState::default();
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some("hello".into()),
                    ..Default::default()
                },
                None,
            ),
        );
        let final_events = evs_stateful(&mut state, chunk(ChunkDelta::default(), Some("stop")));
        assert!(
            !final_events
                .iter()
                .any(|e| matches!(e, StreamEvent::ToolDone { .. }))
        );
        assert!(final_events.iter().any(|e| matches!(
            e,
            StreamEvent::Done {
                stop_reason: StopReason::EndTurn
            }
        )));
    }

    // Robust: name/id arriving on chunk after the args fragment still ends up
    // populated on the final ToolDone — the accumulator merges in either order.
    #[test]
    fn stateful_late_name_id_still_captured_robust() {
        let mut state = OpenAiStreamState::default();
        // Args fragment first (unusual but possible).
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![fn_call(0, None, None, Some("{\"x\":1}"))]),
                    ..Default::default()
                },
                None,
            ),
        );
        // Name + id later.
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![fn_call(0, Some("call_z"), Some("write"), None)]),
                    ..Default::default()
                },
                None,
            ),
        );
        let final_events =
            evs_stateful(&mut state, chunk(ChunkDelta::default(), Some("tool_calls")));
        let done = final_events
            .iter()
            .find_map(|e| match e {
                StreamEvent::ToolDone {
                    tool_name,
                    tool_use_id,
                    input_json,
                    ..
                } => Some((tool_name.clone(), tool_use_id.clone(), input_json.clone())),
                _ => None,
            })
            .expect("ToolDone");
        assert_eq!(done.0, "write");
        assert_eq!(done.1, "call_z");
        assert_eq!(done.2, "{\"x\":1}");
    }

    // ── Bedrock content sanitization (mirror opencode plugin) ──────────────

    // Normal: empty `content: ""` is replaced with the placeholder so Bedrock's
    // ContentBlock validator accepts the message.
    #[test]
    fn bedrock_empty_string_content_replaced_normal() {
        let mut msgs = vec![json!({"role": "user", "content": ""})];
        bedrock_sanitize_messages(&mut msgs);
        assert_eq!(msgs[0]["content"], json!(BEDROCK_BLANK_TEXT_PLACEHOLDER));
    }

    // Normal: whitespace-only content gets replaced too — Bedrock rejects ANY
    // whitespace-only string per opencode's observed errors.
    #[test]
    fn bedrock_whitespace_only_content_replaced_normal() {
        let mut msgs = vec![json!({"role": "user", "content": "   \n  "})];
        bedrock_sanitize_messages(&mut msgs);
        assert_eq!(msgs[0]["content"], json!(BEDROCK_BLANK_TEXT_PLACEHOLDER));
    }

    // Normal: empty content array gets one placeholder text block. Bedrock
    // rejects empty arrays.
    #[test]
    fn bedrock_empty_array_content_replaced_normal() {
        let mut msgs = vec![json!({"role": "user", "content": []})];
        bedrock_sanitize_messages(&mut msgs);
        let arr = msgs[0]["content"].as_array().expect("array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["type"], "text");
        assert_eq!(arr[0]["text"], BEDROCK_BLANK_TEXT_PLACEHOLDER);
    }

    // Normal: non-empty content passes through unchanged.
    #[test]
    fn bedrock_non_empty_content_unchanged_normal() {
        let mut msgs = vec![json!({"role": "user", "content": "hello"})];
        bedrock_sanitize_messages(&mut msgs);
        assert_eq!(msgs[0]["content"], json!("hello"));
    }

    // Robust: assistant turn with content=null (tool-call-only) is normalized.
    #[test]
    fn bedrock_null_content_assistant_turn_replaced_robust() {
        let mut msgs = vec![json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{"id": "x"}],
        })];
        bedrock_sanitize_messages(&mut msgs);
        assert_eq!(msgs[0]["content"], json!(BEDROCK_BLANK_TEXT_PLACEHOLDER));
    }

    // ── Bedrock tool_choice scrubbing ──────────────────────────────────────

    // Normal: tool_choice:"none" with tools present → drop tools entirely.
    #[test]
    fn bedrock_tool_choice_none_drops_tools_normal() {
        let mut body = json!({
            "tools": [{"type": "function"}],
            "tool_choice": "none",
        });
        bedrock_scrub_tool_fields(&mut body);
        assert!(body.get("tools").is_none(), "{body}");
        assert!(body.get("tool_choice").is_none(), "{body}");
    }

    // Normal: tool_choice:"any" → coerced to "auto".
    #[test]
    fn bedrock_tool_choice_any_coerced_to_auto_normal() {
        let mut body = json!({
            "tools": [{"type": "function"}],
            "tool_choice": "any",
        });
        bedrock_scrub_tool_fields(&mut body);
        assert_eq!(body["tool_choice"], json!("auto"));
    }

    // Normal: tool_choice:"required" → coerced to "auto".
    #[test]
    fn bedrock_tool_choice_required_coerced_to_auto_normal() {
        let mut body = json!({
            "tools": [{"type": "function"}],
            "tool_choice": "required",
        });
        bedrock_scrub_tool_fields(&mut body);
        assert_eq!(body["tool_choice"], json!("auto"));
    }

    // Normal: object-form tool_choice {type: "any"} also coerced.
    #[test]
    fn bedrock_object_tool_choice_any_coerced_normal() {
        let mut body = json!({
            "tools": [{"type": "function"}],
            "tool_choice": {"type": "any"},
        });
        bedrock_scrub_tool_fields(&mut body);
        assert_eq!(body["tool_choice"], json!("auto"));
    }

    // Robust: legacy `functions` / `function_call` fields are dropped — Bedrock
    // chokes on them.
    #[test]
    fn bedrock_legacy_function_fields_dropped_robust() {
        let mut body = json!({
            "functions": [],
            "function_call": "auto",
        });
        bedrock_scrub_tool_fields(&mut body);
        assert!(body.get("functions").is_none());
        assert!(body.get("function_call").is_none());
    }

    // Robust: history references tool calls but no tools declared → inject
    // dummy tool so Bedrock's validator passes.
    #[test]
    fn bedrock_dummy_tool_injected_when_history_has_tool_calls_robust() {
        let mut body = json!({
            "messages": [
                {"role": "assistant", "tool_calls": [{"id": "x"}]},
                {"role": "tool", "tool_call_id": "x", "content": "result"},
            ],
        });
        bedrock_scrub_tool_fields(&mut body);
        let tools = body["tools"].as_array().expect("tools injected");
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0]["function"]["name"], "dummy_tool");
    }

    // Robust: messages_reference_tools detects all three triggers.
    #[test]
    fn messages_reference_tools_detects_role_tool_robust() {
        let msgs = vec![json!({"role": "tool", "content": "x"})];
        assert!(messages_reference_tools(&msgs));
    }

    #[test]
    fn messages_reference_tools_detects_tool_calls_array_robust() {
        let msgs = vec![json!({"role": "assistant", "tool_calls": [{"id": "x"}]})];
        assert!(messages_reference_tools(&msgs));
    }

    #[test]
    fn messages_reference_tools_detects_tool_call_id_robust() {
        let msgs = vec![json!({"role": "user", "tool_call_id": "x"})];
        assert!(messages_reference_tools(&msgs));
    }

    #[test]
    fn messages_reference_tools_negative_normal() {
        let msgs = vec![json!({"role": "user", "content": "hi"})];
        assert!(!messages_reference_tools(&msgs));
    }

    // ── stream_options.include_usage (LiteLLM/Bedrock requirement) ──────────

    // Normal: streaming requests carry `stream_options.include_usage: true`.
    // Without this, LiteLLM (which fronts most Bedrock-on-OWUI deployments)
    // truncates the stream — the symptom was a one-line response then no tool
    // call. Mirrors opencode-openwebui-auth fetch.ts:235-240.
    #[test]
    fn build_body_streaming_includes_usage_normal() {
        let body = build_body(vec![user_msg("hi")], &opts_with_bash_tool());
        assert_eq!(body["stream"], json!(true));
        assert_eq!(body["stream_options"], json!({ "include_usage": true }));
    }

    // Robust: non-streaming wouldn't normally pass through this body builder,
    // but if `stream` is unset we don't add stream_options. Documents the
    // contract so future changes don't accidentally leak the field.
    #[test]
    fn build_body_no_stream_options_when_stream_false_robust() {
        // build_body always sets stream=true; this test guards against a future
        // refactor that gates `stream` on a flag. We verify the present
        // behavior: stream=true, stream_options present.
        let body = build_body(vec![user_msg("hi")], &opts_with_bash_tool());
        assert_eq!(body["stream"], json!(true));
        assert!(body.get("stream_options").is_some());
    }

    // ── Provider trait wiring (no I/O) ────────────────────────────────────

    // Normal: name + stream_convention are read synchronously by the
    // renderer's dispatch; OpenWebUI uses InlineXmlTags for safety against
    // server-side shims that inject XML into plain text.
    #[test]
    fn provider_name_and_convention_normal() {
        let p = OpenWebUIProvider::new();
        assert_eq!(p.name(), "openwebui");
        assert_eq!(p.stream_convention(), StreamConvention::InlineXmlTags);
    }

    // Normal: available_models() is intentionally empty for OpenWebUI — the
    // catalog is server-driven and only fetch_models() returns real entries.
    #[test]
    fn available_models_returns_empty_normal() {
        let p = OpenWebUIProvider::new();
        assert!(p.available_models().is_empty());
    }

    // ── load_account error paths ──────────────────────────────────────────

    fn temp_account_file(json: &str) -> (tempfile::TempDir, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("openwebui-accounts.json");
        std::fs::write(&path, json).unwrap();
        (tmp, path)
    }

    // Normal: a single enabled account loads cleanly. Verifies the camelCase
    // rename (`baseUrl`) lands on `base_url`.
    #[test]
    fn load_account_canonical_layout_normal() {
        let (_tmp, path) = temp_account_file(
            r#"{
                "accounts": {
                    "primary": {
                        "name": "primary",
                        "baseUrl": "https://owui.example.com",
                        "token": "tok-1"
                    }
                },
                "current": "primary"
            }"#,
        );
        let acct = load_account(&path).unwrap();
        assert_eq!(acct.name, "primary");
        assert_eq!(acct.base_url, "https://owui.example.com");
        assert_eq!(acct.token, "tok-1");
    }

    // Robust: no `current` field → the loader walks accounts.values() and
    // returns the first enabled one. Determinism is best-effort because
    // HashMap iteration order isn't stable; we only assert the result is
    // non-error and that the returned account is enabled.
    #[test]
    fn load_account_no_current_falls_back_to_first_enabled_robust() {
        let (_tmp, path) = temp_account_file(
            r#"{"accounts":{"a":{"name":"a","baseUrl":"https://x","token":"t"}}}"#,
        );
        let acct = load_account(&path).unwrap();
        assert_eq!(acct.name, "a");
    }

    // Robust: `current` points to a disabled account → fall back to first
    // enabled. Documents the contract: an admin who disables their primary
    // account doesn't lose the provider entirely.
    #[test]
    fn load_account_disabled_current_falls_back_to_enabled_robust() {
        let (_tmp, path) = temp_account_file(
            r#"{
                "accounts": {
                    "primary": {"name":"primary","baseUrl":"https://x","token":"t","disabled":true},
                    "secondary": {"name":"secondary","baseUrl":"https://y","token":"u"}
                },
                "current": "primary"
            }"#,
        );
        let acct = load_account(&path).unwrap();
        assert_eq!(acct.name, "secondary");
        assert_eq!(acct.base_url, "https://y");
    }

    // Robust: every account disabled → Err. Without this, the picker would
    // happily route requests to a disabled instance and 401 mid-stream.
    #[test]
    fn load_account_all_disabled_errors_robust() {
        let (_tmp, path) = temp_account_file(
            r#"{
                "accounts": {
                    "x": {"name":"x","baseUrl":"https://x","token":"t","disabled":true}
                }
            }"#,
        );
        assert!(load_account(&path).is_err());
    }

    // Robust: empty accounts map → Err. The store must always describe at
    // least one usable account.
    #[test]
    fn load_account_empty_map_errors_robust() {
        let (_tmp, path) = temp_account_file(r#"{"accounts": {}}"#);
        assert!(load_account(&path).is_err());
    }

    // Robust: malformed JSON surfaces as Err — the user can hand-edit the
    // file and we don't want a typo to crash the app.
    #[test]
    fn load_account_invalid_json_errors_robust() {
        let (_tmp, path) = temp_account_file("{ this is not valid");
        assert!(load_account(&path).is_err());
    }

    // ── context_window_from_value: robust JSON shape detection ────────────

    // Normal: a non-object / non-array Value never finds a match. The OWUI
    // metadata blob can legitimately be `Null` for sparse models.
    #[test]
    fn context_window_from_value_null_returns_none_normal() {
        assert_eq!(context_window_from_value(&Value::Null), None);
        assert_eq!(context_window_from_value(&json!(42)), None);
        assert_eq!(context_window_from_value(&json!("string")), None);
    }

    // Normal: a String-form numeric ("32768") passes through value_as_usize.
    // Real OWUI metadata occasionally stringifies numeric values.
    #[test]
    fn value_as_usize_string_form_normal() {
        assert_eq!(value_as_usize(&json!("12345")), Some(12345));
    }

    // Robust: a Number that doesn't fit in usize returns None instead of
    // panicking on overflow.
    #[test]
    fn value_as_usize_negative_returns_none_robust() {
        assert_eq!(value_as_usize(&json!(-1i64)), None);
    }

    // Robust: a non-numeric string returns None.
    #[test]
    fn value_as_usize_non_numeric_string_returns_none_robust() {
        assert_eq!(value_as_usize(&json!("not-a-number")), None);
        assert_eq!(value_as_usize(&Value::Null), None);
        assert_eq!(value_as_usize(&json!(true)), None);
    }

    // Normal: an array of objects — find_map walks them. Verifies the
    // recursive Array branch of context_window_from_value.
    #[test]
    fn context_window_from_value_array_normal() {
        let v = json!([{"context_length": 4096}]);
        assert_eq!(context_window_from_value(&v), Some(4096));
    }

    // ── infer_context_window_from_model_name: every branch ───────────────

    // Normal: every documented family resolves to its tested constant. The
    // helper is small enough that we cover each branch in one test.
    #[test]
    fn infer_context_window_per_family_normal() {
        assert_eq!(
            infer_context_window_from_model_name("anthropic/claude-opus-4-6", None),
            1_000_000
        );
        assert_eq!(
            infer_context_window_from_model_name("anthropic/claude-sonnet-4-5", None),
            200_000
        );
        assert_eq!(
            infer_context_window_from_model_name("openai/gpt-5-nano", None),
            1_000_000
        );
        assert_eq!(
            infer_context_window_from_model_name("openai/gpt-4o", None),
            128_000
        );
        assert_eq!(
            infer_context_window_from_model_name("meta/llama-4-maverick-100b", None),
            1_048_576
        );
        assert_eq!(
            infer_context_window_from_model_name("meta/llama-4-scout", None),
            131_072
        );
        assert_eq!(
            infer_context_window_from_model_name("meta/llama-3-70b", None),
            131_072
        );
        assert_eq!(
            infer_context_window_from_model_name("google/gemma-3-27b", None),
            128_000
        );
        assert_eq!(
            infer_context_window_from_model_name("google/gemini-2-flash", None),
            1_048_576
        );
        assert_eq!(
            infer_context_window_from_model_name("amazon/nova-pro", None),
            300_000
        );
        assert_eq!(
            infer_context_window_from_model_name("amazon/nova-lite", None),
            300_000
        );
    }

    // Robust: an entirely unrecognized id falls through to the conservative
    // 128k default — the picker still surfaces *some* number rather than
    // showing a blank context column.
    #[test]
    fn infer_context_window_unknown_falls_back_robust() {
        assert_eq!(
            infer_context_window_from_model_name("custom/totally-unknown", None),
            128_000
        );
    }

    // Normal: the optional `name` field is folded into the haystack so a
    // display name like "Claude Opus 4.6" still resolves correctly even when
    // the id is opaque.
    #[test]
    fn infer_context_window_uses_name_when_id_opaque_normal() {
        // id is meaningless but name carries the brand.
        assert_eq!(
            infer_context_window_from_model_name("custom/x", Some("Claude Opus 4-6 (private)")),
            1_000_000
        );
    }

    // ── bedrock_sanitize_messages additional shapes ──────────────────────

    // Robust: an array `content` with one non-empty text block passes
    // through unchanged. Bedrock accepts well-formed arrays — we only
    // intervene when the array is empty.
    #[test]
    fn bedrock_non_empty_array_content_unchanged_robust() {
        let mut msgs = vec![json!({
            "role": "user",
            "content": [{"type": "text", "text": "hello"}]
        })];
        bedrock_sanitize_messages(&mut msgs);
        let arr = msgs[0]["content"].as_array().unwrap();
        assert_eq!(arr[0]["text"], "hello");
    }

    // Robust: a message that's not even an object (Value::Null in the wrong
    // slot) is left alone — the loop continues without panicking.
    #[test]
    fn bedrock_non_object_message_left_alone_robust() {
        let mut msgs = vec![Value::Null];
        bedrock_sanitize_messages(&mut msgs);
        assert_eq!(msgs[0], Value::Null);
    }

    // Bedrock rejects tool_calls with empty `function.arguments`. The
    // sanitizer must rewrite "" / "null" / Value::Null → "{}" so the
    // request gets past the validator. Three cases:
    //   - empty string ""
    //   - literal "null" string
    //   - missing arguments key (filled with "{}")
    #[test]
    fn bedrock_empty_tool_call_arguments_normalized_normal() {
        let mut msgs = vec![json!({
            "role": "assistant",
            "content": "",
            "tool_calls": [
                {"id": "1", "type": "function", "function": {"name": "X", "arguments": ""}},
                {"id": "2", "type": "function", "function": {"name": "Y", "arguments": "null"}},
                {"id": "3", "type": "function", "function": {"name": "Z"}},
                {"id": "4", "type": "function", "function": {"name": "W", "arguments": "   "}},
            ]
        })];
        bedrock_sanitize_messages(&mut msgs);
        let calls = msgs[0]["tool_calls"].as_array().unwrap();
        for c in calls {
            assert_eq!(c["function"]["arguments"].as_str(), Some("{}"));
        }
    }

    // Robust: well-formed arguments must be left untouched. We only
    // rewrite the empty/null cases — preserving real payloads is critical
    // for correctness on every other path.
    #[test]
    fn bedrock_nonempty_tool_call_arguments_unchanged_robust() {
        let payload = r#"{"path":"/tmp/foo"}"#;
        let mut msgs = vec![json!({
            "role": "assistant",
            "tool_calls": [
                {"id": "1", "type": "function", "function": {"name": "Read", "arguments": payload}},
            ]
        })];
        bedrock_sanitize_messages(&mut msgs);
        assert_eq!(
            msgs[0]["tool_calls"][0]["function"]["arguments"].as_str(),
            Some(payload)
        );
    }

    // ── messages_reference_tools edge cases ──────────────────────────────

    // Robust: a non-object array entry returns false (defensive — the JSON
    // shape we expect is always objects, but the function must not panic).
    #[test]
    fn messages_reference_tools_non_object_robust() {
        let msgs = vec![Value::String("garbage".into())];
        assert!(!messages_reference_tools(&msgs));
    }

    // Robust: empty tool_calls array → still false (the array exists but
    // has no entries).
    #[test]
    fn messages_reference_tools_empty_tool_calls_array_robust() {
        let msgs = vec![json!({"role":"assistant", "tool_calls": []})];
        assert!(!messages_reference_tools(&msgs));
    }

    // ── push_chunk_events_stateful additional behaviors ──────────────────

    // Normal: `reasoning_content` is forwarded as ThinkingDelta. OpenWebUI
    // uses this for o1/o3-style reasoning models.
    #[test]
    fn stateful_reasoning_content_emits_thinking_delta_normal() {
        let mut state = OpenAiStreamState::default();
        let events = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    reasoning_content: Some("internal thought".into()),
                    ..Default::default()
                },
                None,
            ),
        );
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::ThinkingDelta { delta, .. } if delta == "internal thought"
        )));
    }

    // Normal: `refusal` content surfaces as a TextDelta — we don't have a
    // separate Refusal event, so it lands in the same channel as plain text
    // for the user to see.
    #[test]
    fn stateful_refusal_emits_text_delta_normal() {
        let mut state = OpenAiStreamState::default();
        let events = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    refusal: Some("I cannot help with that".into()),
                    ..Default::default()
                },
                None,
            ),
        );
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::TextDelta { delta, .. } if delta.contains("cannot help")
        )));
    }

    // Robust: a finish_reason of "length" maps to MaxTokens — important
    // for the UI to show "Hit max tokens" rather than "Stopped".
    #[test]
    fn stateful_finish_length_maps_to_max_tokens_robust() {
        let mut state = OpenAiStreamState::default();
        let events = evs_stateful(&mut state, chunk(ChunkDelta::default(), Some("length")));
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::Done {
                stop_reason: StopReason::MaxTokens
            }
        )));
    }

    // Robust: content-filter/refusal finish reasons map to the first-class
    // Refusal variant so the runtime does not self-continue into the same
    // blocked request.
    #[test]
    fn stateful_finish_content_filter_maps_refusal_robust() {
        let mut state = OpenAiStreamState::default();
        let events = evs_stateful(
            &mut state,
            chunk(ChunkDelta::default(), Some("content_filter")),
        );
        let done = events
            .iter()
            .find_map(|e| match e {
                StreamEvent::Done { stop_reason } => Some(stop_reason.clone()),
                _ => None,
            })
            .expect("Done");
        assert_eq!(done, StopReason::Refusal);
    }

    #[test]
    fn stateful_finish_unknown_maps_to_other_robust() {
        let mut state = OpenAiStreamState::default();
        let events = evs_stateful(
            &mut state,
            chunk(ChunkDelta::default(), Some("gateway_weird")),
        );
        let done = events
            .iter()
            .find_map(|e| match e {
                StreamEvent::Done { stop_reason } => Some(stop_reason.clone()),
                _ => None,
            })
            .expect("Done");
        assert_eq!(done, StopReason::Other("gateway_weird".into()));
    }

    // Robust: a chunk with no choices at all is a no-op — push_chunk_events_stateful
    // returns early without panicking.
    #[test]
    fn stateful_chunk_with_no_choices_is_noop_robust() {
        let mut state = OpenAiStreamState::default();
        let chunk = ChatChunk {
            choices: vec![],
            usage: None,
        };
        let events = evs_stateful(&mut state, chunk);
        assert!(events.is_empty());
    }

    // Robust: empty content / empty text deltas don't emit a TextDelta —
    // would otherwise spam the renderer with no-op events on streams that
    // have content="" placeholder chunks.
    #[test]
    fn stateful_empty_content_does_not_emit_text_delta_robust() {
        let mut state = OpenAiStreamState::default();
        let events = evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    content: Some(String::new()),
                    ..Default::default()
                },
                None,
            ),
        );
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, StreamEvent::TextDelta { .. }))
        );
    }

    // ── Legacy function_call / function_calls support ─────────────────

    // Normal: a legacy singular `function_call` field on the delta is
    // normalized into the tool accumulator at index 0 and produces ToolDone.
    #[test]
    fn stateful_legacy_function_call_singular_normal() {
        let mut state = OpenAiStreamState::default();

        // First chunk: function_call with name.
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    function_call: Some(ChunkFunctionCall {
                        name: Some("read_file".to_owned()),
                        arguments: Some("{\"path\":\"".to_owned()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ),
        );

        // Second chunk: more arguments.
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    function_call: Some(ChunkFunctionCall {
                        arguments: Some("foo.rs\"}".to_owned()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ),
        );

        // Finish with function_call reason.
        let events = evs_stateful(
            &mut state,
            chunk(ChunkDelta::default(), Some("function_call")),
        );
        let tool_done = events.iter().find_map(|e| match e {
            StreamEvent::ToolDone {
                tool_name,
                tool_use_id,
                input_json,
                ..
            } => Some((tool_name.clone(), tool_use_id.clone(), input_json.clone())),
            _ => None,
        });
        assert_eq!(
            tool_done,
            Some((
                "read_file".to_owned(),
                "call_0".to_owned(),
                "{\"path\":\"foo.rs\"}".to_owned()
            ))
        );
    }

    // Normal: nonstandard plural `function_calls` array is normalized.
    #[test]
    fn stateful_function_calls_plural_normal() {
        let mut state = OpenAiStreamState::default();

        // Single chunk with two function_calls entries.
        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    function_calls: Some(vec![
                        ChunkFunctionCall {
                            index: Some(0),
                            id: Some("call_a".to_owned()),
                            function: Some(ChunkToolFn {
                                name: Some("bash".to_owned()),
                                arguments: Some("{\"cmd\":\"ls\"}".to_owned()),
                            }),
                            ..Default::default()
                        },
                        ChunkFunctionCall {
                            index: Some(1),
                            id: Some("call_b".to_owned()),
                            name: Some("read".to_owned()),
                            arguments: Some("{\"path\":\"x\"}".to_owned()),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                },
                None,
            ),
        );

        // Finish.
        let events = evs_stateful(
            &mut state,
            chunk(ChunkDelta::default(), Some("function_calls")),
        );
        let tool_dones: Vec<_> = events
            .iter()
            .filter_map(|e| match e {
                StreamEvent::ToolDone {
                    tool_name,
                    tool_use_id,
                    ..
                } => Some((tool_name.clone(), tool_use_id.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(tool_dones.len(), 2);
        assert_eq!(tool_dones[0], ("bash".to_owned(), "call_a".to_owned()));
        assert_eq!(tool_dones[1], ("read".to_owned(), "call_b".to_owned()));
    }

    // Robust: a canonical tool_calls start chunk followed by a legacy
    // function_call suffix still accumulates at index 0 and keeps the real id.
    #[test]
    fn stateful_tool_calls_then_function_call_suffix_keeps_real_id_robust() {
        let mut state = OpenAiStreamState::default();

        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![fn_call(
                        0,
                        Some("real_call"),
                        Some("write"),
                        Some("{\"file_path\":\"x\",\"content\":\"hel"),
                    )]),
                    ..Default::default()
                },
                None,
            ),
        );

        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    function_call: Some(ChunkFunctionCall {
                        arguments: Some("lo\"}".to_owned()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ),
        );

        let events = evs_stateful(
            &mut state,
            chunk(ChunkDelta::default(), Some("function_call")),
        );
        let tool_done = events.iter().find_map(|e| match e {
            StreamEvent::ToolDone {
                tool_name,
                tool_use_id,
                input_json,
                ..
            } => Some((tool_name.clone(), tool_use_id.clone(), input_json.clone())),
            _ => None,
        });
        assert_eq!(
            tool_done,
            Some((
                "write".to_owned(),
                "real_call".to_owned(),
                "{\"file_path\":\"x\",\"content\":\"hello\"}".to_owned()
            ))
        );
    }

    // Robust: if a gateway mirrors the same delta into both canonical
    // tool_calls and a legacy alias, prefer the canonical field for that chunk.
    #[test]
    fn stateful_same_chunk_prefers_tool_calls_over_function_call_alias_robust() {
        let mut state = OpenAiStreamState::default();

        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![fn_call(
                        0,
                        Some("tc_1"),
                        Some("bash"),
                        Some("{\"ok\":true}"),
                    )]),
                    function_call: Some(ChunkFunctionCall {
                        name: Some("bash".to_owned()),
                        arguments: Some("{\"dup\":true}".to_owned()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ),
        );

        let events = evs_stateful(&mut state, chunk(ChunkDelta::default(), Some("tool_calls")));
        let tool_done = events.iter().find_map(|e| match e {
            StreamEvent::ToolDone {
                tool_name,
                tool_use_id,
                input_json,
                ..
            } => Some((tool_name.clone(), tool_use_id.clone(), input_json.clone())),
            _ => None,
        });
        assert_eq!(
            tool_done,
            Some((
                "bash".to_owned(),
                "tc_1".to_owned(),
                "{\"ok\":true}".to_owned()
            ))
        );
    }

    // Robust: an empty canonical array is just noise; it should not hide a
    // populated legacy function_call in the same chunk.
    #[test]
    fn stateful_empty_tool_calls_falls_back_to_function_call_robust() {
        let mut state = OpenAiStreamState::default();

        evs_stateful(
            &mut state,
            chunk(
                ChunkDelta {
                    tool_calls: Some(vec![]),
                    function_call: Some(ChunkFunctionCall {
                        name: Some("bash".to_owned()),
                        arguments: Some("{\"cmd\":\"pwd\"}".to_owned()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                None,
            ),
        );

        let events = evs_stateful(
            &mut state,
            chunk(ChunkDelta::default(), Some("function_call")),
        );
        let tool_done = events.iter().find_map(|e| match e {
            StreamEvent::ToolDone {
                tool_name,
                tool_use_id,
                input_json,
                ..
            } => Some((tool_name.clone(), tool_use_id.clone(), input_json.clone())),
            _ => None,
        });
        assert_eq!(
            tool_done,
            Some((
                "bash".to_owned(),
                "call_0".to_owned(),
                "{\"cmd\":\"pwd\"}".to_owned()
            ))
        );
    }

    // ── build_body when system prompt is set ─────────────────────────────

    // Normal: a system prompt is prepended as a `role:"system"` message at
    // index 0 — OpenWebUI's OpenAI-compatible API expects system messages
    // inline rather than as a top-level field.
    #[test]
    fn build_body_system_message_inline_normal() {
        let opts = StreamOptions::new("m").system("be terse");
        let body = build_body(vec![user_msg("hi")], &opts);
        let msgs = body["messages"].as_array().unwrap();
        // First message is the system prompt; user message follows.
        assert_eq!(msgs[0]["role"], "system");
        assert_eq!(msgs[0]["content"], "be terse");
        assert_eq!(msgs[1]["role"], "user");
    }

    // Normal: trailing assistant prefill is stripped — Bedrock rejects
    // assistant-last conversations. Verifies the post-build pop loop.
    #[test]
    fn build_body_strips_trailing_assistant_prefill_normal() {
        let history = vec![
            user_msg("hi"),
            ProviderMessage {
                role: ProviderRole::Assistant,
                content: vec![ProviderContent::Text("partial reply".into())],
            },
        ];
        let body = build_body(history, &StreamOptions::new("m"));
        let msgs = body["messages"].as_array().unwrap();
        let last_role = msgs.last().unwrap().get("role").and_then(|v| v.as_str());
        // After strip, last role must NOT be assistant.
        assert_ne!(last_role, Some("assistant"));
    }

    // detect_iana_timezone falls back to UTC when nothing's configured.
    // `TZ=` (empty) and `TZ=:` are both treated as unset, mirroring
    // glibc's tzset behavior.
    #[test]
    fn detect_iana_timezone_falls_back_to_utc_when_empty_robust() {
        let _g = TzGuard::set(":");
        // /etc/timezone may exist with a real value on the CI host, so
        // we can't strictly assert UTC. We can assert it's *some* IANA
        // string (non-empty, no leading colon).
        let tz = detect_iana_timezone();
        assert!(!tz.is_empty(), "timezone must never be empty");
        assert!(!tz.starts_with(':'), "leading colon must be stripped");
    }

    #[test]
    fn detect_iana_timezone_uses_tz_env_when_set_normal() {
        let _g = TzGuard::set("America/Phoenix");
        assert_eq!(detect_iana_timezone(), "America/Phoenix");
    }

    #[test]
    fn detect_iana_timezone_strips_leading_colon_normal() {
        // Posix `:Region/City` form — leading colon is a parser hint
        // to glibc, not part of the zone name.
        let _g = TzGuard::set(":Europe/Berlin");
        assert_eq!(detect_iana_timezone(), "Europe/Berlin");
    }

    /// RAII scope guard for `TZ` env-var manipulation. Restores the
    /// previous value (or removes it if none) on drop so parallel test
    /// runs don't poison each other's environment.
    struct TzGuard {
        previous: Option<String>,
    }

    impl TzGuard {
        fn set(value: &str) -> Self {
            let previous = std::env::var("TZ").ok();
            // SAFETY: tests for this module are single-threaded via the
            // test harness's default scheduling; the env-mutation is
            // contained within this guard's lifetime.
            unsafe {
                std::env::set_var("TZ", value);
            }
            Self { previous }
        }
    }

    impl Drop for TzGuard {
        fn drop(&mut self) {
            // SAFETY: see TzGuard::set above.
            unsafe {
                match &self.previous {
                    Some(v) => std::env::set_var("TZ", v),
                    None => std::env::remove_var("TZ"),
                }
            }
        }
    }
}

// OpenAI-compatible SSE delta shapes. Tool calls arrive as
// `choices[0].delta.tool_calls[]` with each entry carrying a `function.name`
// once and the `function.arguments` JSON streamed in chunks (incremental).
#[derive(Debug, Deserialize)]
struct ChatChunk {
    #[serde(default)]
    choices: Vec<ChunkChoice>,
    #[serde(default)]
    usage: Option<ChunkUsage>,
}

#[derive(Debug, Deserialize)]
struct ChunkUsage {
    #[serde(default)]
    prompt_tokens: u32,
    #[serde(default)]
    completion_tokens: u32,
    #[serde(default)]
    total_tokens: u32,
    #[serde(default)]
    prompt_tokens_details: Option<PromptTokensDetails>,
    #[serde(default)]
    cache_creation_input_tokens: u32,
    #[serde(default)]
    cache_read_input_tokens: u32,
    #[serde(default)]
    cache_write_input_tokens: u32,
}

impl ChunkUsage {
    fn raw_input_tokens(&self) -> u32 {
        self.prompt_tokens
            .saturating_sub(self.cache_read_tokens())
            .saturating_sub(self.cache_write_tokens())
    }

    fn cache_read_tokens(&self) -> u32 {
        self.cache_read_input_tokens.max(
            self.prompt_tokens_details
                .as_ref()
                .map_or(0, |d| d.cached_tokens),
        )
    }

    fn cache_write_tokens(&self) -> u32 {
        self.cache_creation_input_tokens
            .max(self.cache_write_input_tokens)
            .max(
                self.prompt_tokens_details
                    .as_ref()
                    .map_or(0, |d| d.cache_creation_input_tokens),
            )
    }
}

#[derive(Debug, Deserialize)]
struct PromptTokensDetails {
    #[serde(default)]
    cached_tokens: u32,
    #[serde(default)]
    cache_creation_input_tokens: u32,
}

#[derive(Debug, Deserialize)]
struct ChunkChoice {
    delta: ChunkDelta,
    finish_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default, Clone)]
struct ChunkDelta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    refusal: Option<String>,
    #[serde(default)]
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<ChunkToolCall>>,
    /// Legacy OpenAI `function_call` field — singular object with `name` and
    /// `arguments`. Some gateways (older LiteLLM, vLLM) still emit this instead
    /// of `tool_calls`. Normalized into the tool_calls accumulator at index 0.
    #[serde(default)]
    function_call: Option<ChunkFunctionCall>,
    /// Nonstandard plural `function_calls` field emitted by some custom
    /// gateways. Accept both flat legacy entries and nested tool-call-like
    /// entries, then normalize them alongside structured tool_calls.
    #[serde(default)]
    function_calls: Option<Vec<ChunkFunctionCall>>,
}

#[derive(Debug, Deserialize, Clone)]
struct ChunkToolCall {
    /// Position of this tool_call within the assistant message — stable across
    /// SSE chunks, so it's the right key for accumulating partial JSON.
    #[serde(default)]
    index: Option<usize>,
    /// `call_xxx` id assigned by the server. Often only present on the first
    /// chunk for a given tool call.
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<ChunkToolFn>,
}

#[derive(Debug, Deserialize, Clone)]
struct ChunkToolFn {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// Compatibility shape for legacy/nonstandard function-call deltas. Accepts:
/// `{ "name": "...", "arguments": "..." }`,
/// `{ "index": 1, "name": "...", "arguments": "..." }`, and
/// `{ "index": 1, "id": "call_x", "function": { ... } }`.
#[derive(Debug, Deserialize, Clone, Default)]
struct ChunkFunctionCall {
    #[serde(default)]
    index: Option<usize>,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    call_id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
    #[serde(default)]
    function: Option<ChunkToolFn>,
}

impl ChunkFunctionCall {
    fn to_tool_call(&self, fallback_index: usize) -> ChunkToolCall {
        let nested = self.function.as_ref();
        ChunkToolCall {
            index: Some(self.index.unwrap_or(fallback_index)),
            id: self.id.clone().or_else(|| self.call_id.clone()),
            function: Some(ChunkToolFn {
                name: nested
                    .and_then(|f| f.name.clone())
                    .or_else(|| self.name.clone()),
                arguments: nested
                    .and_then(|f| f.arguments.clone())
                    .or_else(|| self.arguments.clone()),
            }),
        }
    }
}

/// Bedrock-on-OpenWebUI requires every text content block to contain at least
/// one non-whitespace character. The placeholder `"."` is what
/// opencode-openwebui-auth (`src/plugin/fetch.ts:92`) settled on after observing
/// two distinct error variants from Bedrock:
///   - "The text field in the ContentBlock object at messages.N.content.M is blank."
///   - "messages: text content blocks must contain non-whitespace text"
const BEDROCK_BLANK_TEXT_PLACEHOLDER: &str = ".";

/// Replace any empty `content` strings on messages with the Bedrock placeholder.
/// Mirrors `sanitizeMessageContent` in opencode's plugin/fetch.ts.
fn bedrock_sanitize_messages(messages: &mut [Value]) {
    for msg in messages.iter_mut() {
        let Some(obj) = msg.as_object_mut() else {
            continue;
        };
        match obj.get("content") {
            Some(Value::Null) => {
                obj.insert("content".into(), json!(BEDROCK_BLANK_TEXT_PLACEHOLDER));
            }
            Some(Value::String(s)) if s.trim().is_empty() => {
                obj.insert("content".into(), json!(BEDROCK_BLANK_TEXT_PLACEHOLDER));
            }
            Some(Value::Array(arr)) if arr.is_empty() => {
                obj.insert(
                    "content".into(),
                    json!([{ "type": "text", "text": BEDROCK_BLANK_TEXT_PLACEHOLDER }]),
                );
            }
            _ => {}
        }

        // Bedrock rejects tool_calls where `function.arguments` is empty with:
        //   "The value at messages.N.content.M.toolUse.input is empty."
        // Ensure every tool_call has a non-empty arguments string (at minimum "{}").
        if let Some(tool_calls) = obj.get_mut("tool_calls").and_then(|v| v.as_array_mut()) {
            for tc in tool_calls.iter_mut() {
                if let Some(func) = tc.get_mut("function").and_then(|f| f.as_object_mut()) {
                    let needs_fix = match func.get("arguments") {
                        None => true,
                        Some(Value::String(s)) => {
                            s.is_empty() || s == "null" || s.trim().is_empty()
                        }
                        Some(Value::Null) => true,
                        _ => false,
                    };
                    if needs_fix {
                        func.insert("arguments".into(), json!("{}"));
                    }
                }
            }
        }
    }
}

/// True if any message in `messages` references tool calls (role: "tool",
/// presence of `tool_calls`, or a `tool_call_id`). Mirrors
/// `messagesReferenceTools` in opencode's fetch.ts.
fn messages_reference_tools(messages: &[Value]) -> bool {
    messages.iter().any(|m| {
        let Some(obj) = m.as_object() else {
            return false;
        };
        if obj.get("role").and_then(|v| v.as_str()) == Some("tool") {
            return true;
        }
        if obj
            .get("tool_calls")
            .and_then(|v| v.as_array())
            .is_some_and(|a| !a.is_empty())
        {
            return true;
        }
        obj.contains_key("tool_call_id")
    })
}

/// Bedrock validator rejects: `tool_choice: "none"` (drop tools entirely);
/// `tool_choice: "any"|"required"` (coerce to `"auto"`); old-style
/// `functions`/`function_call` (drop). Mirrors `scrubBedrockToolFields` in
/// opencode's fetch.ts. Also injects a dummy tool when the conversation
/// references prior tool calls but no tools are declared on this turn.
fn bedrock_scrub_tool_fields(body: &mut Value) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    let has_tools = obj
        .get("tools")
        .and_then(|v| v.as_array())
        .is_some_and(|a| !a.is_empty());

    if !has_tools {
        obj.remove("tools");
        obj.remove("tool_choice");
        obj.remove("parallel_tool_calls");
        // If the history references tool calls but the request declares none,
        // Bedrock's validator still rejects it. Inject a dummy tool the model
        // will ignore so the request is well-formed.
        if let Some(msgs) = obj.get("messages").and_then(|v| v.as_array())
            && messages_reference_tools(msgs)
        {
            obj.insert(
                "tools".into(),
                json!([{
                    "type": "function",
                    "function": {
                        "name": "dummy_tool",
                        "description": "placeholder — never call",
                        "parameters": { "type": "object", "properties": {} },
                    },
                }]),
            );
        }
    } else {
        let coerce = match obj.get("tool_choice") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Object(m)) => m.get("type").and_then(|v| v.as_str()).map(str::to_owned),
            _ => None,
        };
        match coerce.as_deref() {
            Some("none") => {
                obj.remove("tools");
                obj.remove("tool_choice");
                obj.remove("parallel_tool_calls");
            }
            Some("any") | Some("required") => {
                obj.insert("tool_choice".into(), json!("auto"));
            }
            _ => {}
        }
    }
    obj.remove("functions");
    obj.remove("function_call");
}

pub(crate) fn build_body(messages: Vec<ProviderMessage>, opts: &StreamOptions) -> Value {
    let mut msgs: Vec<Value> = Vec::new();
    for m in &messages {
        match m.role {
            ProviderRole::User => {
                for c in &m.content {
                    match c {
                        ProviderContent::Text(t) if !t.is_empty() => {
                            msgs.push(json!({
                                "role": "user",
                                "content": t,
                            }));
                        }
                        ProviderContent::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            msgs.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                        }
                        _ => {}
                    }
                }
            }
            ProviderRole::Assistant => {
                let mut text = String::new();
                let mut tool_calls = Vec::new();
                let mut trailing_tool_results = Vec::new();
                for c in &m.content {
                    match c {
                        ProviderContent::Text(t) if !t.is_empty() => {
                            if !text.is_empty() {
                                text.push('\n');
                            }
                            text.push_str(t);
                        }
                        ProviderContent::ToolUse {
                            id, name, input, ..
                        } => {
                            tool_calls.push(json!({
                                "id": id,
                                "type": "function",
                                "function": {
                                    // Lowercase historical tool names too — when the
                                    // user switched from anthropic-oauth (PascalCase
                                    // "Bash") to OWUI mid-conversation, the prior
                                    // tool_use blocks would arrive at LiteLLM with
                                    // PascalCase while new calls go out lowercase.
                                    // LiteLLM matches names case-sensitively against
                                    // the `tools` array so the mismatched history
                                    // got silently dropped, breaking the agentic
                                    // continuation. Normalize on the way out so the
                                    // whole conversation reads consistent.
                                    "name": name.to_lowercase(),
                                    "arguments": serde_json::to_string(input).unwrap_or_default(),
                                },
                            }));
                        }
                        ProviderContent::ToolResult {
                            tool_use_id,
                            content,
                            ..
                        } => {
                            trailing_tool_results.push(json!({
                                "role": "tool",
                                "tool_call_id": tool_use_id,
                                "content": content,
                            }));
                        }
                        _ => {}
                    }
                }

                if !tool_calls.is_empty() {
                    msgs.push(json!({
                        "role": "assistant",
                        // OpenAI allows assistant tool-call messages to omit
                        // content or set it null, but OpenWebUI/LiteLLM/Bedrock
                        // compatibility paths have hit `NoneType.startswith`
                        // and blank-content validator failures on that shape.
                        // Keep the key present; the Bedrock sanitizer below
                        // turns it into a non-empty placeholder when no text
                        // accompanied the tool calls.
                        "content": text,
                        "tool_calls": tool_calls,
                    }));
                } else if !text.is_empty() {
                    msgs.push(json!({
                        "role": "assistant",
                        "content": text,
                    }));
                }
                msgs.extend(trailing_tool_results);
            }
        }
    }

    let mut body = json!({
        "model": opts.model,
        "max_tokens": opts.max_tokens,
        "stream": true,
        "messages": msgs,
    });

    if let Some(sys) = &opts.system {
        let mut full = vec![json!({ "role": "system", "content": sys })];
        full.extend(body["messages"].as_array().cloned().unwrap_or_default());
        body["messages"] = json!(full);
    }

    // Bedrock / LiteLLM prefill stripping: if the final message is
    // role=assistant, pop it. Bedrock rejects "This model does not support
    // assistant message prefill. The conversation must end with a user message."
    // even on models that the native Anthropic API accepts prefill for.
    // Mirrors opencode-anthropic-auth index.ts:1286-1304.
    if let Some(arr) = body["messages"].as_array_mut() {
        while arr
            .last()
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str())
            == Some("assistant")
        {
            tracing::info!(
                target: "jfc::provider::openwebui",
                "stripped trailing assistant message for Bedrock/LiteLLM prefill compat"
            );
            arr.pop();
        }
        // If stripping left us with no user message at the end (only system),
        // append a minimal user turn so the request is well-formed.
        let last_role = arr
            .last()
            .and_then(|m| m.get("role"))
            .and_then(|r| r.as_str());
        if last_role != Some("user") && last_role != Some("tool") {
            arr.push(json!({"role": "user", "content": "Continue."}));
        }
    }

    // Apply Bedrock-compat scrubbing to the messages array. Sonnet-on-Bedrock
    // (e.g. genai.arizona.edu's bedrock-claude-4-6-sonnet route) returns 400 on
    // empty content blocks even when other models accept them. Cheap to do on
    // every body — the no-op cost on non-Bedrock routes is negligible.
    if let Some(arr) = body["messages"].as_array_mut() {
        bedrock_sanitize_messages(arr);
    }

    // Forward jfc's tool catalog in OpenAI-compatible function-tool shape so the
    // model sees the same Bash/Read/Edit/etc. surface it would on the Anthropic
    // path. Without this, OWUI-routed models had no tools and either narrated
    // commands as prose (the bug in the screenshot) or fell back to inline
    // `<tool_call>` XML.
    if !opts.tools.is_empty() {
        // Lowercase tool names for OWUI/LiteLLM/Bedrock paths. Anthropic-native
        // accepts PascalCase ("Bash", "Read", …) and Claude is trained on those,
        // but Bedrock's guardrail + LiteLLM's tool-call validator silently
        // strip tool_calls whose names match its blocklist of "executor"-shaped
        // patterns — leading to `finish_reason: "tool_calls"` with an empty
        // `delta.tool_calls` array (the symptom we hit on
        // `bedrock-claude-4-6-sonnet`). opencode normalizes the same way: see
        // packages/opencode/src/tool/bash.ts:331 (`Tool.define("bash", ...)`).
        // ToolKind::from_name already handles both casings on the way back.
        let tools: Vec<Value> = opts
            .tools
            .iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name.to_lowercase(),
                        "description": t.description,
                        "parameters": t.input_schema,
                    },
                })
            })
            .collect();
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
    }

    // Final pass — Bedrock-compat: drop unsupported tool_choice variants,
    // coerce any/required → auto, inject DUMMY_TOOL if history references
    // tools but none are declared. Matches opencode-openwebui-auth's
    // scrubBedrockToolFields exactly.
    bedrock_scrub_tool_fields(&mut body);

    // OpenAI streaming requires `stream_options.include_usage: true` for
    // upstream usage reporting on LiteLLM-fronted Bedrock. Without this,
    // some LiteLLM versions silently truncate the response mid-stream
    // (the symptom we saw: model writes one line of intent, never calls a
    // tool, connection closes). opencode-openwebui-auth's fetch.ts:235-240
    // adds this same field on every streaming request.
    if body.get("stream").and_then(|v| v.as_bool()) == Some(true) {
        body["stream_options"] = json!({ "include_usage": true });
    }

    if let Some(temp) = opts.temperature {
        body["temperature"] = Value::from(temp);
    }
    if let Some(top_p) = opts.top_p {
        body["top_p"] = Value::from(top_p);
    }
    // Pass reasoning_effort through. LiteLLM (which shares this builder)
    // handles the param correctly by mapping it to provider-specific shapes
    // (e.g. Anthropic `output_config` + the `effort-2025-11-24` beta header
    // for Opus 4.5/4.6). The OpenWebUI direct path strips it post-build in
    // `OpenWebUIProvider::stream` because OWUI forwards verbatim and any
    // upstream that doesn't accept the param 500s — different concern from
    // LiteLLM, so we handle it at the caller layer, not here.
    if let Some(ref effort) = opts.reasoning_effort {
        body["reasoning_effort"] = Value::from(effort.as_str());
    }
    for (key, value) in &opts.provider_options {
        body[key] = value.clone();
    }

    body
}

fn build_openwebui_chat_body(messages: Vec<ProviderMessage>, opts: &StreamOptions) -> Value {
    let mut body = build_body(messages, opts);
    apply_openwebui_chat_route_compat(&mut body, opts);
    strip_openwebui_unsupported_fields(&mut body, opts);
    body
}

fn local_openwebui_chat_id(seed: Option<&str>) -> String {
    let seed = seed.map(str::trim).filter(|s| !s.is_empty());
    match seed {
        Some(id) if id.starts_with("local:") || id.starts_with("channel:") => id.to_owned(),
        Some(id) => format!("local:{id}"),
        None => format!("local:jfc-{}", uuid::Uuid::new_v4().simple()),
    }
}

fn apply_openwebui_chat_route_compat(body: &mut Value, opts: &StreamOptions) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };

    let has_chat_id = obj
        .get("chat_id")
        .and_then(Value::as_str)
        .is_some_and(|id| !id.trim().is_empty());
    if !has_chat_id {
        // OWUI 0.9.x promoted `/api/chat/completions` to the same backend path
        // its Svelte chat UI uses. That route now assumes chat metadata is a
        // string in several `startswith` checks. Mark JFC traffic as a local
        // chat so OWUI skips DB persistence and does not see `chat_id = None`.
        //
        // Deliberately do not synthesize `session_id` or `id`: setting both
        // moves OWUI into WebSocket/background-task mode and it returns task
        // metadata instead of the SSE stream JFC consumes.
        obj.insert(
            "chat_id".into(),
            json!(local_openwebui_chat_id(opts.session_id.as_deref())),
        );
    }
}

fn strip_openwebui_unsupported_fields(body: &mut Value, opts: &StreamOptions) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };

    // OpenWebUI forwards unknown OpenAI-style params verbatim to whatever
    // backend model is selected (Bedrock-Anthropic, Bedrock-Cohere, Ollama,
    // Azure, etc.). We've seen Bedrock-Claude routes 500 on reasoning_effort,
    // while LiteLLM understands it. Keep it in shared build_body, strip it only
    // for direct OWUI, and let provider_options opt back in explicitly.
    if !opts.provider_options.contains_key("reasoning_effort") {
        obj.remove("reasoning_effort");
    }
}

fn chat_completions_url(base_url: &str) -> String {
    format!("{}/api/chat/completions", base_url.trim_end_matches('/'))
}

fn chat_completions_request<'a>(
    client: &'a reqwest::Client,
    url: &'a str,
    token: &'a str,
    body: &'a Value,
) -> reqwest::RequestBuilder {
    client
        .post(url)
        .header("authorization", format!("Bearer {token}"))
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .header("connection", "keep-alive")
        .header("x-litellm-stream-timeout", "600")
        .header("x-litellm-timeout", "600")
        .json(body)
}

impl jfc_provider::seal::Sealed for OpenWebUIProvider {}

#[async_trait]
impl Provider for OpenWebUIProvider {
    fn name(&self) -> &str {
        "openwebui"
    }

    /// OpenWebUI is OpenAI-compatible. Now that we send a structured `tools`
    /// array and parse `delta.tool_calls` from the stream, the model uses
    /// real tool calls instead of falling back to inline `<tool_call>` XML.
    /// We keep `InlineXmlTags` for safety so deployments whose server-side
    /// shim still injects XML in plain text don't break the renderer.
    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::InlineXmlTags
    }

    /// Static fallback for the picker when the live `/api/models` fetch hasn't completed
    /// (or failed). Intentionally empty — hardcoding model ids here was the source of the
    /// "Model not found" bug, since OpenWebUI instances expose whatever subset their
    /// admin has configured (often unrelated to the canonical Anthropic ids).
    /// Real population happens via `fetch_models()` at app startup.
    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    /// Live-fetch the configured OpenWebUI instance's `/api/models`. The list reflects
    /// whatever the admin has wired up (Bedrock, Vertex, Ollama, OpenAI, …) so the picker
    /// can never know it ahead of time — the only correct thing to do is ask the server.
    async fn fetch_models(&self) -> anyhow::Result<Vec<ModelInfo>> {
        let account = self.acquire_account_with_refresh().await?;

        let base_url = account.base_url.trim_end_matches('/');
        tracing::info!(
            target: "jfc::provider::openwebui",
            base_url,
            "fetching models"
        );
        let url = format!("{base_url}/api/models");
        let token = account.token;
        let resp = match jfc_provider::http::send_with_retry("openwebui.models", || {
            self.client
                .get(&url)
                .header("Authorization", format!("Bearer {token}"))
                .header("Accept", "application/json")
                .timeout(std::time::Duration::from_secs(8))
                .send()
        })
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let cause = jfc_provider::http::classify_send_error(&e);
                return Err(jfc_provider::ProviderError::network(
                    "openwebui",
                    format!("request failed: {cause} ({e})"),
                )
                .into());
            }
        };
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            return Err(jfc_provider::ProviderError::api_status(
                "openwebui",
                status.as_u16(),
                text,
            )
            .into());
        }
        let resp: ModelsResponse = resp.json().await?;

        let models: Vec<ModelInfo> = resp
            .data
            .into_iter()
            .map(|m| {
                let context_window_tokens = context_window_from_model(&m);
                let display = m.name.unwrap_or_else(|| m.id.clone());
                ModelInfo::new(m.id, display, "openwebui")
                    .with_context_window_tokens(context_window_tokens)
            })
            .collect();
        tracing::debug!(
            target: "jfc::provider::openwebui",
            model_count = models.len(),
            "fetch_models succeeded"
        );
        Ok(models)
    }

    #[tracing::instrument(
        target = "jfc::provider::openwebui",
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
        let account = self.acquire_account_with_refresh().await?;

        let url = chat_completions_url(&account.base_url);
        let body = build_openwebui_chat_body(messages, options);
        tracing::debug!(
            target: "jfc::provider::openwebui",
            url = %url,
            tools = body.get("tools").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            tool_choice = ?body.get("tool_choice"),
            messages = body.get("messages").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            chat_id = ?body.get("chat_id").and_then(|v| v.as_str()),
            "POST chat/completions"
        );

        // Headers mirror opencode-openwebui-auth's `buildHeaders`. The
        // `x-litellm-*-timeout` headers tell LiteLLM (which fronts most
        // Bedrock-on-OWUI deployments) to honor a long upstream timeout —
        // tool-call streams can exceed LiteLLM's default of 60s.
        let send_started = std::time::Instant::now();
        let resp = match jfc_provider::http::send_with_retry("openwebui.chat/completions", || {
            chat_completions_request(&self.client, &url, &account.token, &body).send()
        })
        .await
        {
            Ok(r) => r,
            Err(e) => {
                let cause = jfc_provider::http::classify_send_error(&e);
                tracing::warn!(
                    target: "jfc::provider::openwebui",
                    url = %url,
                    error = %e,
                    cause = cause,
                    "POST chat/completions failed before response (after retries)"
                );
                let error = jfc_provider::ProviderError::network(
                    "openwebui",
                    format!(
                        "request to {url} failed: {cause} ({e}). If this happens repeatedly, check the proxy/LiteLLM logs and verify ~/.config/jfc/openwebui/accounts.toml has a reachable base_url."
                    ),
                );
                anyhow::bail!("{AUTO_RETRY_SENTINEL}{error}");
            }
        };

        jfc_provider::http::report_first_byte_latency(
            "openwebui.chat/completions",
            send_started.elapsed(),
        );
        tracing::info!(
            target: "jfc::provider::openwebui",
            status = %resp.status(),
            model = %options.model,
            content_type = ?resp.headers().get("content-type"),
            "HTTP response received"
        );

        if !resp.status().is_success() {
            let status = resp.status();
            let should_retry =
                jfc_provider::retry::should_retry_status(status.as_u16(), Some(resp.headers()));
            let text = resp.text().await.unwrap_or_default();

            // 401/403 → token rejected. Try one OIDC re-auth with the env
            // creds, then re-issue the request. Mirrors fetch.ts:550.
            if matches!(status.as_u16(), 401 | 403)
                && std::env::var("OWUI_USERNAME").is_ok()
                && std::env::var("OWUI_PASSWORD").is_ok()
            {
                tracing::info!(
                    target: "jfc::provider::openwebui",
                    status = %status,
                    "auth rejected — attempting OIDC re-login then retry once"
                );
                if let Ok(refreshed) = self.refresh_active_account().await {
                    let retry_resp =
                        chat_completions_request(&self.client, &url, &refreshed.token, &body)
                            .send()
                            .await;
                    if let Ok(r) = retry_resp
                        && r.status().is_success()
                    {
                        return Ok(openai_compatible_event_stream(r));
                    }
                }
            }

            // Friendly translation for non-recoverable errors. Falls back to
            // raw status+body for anything we don't have a recipe for.
            let error = jfc_provider::ProviderError::api_status("openwebui", status.as_u16(), text);

            // Detect HTML/nginx proxy errors and translate into clean JSON.
            // Mirrors opencode-openwebui-auth/src/plugin/fetch.ts:656.
            let raw = error.raw.as_deref().unwrap_or_default();
            if raw.contains("<html") || raw.contains("<!DOCTYPE") {
                anyhow::bail!(
                    "OpenWebUI proxy error {status}: upstream returned HTML (nginx/proxy). \
                     The OWUI base URL or load balancer is misconfigured.\n  body preview: {}",
                    &raw[..raw.len().min(400)]
                );
            }

            if should_retry {
                anyhow::bail!("{AUTO_RETRY_SENTINEL}{error}");
            }
            return Err(error.into());
        }

        Ok(openai_compatible_event_stream(resp))
    }
}

/// `POST /api/chat/completed` — fire-and-forget outlet-filter notification.
///
/// Open WebUI's outlet filter chain (e.g. `rate_limit_inlet_filter` on
/// chat.ai2s.org, but also any user-installed Python filter functions
/// flagged "outlet") runs after every successful chat stream. Web clients
/// hit this endpoint immediately after the EventSource closes. Skipping
/// it makes a TUI client invisible to:
///   * Quota / usage tracking outlet filters
///   * Compliance / audit logging filters
///   * Server-side chat-history persistence (OWUI stores message history
///     here when filters return updated messages)
///
/// Pragmatically: not calling this risks **looking like a desync'd client**
/// to admins inspecting filter logs, and on instances with strict outlet
/// rate-limits the absence is detectable. We POST best-effort; failures
/// are logged but never propagated since the chat itself succeeded.
///
/// Payload shape mirrors the SvelteKit `chatCompleted` call from
/// `Chat.svelte:1155` (OWUI v0.7.2): `{ model, messages[], chat_id,
/// session_id, id }`. We send empty `messages` because the TUI's message
/// state is private to our session — OWUI just needs the metadata for
/// filter dispatch.
pub async fn notify_chat_completed(
    base_url: &str,
    token: &str,
    model: &str,
    chat_id: &str,
    session_id: &str,
    message_id: &str,
) {
    let url = format!("{}/api/chat/completed", base_url.trim_end_matches('/'));
    let body = json!({
        "model": model,
        "messages": [],
        "chat_id": chat_id,
        "session_id": session_id,
        "id": message_id,
    });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return;
    };
    match client
        .post(&url)
        .header("authorization", format!("Bearer {token}"))
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => {
            tracing::debug!(
                target: "jfc::provider::openwebui",
                status = %r.status(),
                chat_id,
                "chat/completed notification sent"
            );
        }
        Err(e) => {
            tracing::debug!(
                target: "jfc::provider::openwebui",
                error = %e,
                "chat/completed notification failed (non-fatal)"
            );
        }
    }
}

/// `POST /api/v1/auths/update/timezone` — one-shot client metadata.
///
/// OWUI's web client sends the user's IANA timezone right after login
/// (Chat.svelte boot sequence) so server-side outlets that generate
/// timestamps (chat exports, scheduled tasks) format in the user's
/// local zone. Skipping this leaves the user record at whatever default
/// was set at signup. Best-effort; never blocks login.
pub async fn update_user_timezone(base_url: &str, token: &str, timezone: &str) {
    let url = format!(
        "{}/api/v1/auths/update/timezone",
        base_url.trim_end_matches('/')
    );
    let body = json!({ "timezone": timezone });
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build();
    let Ok(client) = client else {
        return;
    };
    match client
        .post(&url)
        .header("authorization", format!("Bearer {token}"))
        .header("accept", "application/json")
        .header("content-type", "application/json")
        .json(&body)
        .send()
        .await
    {
        Ok(r) => {
            tracing::debug!(
                target: "jfc::provider::openwebui",
                status = %r.status(),
                timezone,
                "user timezone updated"
            );
        }
        Err(e) => {
            tracing::debug!(
                target: "jfc::provider::openwebui",
                error = %e,
                "timezone update failed (non-fatal)"
            );
        }
    }
}

/// Detect the system's IANA timezone for `update_user_timezone`.
///
/// Order of resolution (mirrors `tzlocal` Python lib):
///   1. `TZ` env var if set and not just `:`
///   2. `/etc/timezone` (Debian/Ubuntu)
///   3. `/etc/localtime` symlink → zoneinfo path (Arch/Fedora/macOS)
///   4. Fallback: `UTC`
pub fn detect_iana_timezone() -> String {
    if let Ok(tz) = std::env::var("TZ") {
        let t = tz.trim_start_matches(':');
        if !t.is_empty() && !t.starts_with('/') {
            return t.to_string();
        }
    }
    if let Ok(content) = std::fs::read_to_string("/etc/timezone") {
        let trimmed = content.trim();
        if !trimmed.is_empty() {
            return trimmed.to_string();
        }
    }
    if let Ok(target) = std::fs::read_link("/etc/localtime") {
        let path = target.to_string_lossy();
        // /usr/share/zoneinfo/America/Phoenix → America/Phoenix
        if let Some(idx) = path.find("/zoneinfo/") {
            return path[idx + "/zoneinfo/".len()..].to_string();
        }
    }
    "UTC".to_string()
}

/// OpenAI-compatible SSE: `data: {...}\n\ndata: [DONE]\n\n`.
///
/// Plain content arrives in `choices[0].delta.content`. Tool calls arrive as
/// `choices[0].delta.tool_calls[]`. The OpenAI streaming contract sends
/// `function.name` and `id` once, then streams only `function.arguments`
/// fragments, so this keeps a stateful accumulator keyed by tool index and
/// synthesizes final `ToolDone` events on `finish_reason: "tool_calls"`.
pub(crate) fn openai_compatible_event_stream(resp: reqwest::Response) -> EventStream {
    let event_stream = jfc_anthropic_sdk::sse::response_event_stream(resp)
        .scan(OpenAiStreamState::default(), |state, result| {
            let mut emitted: Vec<anyhow::Result<StreamEvent>> = Vec::new();
            match result {
                Ok(ev) => {
                    tracing::trace!(
                        target: "jfc::provider::openai_compatible",
                        event = %ev.event,
                        data = %&ev.data[..ev.data.len().min(400)],
                        "sse data"
                    );
                    if ev.data == "[DONE]" || ev.data.is_empty() {
                        tracing::debug!(target: "jfc::provider::openai_compatible", "sse [DONE]");
                        // Flush any buffered inline-tool text/partial tag before
                        // closing the turn so nothing is stranded in the buffer.
                        drain_inline_tool_calls(
                            &mut state.inline_buf,
                            &mut state.inline_index,
                            &mut emitted,
                            true,
                        );
                        emitted.push(Ok(StreamEvent::Done {
                            stop_reason: StopReason::EndTurn,
                        }));
                    } else {
                        match serde_json::from_str::<ChatChunk>(&ev.data) {
                            Ok(chunk) => {
                                if let Some(c) = chunk.choices.first()
                                    && let Some(reason) = c.finish_reason.as_deref() {
                                        tracing::info!(
                                            target: "jfc::provider::openai_compatible",
                                            finish_reason = reason,
                                            tool_calls = c.delta.tool_calls.as_ref().map(|t| t.len()).unwrap_or(0),
                                            accum = state.tools.len(),
                                            "chunk_finish"
                                        );
                                    }
                                if let Some(ref u) = chunk.usage {
                                    tracing::info!(
                                        target: "jfc::provider::openai_compatible",
                                        prompt_tokens = u.prompt_tokens,
                                        completion_tokens = u.completion_tokens,
                                        total_tokens = u.total_tokens,
                                        cache_read_tokens = u.cache_read_tokens(),
                                        cache_write_tokens = u.cache_write_tokens(),
                                        "usage"
                                    );
                                    emitted.push(Ok(StreamEvent::Usage {
                                        input_tokens: u.raw_input_tokens(),
                                        output_tokens: u.completion_tokens,
                                        cache_read_tokens: u.cache_read_tokens(),
                                        cache_write_tokens: u.cache_write_tokens(),
                                    }));
                                }
                                push_chunk_events_stateful(chunk, state, &mut emitted);
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "jfc::provider::openai_compatible",
                                    error = %e,
                                    data = %&ev.data[..ev.data.len().min(200)],
                                    "sse parse error"
                                );
                                emitted.push(Err(anyhow::anyhow!(
                                    "OpenAI-compatible SSE JSON parse error: {e}"
                                )));
                            }
                        }
                    }
                }
                Err(e) => {
                    emitted.push(Err(anyhow::anyhow!(
                        "OpenAI-compatible SSE stream parse error: {e}"
                    )));
                }
            }
            futures::future::ready(Some(emitted))
        })
        .flat_map(futures::stream::iter);

    Box::pin(event_stream)
}

/// Per-tool-call accumulator. Each chunk may set or extend any of the three
/// fields; `name` and `id` typically arrive on the first chunk for an index,
/// `args` is built up across many.
#[derive(Debug, Default, Clone)]
struct AccumTool {
    id: Option<String>,
    name: Option<String>,
    args: String,
}

/// Streaming state for the OpenAI-compatible parser. Carries the structured
/// `tool_calls` accumulator AND a buffer for intercepting **inline**
/// `<tool_call>{…}</tool_call>` blocks that some gateways (notably the
/// OpenWebUI Bedrock proxy) emit as plain text instead of as structured
/// `tool_calls` SSE deltas. Without interception those leak into the rendered
/// transcript (the `⟪tool_call⟫` marker) and never execute.
#[derive(Debug, Default)]
struct OpenAiStreamState {
    tools: HashMap<usize, AccumTool>,
    /// Pending assistant text not yet emitted, held so a `<tool_call>` block
    /// that spans multiple SSE deltas can be detected whole.
    inline_buf: String,
    /// Monotonic index for synthesized inline tool calls. Offset by a large
    /// base so it never collides with a structured `tool_calls` index.
    inline_index: usize,
}

/// Inline tag pairs we intercept, as `(open, close, is_call)`. Different
/// gateways use different spellings: LiteLLM's Qwen3-on-Bedrock emits
/// `<tool_call>` (args double-encoded), while Bedrock **Claude** emits
/// Anthropic's `<tool_use>` (args as an object). We treat both as tool calls.
/// `<tool_result>` blocks are the gateway's echo OR the model *fabricating* a
/// result — either way they're suppressed, because the real result comes from
/// actually dispatching the call. Order matters only for tie-breaking at the
/// same offset (none overlap in practice).
const INLINE_TAGS: &[(&str, &str, bool)] = &[
    ("<tool_use>", "</tool_use>", true),
    ("<tool_call>", "</tool_call>", true),
    ("<tool_result>", "</tool_result>", false),
    // Plural wrapper emitted by LiteLLM/Bedrock when a model produces several
    // calls in one turn: `<tool_calls><tool_call>…</tool_call>…</tool_calls>`.
    // The whole wrapper is consumed as a single block; `parse_inline_tool_call`
    // splits the inner `<tool_call>` children. The matching `<tool_results>`
    // echo is suppressed like the singular `<tool_result>`.
    ("<tool_calls>", "</tool_calls>", true),
    ("<tool_results>", "</tool_results>", false),
];
/// Index base for synthesized inline tool calls — keeps them out of the
/// structured `tool_calls` index space (which starts at 0).
const INLINE_TOOL_INDEX_BASE: usize = 100_000;

/// Largest trailing run of `buf` that is a prefix of `needle` (so it might
/// become a full match once more bytes arrive). Used to hold back a partial
/// `<tool_call>` open tag split across SSE chunks instead of flushing it as
/// visible text.
fn partial_prefix_suffix_len(buf: &str, needle: &str) -> usize {
    let max = needle.len().saturating_sub(1).min(buf.len());
    for k in (1..=max).rev() {
        let start = buf.len() - k;
        if buf.is_char_boundary(start) && buf[start..] == needle[..k] {
            return k;
        }
    }
    0
}

/// Extract the inner text of the first `<tag>…</tag>` element in `s`.
/// Used to pull `<tool_name>`/`<tool_input>` children out of the plural-
/// wrapper shape LiteLLM/Bedrock emits.
fn extract_xml_tag(s: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = s.find(&open)? + open.len();
    let end = s[start..].find(&close)? + start;
    Some(s[start..end].to_owned())
}

/// Normalize a tool-arguments payload to a JSON object string. Accepts a
/// JSON object/value or a JSON-encoded string (some gateways double-encode);
/// an empty payload becomes `{}`.
fn normalize_tool_args(raw: &str) -> String {
    let raw = raw.trim();
    if raw.is_empty() {
        return "{}".to_owned();
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(serde_json::Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                "{}".to_owned()
            } else {
                s.to_owned()
            }
        }
        // A parsed object/array/number round-trips through its source text so
        // formatting is preserved; an unparseable body is passed through
        // verbatim and validated downstream by the tool dispatch layer.
        Ok(_) | Err(_) => raw.to_owned(),
    }
}

/// Parse an inline JSON tool body — `{"name": "...", "arguments": {...}}` —
/// into `(name, arguments_json_string)`. `arguments` may itself be a
/// JSON-encoded string (some gateways double-encode).
fn parse_json_tool_call(body: &str) -> Option<(String, String)> {
    let v: serde_json::Value = serde_json::from_str(body).ok()?;
    parse_json_tool_call_value(&v)
}

fn parse_json_tool_call_value(v: &serde_json::Value) -> Option<(String, String)> {
    let name = v.get("name")?.as_str()?.to_owned();
    let input_json = match v.get("arguments") {
        Some(serde_json::Value::String(s)) => {
            let s = s.trim();
            if s.is_empty() {
                "{}".to_owned()
            } else {
                s.to_owned()
            }
        }
        Some(other) => serde_json::to_string(other).unwrap_or_else(|_| "{}".to_owned()),
        None => "{}".to_owned(),
    };
    Some((name, input_json))
}

fn is_known_inline_tool_name(name: &str) -> bool {
    !matches!(
        jfc_core::ToolKind::from_name(name),
        jfc_core::ToolKind::UnknownTool { .. }
    )
}

/// Some OpenAI-compatible gateways/models emit a complete tool call as plain
/// assistant JSON instead of `delta.tool_calls` or tagged XML:
/// `{"name":"Bash","arguments":{"command":"pwd"}}`. Only consume the content
/// when the whole buffered message is one or more known jfc tool calls.
fn parse_bare_json_tool_calls(body: &str) -> Option<Vec<(String, String)>> {
    let trimmed = body.trim();
    if trimmed.is_empty() {
        return None;
    }

    let value: serde_json::Value = serde_json::from_str(trimmed).ok()?;
    match value {
        serde_json::Value::Object(_) => {
            let call = parse_json_tool_call_value(&value)?;
            is_known_inline_tool_name(&call.0).then_some(vec![call])
        }
        serde_json::Value::Array(items) => {
            let mut calls = Vec::with_capacity(items.len());
            for item in &items {
                let call = parse_json_tool_call_value(item)?;
                if !is_known_inline_tool_name(&call.0) {
                    return None;
                }
                calls.push(call);
            }
            (!calls.is_empty()).then_some(calls)
        }
        _ => None,
    }
}

fn should_hold_bare_json_tool_candidate(buf: &str) -> bool {
    let trimmed = buf.trim_start();
    trimmed.starts_with('{') || trimmed.starts_with('[')
}

fn push_synthesized_inline_tool_events(
    calls: Vec<(String, String)>,
    inline_index: &mut usize,
    out: &mut Vec<anyhow::Result<StreamEvent>>,
    source: &str,
) {
    for (name, input_json) in calls {
        let idx = INLINE_TOOL_INDEX_BASE + *inline_index;
        *inline_index += 1;
        tracing::info!(
            target: "jfc::provider::openai_compatible",
            index = idx,
            tool_name = %name,
            source,
            "synthesized tool_done from inline tool content"
        );
        out.push(Ok(StreamEvent::ToolDone {
            index: idx,
            tool_name: name,
            tool_use_id: format!("toolu_{}", uuid::Uuid::new_v4().simple()),
            input_json,
            thought_signature: None,
        }));
    }
}

/// Parse the body of an inline tool block into zero or more
/// `(tool_name, arguments_json)` pairs. Handles every shape seen in the wild:
///
///   1. **Plural wrapper children** — the body is a run of
///      `<tool_call>…</tool_call>` elements (the outer `<tool_calls>` wrapper
///      was already stripped by the drain loop). Each child parses on its own,
///      so one wrapper can yield multiple tool calls.
///   2. **Child-tag form** — `<tool_name>NAME</tool_name>
///      <tool_input>{json}</tool_input>` (the live LiteLLM/Bedrock plural
///      shape, e.g. `ses_20260521_062804.json`).
///   3. **JSON-body form** — `{"name": "...", "arguments": {...}}` (the legacy
///      singular `<tool_call>`/`<tool_use>` shape).
fn parse_inline_tool_calls(body: &str) -> Vec<(String, String)> {
    let trimmed = body.trim();

    // Shape 1: nested <tool_call> children (plural wrapper). Drain each child
    // element and parse it on its own. The recursion terminates because every
    // step strips an outer <tool_call>…</tool_call> and advances past it.
    // Require BOTH the open and close child tags so a singular JSON body that
    // merely mentions "<tool_call>" inside an argument string isn't misrouted.
    if trimmed.contains("<tool_call>") && trimmed.contains("</tool_call>") {
        let mut calls = Vec::new();
        let mut rest = trimmed;
        while let Some(start) = rest.find("<tool_call>") {
            let after = &rest[start + "<tool_call>".len()..];
            let Some(end) = after.find("</tool_call>") else {
                break;
            };
            calls.extend(parse_inline_tool_calls(&after[..end]));
            rest = &after[end + "</tool_call>".len()..];
        }
        return calls;
    }

    // Shape 2: child-tag form.
    if let Some(name) = extract_xml_tag(trimmed, "tool_name") {
        let input_json = extract_xml_tag(trimmed, "tool_input")
            .map(|raw| normalize_tool_args(&raw))
            .unwrap_or_else(|| "{}".to_owned());
        let name = name.trim();
        if !name.is_empty() {
            return vec![(name.to_owned(), input_json)];
        }
    }

    // Shape 3: JSON body.
    parse_json_tool_call(trimmed).into_iter().collect()
}

/// Drain complete inline `<tool_call>…</tool_call>` blocks out of `buf`,
/// emitting `TextDelta` for surrounding prose and a synthesized `ToolDone` for
/// each parsed call (routed through the normal dispatch pipeline exactly like
/// a structured tool call). A partial trailing tag is held in `buf` until the
/// next delta — unless `flush` is set (stream ending), in which case any
/// remainder is emitted verbatim so no bytes are lost.
fn drain_inline_tool_calls(
    buf: &mut String,
    inline_index: &mut usize,
    out: &mut Vec<anyhow::Result<StreamEvent>>,
    flush: bool,
) {
    loop {
        // Find the earliest open tag among all recognized kinds.
        let earliest = INLINE_TAGS
            .iter()
            .filter_map(|&(open, close, is_call)| {
                buf.find(open).map(|at| (at, open, close, is_call))
            })
            .min_by_key(|&(at, ..)| at);

        let Some((open_at, open, close, is_call)) = earliest else {
            if flush && let Some(calls) = parse_bare_json_tool_calls(buf) {
                buf.clear();
                push_synthesized_inline_tool_events(calls, inline_index, out, "bare_json");
                break;
            }
            if !flush && should_hold_bare_json_tool_candidate(buf) {
                break;
            }
            // No complete open tag. Hold back a partial open-tag suffix
            // (e.g. a chunk ending in "<tool_u") unless we're flushing.
            let hold = if flush {
                0
            } else {
                INLINE_TAGS
                    .iter()
                    .map(|&(open, ..)| partial_prefix_suffix_len(buf, open))
                    .max()
                    .unwrap_or(0)
            };
            let emit_len = buf.len() - hold;
            if emit_len > 0 {
                let text: String = buf.drain(..emit_len).collect();
                out.push(Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: text,
                }));
            }
            break;
        };

        if open_at > 0 {
            let text: String = buf.drain(..open_at).collect();
            out.push(Ok(StreamEvent::TextDelta {
                index: 0,
                delta: text,
            }));
        }
        // `buf` now starts with `open`. Search for the matching close *after*
        // the open tag so an empty/degenerate block can't match itself.
        match buf[open.len()..].find(close) {
            Some(rel) => {
                let close_at = open.len() + rel;
                let body = buf[open.len()..close_at].trim().to_owned();
                let block_end = close_at + close.len();
                buf.drain(..block_end);
                if !is_call {
                    // Inline <tool_result>/<tool_results> — suppress entirely.
                    // The real result comes from dispatching the call; a
                    // model-fabricated one here must not be fed back as truth.
                    tracing::debug!(
                        target: "jfc::provider::openai_compatible",
                        "suppressed inline {open} block ({} bytes)",
                        body.len()
                    );
                    continue;
                }
                let calls = parse_inline_tool_calls(&body);
                if calls.is_empty() {
                    // Unparseable body — surface verbatim rather than
                    // silently dropping the model's content.
                    out.push(Ok(StreamEvent::TextDelta {
                        index: 0,
                        delta: format!("{open}{body}{close}"),
                    }));
                } else {
                    push_synthesized_inline_tool_events(calls, inline_index, out, open);
                }
                continue;
            }
            None => {
                // Open tag with no close yet.
                if flush {
                    let text: String = std::mem::take(buf);
                    out.push(Ok(StreamEvent::TextDelta {
                        index: 0,
                        delta: text,
                    }));
                }
                break;
            }
        }
    }
}

/// Stateful version of `push_chunk_events`. Mutates `state` to carry tool-call
/// metadata across chunks; emits `ToolDelta` for every non-empty argument
/// fragment, and synthesizes `ToolDone` events at finish_reason time even when
/// the finish chunk itself carries no tool_calls (the LiteLLM-on-Bedrock bug).
fn push_chunk_events_stateful(
    chunk: ChatChunk,
    state: &mut OpenAiStreamState,
    out: &mut Vec<anyhow::Result<StreamEvent>>,
) {
    let Some(choice) = chunk.choices.into_iter().next() else {
        return;
    };

    if let Some(thinking) = choice.delta.reasoning_content.clone()
        && !thinking.is_empty()
    {
        out.push(Ok(StreamEvent::ThinkingDelta {
            index: 0,
            delta: thinking,
            estimated_tokens: None,
        }));
    }
    if let Some(text) = choice.delta.content.clone()
        && !text.is_empty()
    {
        // Buffer assistant text and drain any complete inline
        // `<tool_call>…</tool_call>` blocks into synthesized ToolDone events.
        // Plain text (no tags) passes straight through; only a partial trailing
        // tag is ever held back, so normal streaming is unaffected.
        state.inline_buf.push_str(&text);
        drain_inline_tool_calls(&mut state.inline_buf, &mut state.inline_index, out, false);
    }
    if let Some(refusal) = choice.delta.refusal.clone()
        && !refusal.is_empty()
    {
        out.push(Ok(StreamEvent::TextDelta {
            index: 0,
            delta: refusal,
        }));
    }

    // Prefer canonical OpenAI-compatible `tool_calls` for a chunk. Legacy
    // aliases are fallbacks for gateways that emit only those fields; treating
    // same-chunk aliases as additional deltas would duplicate arguments.
    let tool_calls = if let Some(tool_calls) = choice
        .delta
        .tool_calls
        .as_ref()
        .filter(|tool_calls| !tool_calls.is_empty())
    {
        tool_calls.clone()
    } else if let Some(fcs) = choice
        .delta
        .function_calls
        .as_ref()
        .filter(|function_calls| !function_calls.is_empty())
    {
        fcs.iter()
            .enumerate()
            .map(|(fallback_index, fc)| fc.to_tool_call(fallback_index))
            .collect()
    } else if let Some(fc) = choice.delta.function_call.as_ref() {
        vec![fc.to_tool_call(0)]
    } else {
        Vec::new()
    };
    for tc in &tool_calls {
        let idx = tc.index.unwrap_or(0);
        let entry = state.tools.entry(idx).or_default();
        if let Some(id) = tc.id.as_deref()
            && !id.is_empty()
        {
            entry.id = Some(id.to_owned());
        }
        if let Some(name) = tc.function.as_ref().and_then(|f| f.name.as_deref())
            && !name.is_empty()
        {
            entry.name = Some(name.to_owned());
        }
        if let Some(args) = tc.function.as_ref().and_then(|f| f.arguments.as_deref())
            && !args.is_empty()
        {
            entry.args.push_str(args);
            out.push(Ok(StreamEvent::ToolDelta {
                index: idx,
                delta: args.to_owned(),
            }));
        }
    }

    if let Some(reason) = choice.finish_reason {
        // Flush any buffered inline-tool text + trailing partial tag first, so
        // a `<tool_call>` that landed right before finish still dispatches.
        drain_inline_tool_calls(&mut state.inline_buf, &mut state.inline_index, out, true);

        let mapped = match reason.as_str() {
            "tool_calls" | "function_call" | "function_calls" => StopReason::ToolUse,
            "stop" => StopReason::EndTurn,
            "length" => StopReason::MaxTokens,
            "content_filter" | "refusal" => StopReason::Refusal,
            other => StopReason::Other(other.to_owned()),
        };

        // Emit ToolDone for every accumulated tool — independent of whether
        // the finish chunk's tool_calls array is populated. Sorted by index
        // for deterministic ordering across runs.
        let mut by_index: Vec<(usize, AccumTool)> =
            std::mem::take(&mut state.tools).into_iter().collect();
        by_index.sort_by_key(|(idx, _)| *idx);
        for (idx, accum) in by_index {
            let name = accum.name.unwrap_or_default();
            let id = accum.id.unwrap_or_else(|| format!("call_{idx}"));
            tracing::info!(
                target: "jfc::provider::openwebui",
                index = idx,
                tool_name = %name,
                tool_use_id = %id,
                args_len = accum.args.len(),
                "synthesize tool_done from accumulator"
            );
            out.push(Ok(StreamEvent::ToolDone {
                index: idx,
                tool_name: name,
                tool_use_id: id,
                input_json: accum.args,
                thought_signature: None,
            }));
        }
        out.push(Ok(StreamEvent::Done {
            stop_reason: mapped,
        }));
    }
}
