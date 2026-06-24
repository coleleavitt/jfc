//! DB-backed append-only journal for workflow resume.
//!
//! Each `agent()` call produces a chain-hash key:
//!   key = "v2:" + sha256(running_hash + "\0" + prompt + "\0" + opts_json)
//! The running_hash chains: each agent's key becomes the next's running_hash
//! input, giving "longest unchanged prefix" semantics.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

const GLOBAL_WORKFLOW_SESSION_ID: &str = "__workflow_global__";
const WORKFLOW_JOURNAL_KIND: &str = "workflow_journal";

/// A single journal entry (one line in the JSONL file).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum JournalEntry {
    #[serde(rename = "started")]
    Started { key: String, agent_id: String },
    #[serde(rename = "result")]
    Result {
        key: String,
        agent_id: String,
        result: serde_json::Value,
    },
}

/// Loaded journal state for resume.
pub struct JournalCache {
    pub results: HashMap<String, serde_json::Value>,
    pub started: HashMap<String, Vec<String>>,
}

/// Append-only journal writer.
pub struct JournalWriter {
    session_id: String,
    run_id: String,
}

impl JournalWriter {
    pub fn new(session_id: Option<&str>, run_id: &str) -> Self {
        Self {
            session_id: session_id
                .filter(|s| !s.trim().is_empty())
                .unwrap_or(GLOBAL_WORKFLOW_SESSION_ID)
                .to_owned(),
            run_id: run_id.to_owned(),
        }
    }

    pub async fn append(&self, entry: &JournalEntry) -> std::io::Result<()> {
        let session_id = self.session_id.clone();
        let run_id = self.run_id.clone();
        let json = serde_json::to_string(entry).map_err(io_invalid)?;
        tokio::task::spawn_blocking(move || {
            jfc_knowledge::block_on_knowledge(async {
                let store = jfc_knowledge::KnowledgeStore::open_default().await.map_err(io_other)?;
                store
                    .append_session_artifact_event(&session_id, WORKFLOW_JOURNAL_KIND, &run_id, &json)
                    .await
                    .map_err(io_other)?;
                Ok(())
            })
        })
        .await
        .map_err(io_other)?
    }

    pub fn label(&self) -> String {
        format!("db:{}/{}", self.session_id, self.run_id)
    }
}

/// Load a journal into a cache for resume lookups.
pub async fn load_journal(session_id: Option<&str>, run_id: &str) -> JournalCache {
    let session_id = session_id
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(GLOBAL_WORKFLOW_SESSION_ID)
        .to_owned();
    let run_id = run_id.to_owned();
    let entries = tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default().await.ok()?;
            let rows = store
                .list_session_artifact_events(
                    &session_id,
                    WORKFLOW_JOURNAL_KIND,
                    Some(&run_id),
                    100_000,
                )
                .await
                .ok()?;
            Some(
                rows.into_iter()
                    .filter_map(|row| serde_json::from_str::<JournalEntry>(&row.value_json).ok())
                    .collect::<Vec<_>>(),
            )
        })
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default();

    let mut results = HashMap::new();
    let mut started: HashMap<String, Vec<String>> = HashMap::new();
    for entry in entries {
        match entry {
            JournalEntry::Started { key, agent_id } => {
                started.entry(key).or_default().push(agent_id);
            }
            JournalEntry::Result {
                key,
                agent_id: _,
                result,
            } => {
                results.insert(key, result);
            }
        }
    }

    JournalCache { results, started }
}

fn io_invalid(error: impl std::error::Error + Send + Sync + 'static) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

fn io_other(error: impl std::error::Error + Send + Sync + 'static) -> std::io::Error {
    std::io::Error::other(error)
}

/// Compute the chain-hash key for an agent() call.
/// `running_hash` is the hex-encoded hash from the previous agent call
/// (empty string for the first call).
pub fn compute_key(running_hash: &str, prompt: &str, opts_json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(running_hash.as_bytes());
    hasher.update(b"\0");
    hasher.update(prompt.as_bytes());
    hasher.update(b"\0");
    hasher.update(opts_json.as_bytes());
    let hash = hex::encode(hasher.finalize());
    format!("v2:{hash}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compute_key_deterministic_normal() {
        let k1 = compute_key("", "hello", "{}");
        let k2 = compute_key("", "hello", "{}");
        assert_eq!(k1, k2);
        assert!(k1.starts_with("v2:"));
    }

    #[test]
    fn compute_key_chains_correctly_normal() {
        let k1 = compute_key("", "first", "{}");
        let k2 = compute_key(&k1, "second", "{}");
        let k3 = compute_key("", "second", "{}");
        // k2 != k3 because running_hash differs
        assert_ne!(k2, k3);
    }

    #[test]
    fn compute_key_different_prompts_differ_normal() {
        let k1 = compute_key("", "hello", "{}");
        let k2 = compute_key("", "world", "{}");
        assert_ne!(k1, k2);
    }

    #[tokio::test]
    async fn journal_round_trip_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let prior = std::env::var("JFC_KNOWLEDGE_DB").ok();
        unsafe {
            std::env::set_var("JFC_KNOWLEDGE_DB", tmp.path().join("knowledge.db"));
        }
        struct Guard(Option<String>);
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    match self.0.take() {
                        Some(prev) => std::env::set_var("JFC_KNOWLEDGE_DB", prev),
                        None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                    }
                }
            }
        }
        let _guard = Guard(prior);
        let writer = JournalWriter::new(Some("ses_wf_test"), "wf_test123");

        writer
            .append(&JournalEntry::Started {
                key: "v2:abc".into(),
                agent_id: "agent_1".into(),
            })
            .await
            .unwrap();

        writer
            .append(&JournalEntry::Result {
                key: "v2:abc".into(),
                agent_id: "agent_1".into(),
                result: serde_json::json!("hello world"),
            })
            .await
            .unwrap();

        let cache = load_journal(Some("ses_wf_test"), "wf_test123").await;
        assert_eq!(cache.results.len(), 1);
        assert_eq!(cache.results["v2:abc"], serde_json::json!("hello world"));
        assert_eq!(cache.started["v2:abc"].len(), 1);
    }
}
