//! Cutover #2 (first increment): a typed-[`SessionEntry`] JSONL sidecar written
//! alongside the live DB transcript save.
//!
//! ADDITIVE + fire-and-forget: the DB `save_session_transcript_to_db` and the
//! DB-only load path are untouched, and nothing reads this sidecar yet. It is
//! the first step toward the append-entry session substrate (PLAN.md Wave E) —
//! it proves the `ChatMessage` → `SessionEntry` projection on the live save path
//! without changing any persistence behavior. This first pass converts only
//! user/assistant TEXT messages; tool/thinking/compaction parts and full
//! metadata fidelity are a later, test-backed step.
//!
//! Note: it writes a full snapshot each save (not yet a true append-only log);
//! concurrent saves can race the unread file, which is harmless until a reader
//! exists.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::{ChatMessage, MessagePart};
use jfc_session::{MessageContentPart, SessionEntry, SessionEntryId, SessionEntryKind};

fn sidecar_path(session_id: &str) -> PathBuf {
    jfc_session::sessions_dir().join(format!("{session_id}.jsonl"))
}

fn now_unix_string() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs().to_string())
        .unwrap_or_default()
}

/// Project the transcript's user/assistant text messages into typed entries.
fn to_entries(session_id: &str, messages: &[ChatMessage]) -> Vec<SessionEntry> {
    let timestamp = now_unix_string();
    let mut entries = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let text = message
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n");
        if text.trim().is_empty() {
            continue;
        }
        let content = vec![MessageContentPart::text(text)];
        let kind = if message.role_is_user() {
            SessionEntryKind::user_message(content)
        } else {
            SessionEntryKind::assistant_message(content)
        };
        let id = SessionEntryId::new(format!("{session_id}-{index}"));
        entries.push(SessionEntry::new(id, timestamp.clone(), kind));
    }
    entries
}

/// Write the typed-entry sidecar (full snapshot, one JSON object per line).
/// Best-effort: every error is logged at debug and swallowed, so it can never
/// affect the real save. Intended to be `tokio::spawn`-ed detached.
pub async fn write_sidecar(session_id: String, messages: Vec<ChatMessage>) {
    let entries = to_entries(&session_id, &messages);
    let body = match entries
        .iter()
        .map(serde_json::to_string)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(lines) => lines.join("\n"),
        Err(error) => {
            tracing::debug!(target: "jfc::session::entry_log", %error, "sidecar serialize failed");
            return;
        }
    };
    let path = sidecar_path(&session_id);
    if let Err(error) = tokio::fs::write(&path, body).await {
        tracing::debug!(
            target: "jfc::session::entry_log",
            %error,
            path = %path.display(),
            "sidecar write failed"
        );
    }
}
