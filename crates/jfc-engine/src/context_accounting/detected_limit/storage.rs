use serde::{Deserialize, Serialize};

use super::{DetectedContextLimit, plausible_limit};

const DETECTED_CONTEXT_LIMIT_KIND: &str = "detected_context_limit";
const DETECTED_CONTEXT_LIMIT_KEY: &str = "active";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredDetectedContextLimit {
    model: String,
    limit_tokens: usize,
    actual_tokens: Option<usize>,
    detected_at_ms: i64,
}

pub(crate) async fn persist_session_detected_context_limit(
    session_id: &str,
    model: &str,
    detected: DetectedContextLimit,
) -> anyhow::Result<()> {
    let store = jfc_knowledge::KnowledgeStore::open_default().await?;
    let value = StoredDetectedContextLimit {
        model: model.to_owned(),
        limit_tokens: detected.limit_tokens,
        actual_tokens: detected.actual_tokens,
        detected_at_ms: chrono::Utc::now().timestamp_millis(),
    };
    let json = serde_json::to_string(&value)?;
    store
        .upsert_session_artifact(
            session_id,
            DETECTED_CONTEXT_LIMIT_KIND,
            DETECTED_CONTEXT_LIMIT_KEY,
            &json,
        )
        .await?;
    Ok(())
}

pub(crate) async fn load_session_detected_context_limit(
    session_id: &str,
    model: &str,
) -> Option<DetectedContextLimit> {
    let store = jfc_knowledge::KnowledgeStore::open_default().await.ok()?;
    let artifact = store
        .get_session_artifact(
            session_id,
            DETECTED_CONTEXT_LIMIT_KIND,
            DETECTED_CONTEXT_LIMIT_KEY,
        )
        .await
        .ok()??;
    let stored: StoredDetectedContextLimit = serde_json::from_str(&artifact.value_json).ok()?;
    if stored.model != model {
        return None;
    }
    let limit_tokens = plausible_limit(stored.limit_tokens)?;
    Some(DetectedContextLimit {
        actual_tokens: stored.actual_tokens,
        limit_tokens,
    })
}
