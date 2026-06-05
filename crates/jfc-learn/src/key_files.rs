//! Key File Pinning — track frequently-read files and pin them for system prompt inclusion.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::LearnError;

// ─── Types ──────────────────────────────────────────────────────────────────

/// A pinned file entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PinnedFile {
    pub file_path: String,
    pub content_hash: String,
    pub last_pinned_at: u64,
    pub reason: String,
}

/// A file read event for tracking.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadEvent {
    pub file_path: String,
    pub session_id: String,
    pub read_at_ms: u64,
}

// ─── Store ──────────────────────────────────────────────────────────────────

/// Store for key file tracking and pinning.
pub struct KeyFileStore {
    pub root: PathBuf,
}

impl KeyFileStore {
    /// Open (or create) the key-files store directory.
    pub fn open(root: &Path) -> Result<Self, LearnError> {
        let store_root = root.join(".jfc").join("key-files");
        fs::create_dir_all(&store_root)?;
        Ok(Self { root: store_root })
    }

    /// Record a file read event.
    pub fn record_read(&self, event: &ReadEvent) -> Result<(), LearnError> {
        let path = self.root.join("reads.jsonl");
        let mut file = OpenOptions::new().create(true).append(true).open(&path)?;
        let line = serde_json::to_string(event)?;
        writeln!(file, "{}", line)?;
        Ok(())
    }

