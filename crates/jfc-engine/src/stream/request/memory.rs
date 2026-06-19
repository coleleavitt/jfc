use std::sync::Arc;

use jfc_provider::{ModelId, Provider};

pub(super) fn append_memory_recall_context(
    system_prompt: &mut String,
    recall_block: Option<&String>,
    memories: &[jfc_memory::MemoryEntry],
    recall_enabled: bool,
    recall_was_fresh: bool,
) -> usize {
    if let Some(block) = recall_block {
        tracing::debug!(
            target: "jfc::stream",
            recall_block_len = block.len(),
            "using memory recall block (skipping full memory dump)"
        );
        system_prompt.push_str(block);
        return if recall_was_fresh { block.len() } else { 0 };
    }

    if !memories.is_empty() {
        tracing::debug!(
            target: "jfc::stream",
            memory_count = memories.len(),
            recall_enabled,
            "memory recall produced no block; skipping unbounded full-memory prompt dump"
        );
    }

    0
}

/// Pick a fast/cheap model for the pre-flight memory/plan recall. Recall is a
/// trivial select→extract classification, so running it on the main model
/// (e.g. opus) is needlessly slow and expensive — a haiku-class model does it
/// in a fraction of the time, which is the bigger half of cold-recall latency.
/// Uses the provider's own haiku model when it advertises one; otherwise falls
/// back to the main model (recall then behaves exactly as before, and still
/// degrades to a full memory dump if it errors). Provider-aware, so it picks
/// the right haiku id for Anthropic/Bedrock/Vertex and no-ops for providers
/// (e.g. OpenWebUI) that don't offer one.
pub(super) fn fast_recall_model(provider: &Arc<dyn Provider>, main: &ModelId) -> ModelId {
    provider
        .available_models()
        .into_iter()
        .find(|m| m.id.as_str().contains("haiku"))
        .map(|m| m.id)
        .unwrap_or_else(|| main.clone())
}

pub(super) async fn sdk_memory_store_prompt_section() -> Option<String> {
    let ids = configured_memory_store_ids();
    if ids.is_empty() {
        return None;
    }
    let Some(client) = crate::sdk_bridge::build_client() else {
        return Some(crate::system_reminder::format(
            "JFC_MEMORY_STORE_IDS is configured, but no Anthropic SDK API key profile is available. \
             Remote SDK memory stores were not loaded for this turn.",
        ));
    };
    let service = jfc_anthropic_sdk::memory_stores::MemoryStoreService::new(client);
    let limit = std::env::var("JFC_MEMORY_STORE_LIMIT")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(20)
        .clamp(1, 100);
    let timeout = std::env::var("JFC_MEMORY_STORE_TIMEOUT_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(std::time::Duration::from_secs)
        .unwrap_or_else(|| std::time::Duration::from_secs(8));

    let mut out = String::from("\n\n## SDK memory stores\n\n");
    let mut loaded_any = false;
    for store_id in ids {
        let params = jfc_anthropic_sdk::pagination::ListParams {
            limit: Some(limit),
            ..Default::default()
        };
        match tokio::time::timeout(timeout, service.list_memories(&store_id, &params)).await {
            Ok(Ok(page)) => {
                loaded_any = true;
                out.push_str(&format!("### {store_id}\n\n"));
                if page.data.is_empty() {
                    out.push_str("(no memories returned)\n\n");
                } else {
                    for memory in page.data {
                        let body = render_sdk_memory_content(&memory);
                        out.push_str(&format!("- `{}`: {}\n", memory.id, body));
                    }
                    out.push('\n');
                }
            }
            Ok(Err(err)) => {
                out.push_str(&format!(
                    "### {store_id}\n\n(remote memory load failed: {err})\n\n"
                ));
            }
            Err(_) => {
                out.push_str(&format!(
                    "### {store_id}\n\n(remote memory load timed out after {}s)\n\n",
                    timeout.as_secs()
                ));
            }
        }
    }

    if loaded_any || !out.trim().is_empty() {
        Some(out)
    } else {
        None
    }
}

fn configured_memory_store_ids() -> Vec<String> {
    let raw = std::env::var("JFC_MEMORY_STORE_IDS")
        .ok()
        .or_else(|| std::env::var("JFC_MEMORY_STORE_ID").ok())
        .unwrap_or_default();
    raw.split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

fn render_sdk_memory_content(memory: &jfc_anthropic_sdk::memory_stores::Memory) -> String {
    let content = memory
        .content
        .as_ref()
        .map(memory_value_to_text)
        .or_else(|| {
            memory
                .extra
                .get("content")
                .or_else(|| memory.extra.get("text"))
                .or_else(|| memory.extra.get("body"))
                .map(memory_value_to_text)
        })
        .unwrap_or_else(|| "(empty)".to_owned());
    content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(1_000)
        .collect()
}

fn memory_value_to_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(map) => {
            for key in ["text", "content", "body", "value"] {
                if let Some(text) = map.get(key).and_then(|v| v.as_str()) {
                    return text.to_owned();
                }
            }
            serde_json::to_string(value).unwrap_or_else(|_| value.to_string())
        }
        _ => serde_json::to_string(value).unwrap_or_else(|_| value.to_string()),
    }
}
