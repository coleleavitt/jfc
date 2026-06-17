//! Per-session inbox: lightweight inter-session messaging.
//!
//! Messages are stored under XDG config dir:
//!   ~/.config/jfc/session-inbox/<session_id>.json
//! as a JSON array of `SessionInboxMessage`.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::fs;

/// Message persisted in a session's inbox file.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SessionInboxMessage {
    pub text: String,
    #[serde(default)]
    pub from: Option<String>,
    pub timestamp: String,
    #[serde(default)]
    pub read: bool,
}

/// Root directory for session inbox files.
pub fn session_inbox_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("session-inbox")
}

fn inbox_path(session_id: &str) -> PathBuf {
    session_inbox_dir().join(format!("{session_id}.json"))
}

async fn read_messages_unlocked(path: &Path) -> Vec<SessionInboxMessage> {
    match fs::read_to_string(path).await {
        Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => Vec::new(),
    }
}

/// Append a message to a session's inbox, creating the file if needed.
pub async fn write_message(
    session_id: &str,
    from: Option<&str>,
    text: &str,
) -> std::io::Result<()> {
    let dir = session_inbox_dir();
    fs::create_dir_all(&dir).await?;
    let path = inbox_path(session_id);
    if !path.exists() {
        fs::write(&path, "[]").await?;
    }
    let mut msgs = read_messages_unlocked(&path).await;
    msgs.push(SessionInboxMessage {
        text: text.to_owned(),
        from: from.map(str::to_owned),
        timestamp: chrono::Utc::now().to_rfc3339(),
        read: false,
    });
    let json = serde_json::to_string_pretty(&msgs)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    fs::write(&path, json).await
}

/// Read all messages for a session inbox.
pub async fn read_messages(session_id: &str) -> Vec<SessionInboxMessage> {
    let path = inbox_path(session_id);
    read_messages_unlocked(&path).await
}

/// Clear a session's inbox file (empties the array; creates file if missing).
pub async fn clear_inbox(session_id: &str) -> std::io::Result<()> {
    let dir = session_inbox_dir();
    fs::create_dir_all(&dir).await?;
    let path = inbox_path(session_id);
    fs::write(&path, "[]").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    struct TempConfigHome {
        _dir: TempDir,
        prior: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempConfigHome {
        fn new() -> Self {
            // Serialize env var mutation across tests in this process.
            let guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new().expect("tempdir");
            let prior = std::env::var("XDG_CONFIG_HOME").ok();
            unsafe {
                std::env::set_var("XDG_CONFIG_HOME", dir.path());
            }
            Self {
                _dir: dir,
                prior,
                _guard: guard,
            }
        }
    }

    impl Drop for TempConfigHome {
        fn drop(&mut self) {
            unsafe {
                match self.prior.take() {
                    Some(prev) => std::env::set_var("XDG_CONFIG_HOME", prev),
                    None => std::env::remove_var("XDG_CONFIG_HOME"),
                }
            }
        }
    }

    #[tokio::test]
    async fn write_then_read_round_trip_normal() {
        let _g = TempConfigHome::new();
        clear_inbox("ses_123").await.unwrap();
        write_message("ses_123", Some("ses_src"), "hello world")
            .await
            .unwrap();
        let msgs = read_messages("ses_123").await;
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text, "hello world");
        assert_eq!(msgs[0].from.as_deref(), Some("ses_src"));
        assert!(!msgs[0].read);
    }

    #[tokio::test]
    async fn clear_inbox_empties_messages_normal() {
        let _g = TempConfigHome::new();
        write_message("ses_abc", None, "m1").await.unwrap();
        clear_inbox("ses_abc").await.unwrap();
        let msgs = read_messages("ses_abc").await;
        assert!(msgs.is_empty());
    }
}
