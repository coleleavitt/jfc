//! JSONL append-only journal for workflow resume.
//!
//! Each `agent()` call produces a chain-hash key:
//!   key = "v2:" + sha256(running_hash + "\0" + prompt + "\0" + opts_json)
//! The running_hash chains: each agent's key becomes the next's running_hash
//! input, giving "longest unchanged prefix" semantics.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tokio::fs;
use tokio::io::AsyncWriteExt;

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
    path: PathBuf,
}

impl JournalWriter {
    pub fn new(session_dir: &Path, run_id: &str) -> Self {
        Self {
            path: session_dir.join(format!("workflow_journal_{run_id}.jsonl")),
        }
    }

    pub async fn append(&self, entry: &JournalEntry) -> std::io::Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let mut line = serde_json::to_string(entry).unwrap_or_default();
        line.push('\n');
        let mut file = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .await?;
        file.write_all(line.as_bytes()).await?;
        file.flush().await?;
        Ok(())
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Load a journal file into a cache for resume lookups.
pub async fn load_journal(session_dir: &Path, run_id: &str) -> JournalCache {
    let path = session_dir.join(format!("workflow_journal_{run_id}.jsonl"));
    let mut results = HashMap::new();
    let mut started: HashMap<String, Vec<String>> = HashMap::new();

    let content = match fs::read_to_string(&path).await {
        Ok(c) => c,
        Err(_) => return JournalCache { results, started },
    };

    for line in content.lines() {
        if line.is_empty() {
            continue;
        }
        let Ok(entry) = serde_json::from_str::<JournalEntry>(line) else {
            continue;
        };
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
        let writer = JournalWriter::new(tmp.path(), "wf_test123");

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

        let cache = load_journal(tmp.path(), "wf_test123").await;
        assert_eq!(cache.results.len(), 1);
        assert_eq!(cache.results["v2:abc"], serde_json::json!("hello world"));
        assert_eq!(cache.started["v2:abc"].len(), 1);
    }
}
