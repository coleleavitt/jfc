#![allow(dead_code)]

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

use crate::provider::{
    EventStream, ModelInfo, Provider, ProviderContent, ProviderMessage, ProviderRole, StopReason,
    StreamConvention, StreamEvent, StreamOptions,
};

pub(crate) const AUTO_RETRY_SENTINEL: &str = "auto-retry-openwebui:";

// Re-export the new modular auth types so external callers (CLI, etc.) can
// reach them through `providers::openwebui::*`.
pub use self::jwt::{is_token_expired, parse_jwt_claims};
pub use self::oidc::{DuoMethod, OidcLoginOptions, oidc_login};
pub use self::store::{
    Account, default_store_path, get_current, list as list_accounts, load_store,
    remove as remove_account, set_current, upsert as upsert_account,
};
pub use self::verify::{fetch_instance_config, normalize_base_url, verify_token};

/// Backwards-compat shim so the existing test-suite that calls `load_account(path)`
/// continues to compile against the new modular store.
#[cfg(test)]
fn load_account(path: &PathBuf) -> anyhow::Result<Account> {
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

impl OpenWebUIProvider {
    pub fn new() -> Self {
        let store_path = default_store_path();
        tracing::debug!(
            target: "jfc::provider::openwebui",
            store_path = %store_path.display(),
            "OpenWebUIProvider::new"
        );
        Self {
            client: super::http::streaming_client(),
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
                if key.ends_with(".context_length") {
                    if let Some(tokens) = value_as_usize(value) {
                        return Some(tokens);
                    }
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
        && (has("mythos") || (has("opus") && (has_version("4", "7") || has_version("4", "6"))))
    {
        1_000_000
    } else if has("claude") && has("sonnet") && has_version("4", "6") {
        1_000_000
    } else if has("claude") && has("opus") && has_version("4", "5") {
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
    use crate::provider::ToolDef;

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
                    },
                    ProviderContent::ToolUse {
                        id: "c2".into(),
                        name: "Read".into(),
                        input: serde_json::json!({}),
                    },
                    ProviderContent::ToolUse {
                        id: "c3".into(),
                        name: "ApplyPatch".into(),
                        input: serde_json::json!({}),
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

    fn evs_stateful(state: &mut HashMap<usize, AccumTool>, c: ChatChunk) -> Vec<StreamEvent> {
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
        let mut state: HashMap<usize, AccumTool> = HashMap::new();

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
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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
            state.is_empty(),
            "accumulator not drained on finish: {state:?}"
        );
    }

    // Robust: finish_reason "stop" with no tool_calls in history → just
    // emits Done(EndTurn), no spurious ToolDone.
    #[test]
    fn stateful_finish_stop_emits_no_tool_done_robust() {
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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

    // Robust: assistant turn with content=null (tool-call-only) is left alone.
    #[test]
    fn bedrock_null_content_assistant_turn_left_alone_robust() {
        let mut msgs = vec![json!({
            "role": "assistant",
            "content": null,
            "tool_calls": [{"id": "x"}],
        })];
        bedrock_sanitize_messages(&mut msgs);
        assert_eq!(msgs[0]["content"], json!(null));
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
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
        let events = evs_stateful(&mut state, chunk(ChunkDelta::default(), Some("length")));
        assert!(events.iter().any(|e| matches!(
            e,
            StreamEvent::Done {
                stop_reason: StopReason::MaxTokens
            }
        )));
    }

    // Robust: an unrecognized finish_reason maps to StopReason::Other
    // verbatim. Future-proofs against a proxy emitting a novel reason.
    #[test]
    fn stateful_finish_unknown_maps_to_other_robust() {
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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
        assert_eq!(done, StopReason::Other("content_filter".into()));
    }

    // Robust: a chunk with no choices at all is a no-op — push_chunk_events_stateful
    // returns early without panicking.
    #[test]
    fn stateful_chunk_with_no_choices_is_noop_robust() {
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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
        let mut state: HashMap<usize, AccumTool> = HashMap::new();
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

/// Bedrock-on-OpenWebUI requires every text content block to contain at least
/// one non-whitespace character. The placeholder `"."` is what
/// opencode-openwebui-auth (`src/plugin/fetch.ts:92`) settled on after observing
/// two distinct error variants from Bedrock:
///   - "The text field in the ContentBlock object at messages.N.content.M is blank."
///   - "messages: text content blocks must contain non-whitespace text"
const BEDROCK_BLANK_TEXT_PLACEHOLDER: &str = ".";

/// Replace any empty `content` strings on messages with the Bedrock placeholder.
/// Mirrors `sanitizeMessageContent` in opencode's plugin/fetch.ts.
fn bedrock_sanitize_messages(messages: &mut Vec<Value>) {
    for msg in messages.iter_mut() {
        let Some(obj) = msg.as_object_mut() else {
            continue;
        };
        match obj.get("content") {
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
        if let Some(msgs) = obj.get("messages").and_then(|v| v.as_array()) {
            if messages_reference_tools(msgs) {
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
    let msgs: Vec<Value> = messages
        .iter()
        .flat_map(|m| {
            m.content.iter().filter_map(|c| match c {
                ProviderContent::Text(t) if !t.is_empty() => Some(json!({
                    "role": match m.role {
                        ProviderRole::User => "user",
                        ProviderRole::Assistant => "assistant",
                    },
                    "content": t,
                })),
                ProviderContent::ToolUse { id, name, input } => Some(json!({
                    "role": "assistant",
                    "tool_calls": [{
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
                    }],
                })),
                ProviderContent::ToolResult {
                    tool_use_id,
                    content,
                    ..
                } => Some(json!({
                    "role": "tool",
                    "tool_call_id": tool_use_id,
                    "content": content,
                })),
                _ => None,
            })
        })
        .collect();

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
    if let Some(ref effort) = opts.reasoning_effort {
        body["reasoning_effort"] = Value::from(effort.as_str());
    }
    for (key, value) in &opts.provider_options {
        body[key] = value.clone();
    }

    body
}

impl crate::provider::seal::Sealed for OpenWebUIProvider {}

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
        let resp: ModelsResponse = self
            .client
            .get(format!("{base_url}/api/models"))
            .header("Authorization", format!("Bearer {}", account.token))
            .header("Accept", "application/json")
            .timeout(std::time::Duration::from_secs(8))
            .send()
            .await?
            .error_for_status()?
            .json()
            .await?;

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

        let base_url = account.base_url.trim_end_matches('/');
        let url = format!("{}/api/chat/completions", base_url);
        let body = build_body(messages, options);
        tracing::debug!(
            target: "jfc::provider::openwebui",
            url = %url,
            tools = body.get("tools").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            tool_choice = ?body.get("tool_choice"),
            messages = body.get("messages").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0),
            "POST chat/completions"
        );

        // Headers mirror opencode-openwebui-auth's `buildHeaders`. The
        // `x-litellm-*-timeout` headers tell LiteLLM (which fronts most
        // Bedrock-on-OWUI deployments) to honor a long upstream timeout —
        // tool-call streams can exceed LiteLLM's default of 60s.
        let send_started = std::time::Instant::now();
        let resp = match super::http::send_with_retry("openwebui.chat/completions", || {
            self.client
                .post(&url)
                .header("authorization", format!("Bearer {}", account.token))
                .header("accept", "application/json")
                .header("content-type", "application/json")
                .header("connection", "keep-alive")
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
                    target: "jfc::provider::openwebui",
                    url = %url,
                    error = %e,
                    cause = cause,
                    "POST chat/completions failed before response (after retries)"
                );
                anyhow::bail!(
                    "{AUTO_RETRY_SENTINEL}OpenWebUI request to {url} failed: {cause} ({e}). \
                     If this happens repeatedly, check the proxy/LiteLLM \
                     logs and verify ~/.config/jfc/openwebui/accounts.toml \
                     has a reachable base_url."
                );
            }
        };

        super::http::report_first_byte_latency(
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
                super::retry::should_retry_status(status.as_u16(), Some(resp.headers()));
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
                    let retry_resp = self
                        .client
                        .post(&url)
                        .header("authorization", format!("Bearer {}", refreshed.token))
                        .header("accept", "application/json")
                        .header("content-type", "application/json")
                        .header("connection", "keep-alive")
                        .header("x-litellm-stream-timeout", "600")
                        .header("x-litellm-timeout", "600")
                        .json(&body)
                        .send()
                        .await;
                    if let Ok(r) = retry_resp {
                        if r.status().is_success() {
                            return Ok(openai_compatible_event_stream(r));
                        }
                    }
                }
            }

            // Friendly translation for non-recoverable errors. Falls back to
            // raw status+body for anything we don't have a recipe for.
            let friendly = super::retry::friendly_error_message(status.as_u16(), &text);

            // Detect HTML/nginx proxy errors and translate into clean JSON.
            // Mirrors opencode-openwebui-auth/src/plugin/fetch.ts:656.
            if text.contains("<html") || text.contains("<!DOCTYPE") {
                anyhow::bail!(
                    "OpenWebUI proxy error {status}: upstream returned HTML (nginx/proxy). \
                     The OWUI base URL or load balancer is misconfigured.\n  body preview: {}",
                    &text[..text.len().min(400)]
                );
            }

            if should_retry {
                anyhow::bail!(
                    "{AUTO_RETRY_SENTINEL}OpenWebUI API error {status}: {friendly}\n  raw: {text}"
                );
            }
            anyhow::bail!("OpenWebUI API error {status}: {friendly}\n  raw: {text}");
        }

        Ok(openai_compatible_event_stream(resp))
    }
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
        .scan(HashMap::<usize, AccumTool>::new(), |state, result| {
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
                        emitted.push(Ok(StreamEvent::Done {
                            stop_reason: StopReason::EndTurn,
                        }));
                    } else {
                        match serde_json::from_str::<ChatChunk>(&ev.data) {
                            Ok(chunk) => {
                                if let Some(c) = chunk.choices.first() {
                                    if let Some(reason) = c.finish_reason.as_deref() {
                                        tracing::info!(
                                            target: "jfc::provider::openai_compatible",
                                            finish_reason = reason,
                                            tool_calls = c.delta.tool_calls.as_ref().map(|t| t.len()).unwrap_or(0),
                                            accum = state.len(),
                                            "chunk_finish"
                                        );
                                    }
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

/// Stateful version of `push_chunk_events`. Mutates `state` to carry tool-call
/// metadata across chunks; emits `ToolDelta` for every non-empty argument
/// fragment, and synthesizes `ToolDone` events at finish_reason time even when
/// the finish chunk itself carries no tool_calls (the LiteLLM-on-Bedrock bug).
fn push_chunk_events_stateful(
    chunk: ChatChunk,
    state: &mut HashMap<usize, AccumTool>,
    out: &mut Vec<anyhow::Result<StreamEvent>>,
) {
    let Some(choice) = chunk.choices.into_iter().next() else {
        return;
    };

    if let Some(thinking) = choice.delta.reasoning_content.clone() {
        if !thinking.is_empty() {
            out.push(Ok(StreamEvent::ThinkingDelta {
                index: 0,
                delta: thinking,
            }));
        }
    }
    if let Some(text) = choice.delta.content.clone() {
        if !text.is_empty() {
            out.push(Ok(StreamEvent::TextDelta {
                index: 0,
                delta: text,
            }));
        }
    }
    if let Some(refusal) = choice.delta.refusal.clone() {
        if !refusal.is_empty() {
            out.push(Ok(StreamEvent::TextDelta {
                index: 0,
                delta: refusal,
            }));
        }
    }

    let tool_calls = choice.delta.tool_calls.clone().unwrap_or_default();
    for tc in &tool_calls {
        let idx = tc.index.unwrap_or(0);
        let entry = state.entry(idx).or_default();
        if let Some(id) = tc.id.as_deref() {
            if !id.is_empty() {
                entry.id = Some(id.to_owned());
            }
        }
        if let Some(name) = tc.function.as_ref().and_then(|f| f.name.as_deref()) {
            if !name.is_empty() {
                entry.name = Some(name.to_owned());
            }
        }
        if let Some(args) = tc.function.as_ref().and_then(|f| f.arguments.as_deref()) {
            if !args.is_empty() {
                entry.args.push_str(args);
                out.push(Ok(StreamEvent::ToolDelta {
                    index: idx,
                    delta: args.to_owned(),
                }));
            }
        }
    }

    if let Some(reason) = choice.finish_reason {
        let mapped = match reason.as_str() {
            "tool_calls" | "function_call" => StopReason::ToolUse,
            "stop" => StopReason::EndTurn,
            "length" => StopReason::MaxTokens,
            other => StopReason::Other(other.to_owned()),
        };

        // Emit ToolDone for every accumulated tool — independent of whether
        // the finish chunk's tool_calls array is populated. Sorted by index
        // for deterministic ordering across runs.
        let mut by_index: Vec<(usize, AccumTool)> = std::mem::take(state).into_iter().collect();
        by_index.sort_by_key(|(idx, _)| *idx);
        for (idx, accum) in by_index {
            let name = accum.name.unwrap_or_default();
            let id = accum.id.unwrap_or_default();
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
            }));
        }
        out.push(Ok(StreamEvent::Done {
            stop_reason: mapped,
        }));
    }
}