    /// Load read history from the JSONL file.
    pub fn load_read_history(&self) -> Result<Vec<ReadEvent>, LearnError> {
        let path = self.root.join("reads.jsonl");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&path)?;
        let reader = BufReader::new(file);
        let mut events = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() {
                continue;
            }
            if let Ok(event) = serde_json::from_str::<ReadEvent>(&line) {
                events.push(event);
            }
        }
        Ok(events)
    }

    /// Identify candidate files for pinning: files read in ≥min_sessions distinct sessions.
    pub fn identify_candidates(reads: &[ReadEvent], min_sessions: u32) -> Vec<String> {
        use std::collections::{HashMap, HashSet};

        let mut file_sessions: HashMap<&str, HashSet<&str>> = HashMap::new();
        for event in reads {
            file_sessions
                .entry(&event.file_path)
                .or_default()
                .insert(&event.session_id);
        }

        let mut candidates: Vec<String> = file_sessions
            .into_iter()
            .filter(|(_, sessions)| sessions.len() >= min_sessions as usize)
            .map(|(path, _)| path.to_string())
            .collect();
        candidates.sort();
        candidates
    }

    /// Pin a file.
    pub fn pin(&self, file_path: &str, reason: &str) -> Result<(), LearnError> {
        let mut pinned = self.load_pinned_internal()?;

        // Compute content hash
        let content_hash = if let Ok(content) = fs::read_to_string(file_path) {
            use sha2::{Digest, Sha256};
            let hash = Sha256::new().chain_update(content.as_bytes()).finalize();
            hash.iter().map(|b| format!("{b:02x}")).collect::<String>()
        } else {
            String::from("missing")
        };

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;

        // Remove existing entry for same path
        pinned.retain(|p| p.file_path != file_path);
        pinned.push(PinnedFile {
            file_path: file_path.to_string(),
            content_hash,
            last_pinned_at: now_ms,
            reason: reason.to_string(),
        });

        self.save_pinned(&pinned)?;
        Ok(())
    }

    /// Unpin a file.
    pub fn unpin(&self, file_path: &str) -> Result<(), LearnError> {
        let mut pinned = self.load_pinned_internal()?;
        pinned.retain(|p| p.file_path != file_path);
        self.save_pinned(&pinned)?;
        Ok(())
    }

    /// List all pinned files.
    pub fn list_pinned(&self) -> Result<Vec<PinnedFile>, LearnError> {
        self.load_pinned_internal()
    }

    /// Render a `<key-files>` XML block from pinned files, reading content from disk.
    /// Truncates total output to approximately budget_tokens (rough: 4 chars per token).
    pub fn render_key_files_block(pinned: &[PinnedFile], budget_tokens: u32) -> String {
        if pinned.is_empty() {
            return String::new();
        }

        let budget_chars = (budget_tokens as usize) * 4;
        let mut out = String::from("<key-files>\n");
        let mut remaining = budget_chars.saturating_sub(out.len() + 13); // 13 for closing tag

        for pf in pinned {
            let content = match fs::read_to_string(&pf.file_path) {
                Ok(c) => c,
                Err(_) => {
                    let sentinel =
                        format!("  <file path=\"{}\" status=\"missing\" />\n", pf.file_path);
                    if sentinel.len() <= remaining {
                        out.push_str(&sentinel);
                        remaining -= sentinel.len();
                    }
                    continue;
                }
            };

            let header = format!("  <file path=\"{}\">\n", pf.file_path);
            let footer = "  </file>\n";
            let overhead = header.len() + footer.len();

            if overhead >= remaining {
                break;
            }

            let max_content = remaining - overhead;
            let truncated_content = if content.len() > max_content {
                format!(
                    "{}...(truncated)",
                    &content[..max_content.saturating_sub(15)]
                )
            } else {
                content
            };

            let entry = format!("{}{}\n{}", header, truncated_content, footer);
            if entry.len() <= remaining {
                out.push_str(&entry);
                remaining -= entry.len();
            } else {
                break;
            }
        }

        out.push_str("</key-files>");
        out
    }

    // ─── Internal helpers ───────────────────────────────────────────────

    fn load_pinned_internal(&self) -> Result<Vec<PinnedFile>, LearnError> {
        let path = self.root.join("pinned.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = fs::read_to_string(&path)?;
        let pinned: Vec<PinnedFile> = serde_json::from_str(&content)?;
        Ok(pinned)
    }

    fn save_pinned(&self, pinned: &[PinnedFile]) -> Result<(), LearnError> {
        let path = self.root.join("pinned.json");
        let json = serde_json::to_string_pretty(pinned)?;
        fs::write(&path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn record_and_identify_candidates_normal() {
        let tmp = TempDir::new().unwrap();
        let store = KeyFileStore::open(tmp.path()).unwrap();

        // Record reads from 3 different sessions
        for i in 0..3 {
            store
                .record_read(&ReadEvent {
                    file_path: "src/main.rs".to_string(),
                    session_id: format!("session-{}", i),
                    read_at_ms: 1700000000 + i * 1000,
                })
                .unwrap();
        }
        // Only 1 session for another file
        store
            .record_read(&ReadEvent {
                file_path: "src/lib.rs".to_string(),
                session_id: "session-0".to_string(),
                read_at_ms: 1700000000,
            })
            .unwrap();

        let reads = store.load_read_history().unwrap();
        let candidates = KeyFileStore::identify_candidates(&reads, 3);
        assert_eq!(candidates, vec!["src/main.rs"]);
    }

    #[test]
    fn pin_and_list_normal() {
        let tmp = TempDir::new().unwrap();
        let store = KeyFileStore::open(tmp.path()).unwrap();

        // Create a real file to pin
        let file_path = tmp.path().join("test.rs");
        fs::write(&file_path, "fn main() {}").unwrap();

        store
            .pin(file_path.to_str().unwrap(), "frequently accessed")
            .unwrap();

        let pinned = store.list_pinned().unwrap();
        assert_eq!(pinned.len(), 1);
        assert_eq!(pinned[0].file_path, file_path.to_str().unwrap());
        assert_eq!(pinned[0].reason, "frequently accessed");
        assert_ne!(pinned[0].content_hash, "missing");

        // Unpin
        store.unpin(file_path.to_str().unwrap()).unwrap();
        let pinned = store.list_pinned().unwrap();
        assert!(pinned.is_empty());
    }

    #[test]
    fn render_block_truncates_to_budget_normal() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("big.rs");
        // Write a large file
        let content = "x".repeat(10000);
        fs::write(&file_path, &content).unwrap();

        let pinned = vec![PinnedFile {
            file_path: file_path.to_str().unwrap().to_string(),
            content_hash: "abc".to_string(),
            last_pinned_at: 0,
            reason: "test".to_string(),
        }];

        // Very small budget — 50 tokens = ~200 chars
        let rendered = KeyFileStore::render_key_files_block(&pinned, 50);
        assert!(rendered.len() < 300);
        assert!(rendered.contains("<key-files>"));
        assert!(rendered.contains("</key-files>"));
    }

    #[test]
    fn missing_file_renders_sentinel_robust() {
        let pinned = vec![PinnedFile {
            file_path: "/nonexistent/file.rs".to_string(),
            content_hash: "missing".to_string(),
            last_pinned_at: 0,
            reason: "test".to_string(),
        }];

        let rendered = KeyFileStore::render_key_files_block(&pinned, 1000);
        assert!(rendered.contains("status=\"missing\""));
        assert!(rendered.contains("/nonexistent/file.rs"));
    }
}
