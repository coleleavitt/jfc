//! Per-session inbox: lightweight inter-session messaging.

use serde::{Deserialize, Serialize};

const INBOX_KIND: &str = "inbox";
const INBOX_KEY: &str = "message";

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

/// Append a message to a session's inbox.
pub async fn write_message(
    session_id: &str,
    from: Option<&str>,
    text: &str,
) -> std::io::Result<()> {
    let session_id = session_id.to_owned();
    let msg = SessionInboxMessage {
        text: text.to_owned(),
        from: from.map(str::to_owned),
        timestamp: chrono::Utc::now().to_rfc3339(),
        read: false,
    };
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default()
                .await
                .map_err(io_other)?;
            let json = serde_json::to_string(&msg).map_err(io_invalid)?;
            store
                .append_session_artifact_event(&session_id, INBOX_KIND, INBOX_KEY, &json)
                .await
                .map_err(io_other)?;
            Ok::<_, std::io::Error>(())
        })
    })
    .await
    .map_err(io_other)?
}

/// Read all messages for a session inbox.
pub async fn read_messages(session_id: &str) -> Vec<SessionInboxMessage> {
    let session_id = session_id.to_owned();
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default().await.ok()?;
            let rows = store
                .list_session_artifact_events(&session_id, INBOX_KIND, Some(INBOX_KEY), 10_000)
                .await
                .ok()?;
            Some(
                rows.into_iter()
                    .filter_map(|row| {
                        serde_json::from_str::<SessionInboxMessage>(&row.value_json).ok()
                    })
                    .collect(),
            )
        })
    })
    .await
    .ok()
    .flatten()
    .unwrap_or_default()
}

/// Clear a session's inbox.
pub async fn clear_inbox(session_id: &str) -> std::io::Result<()> {
    let session_id = session_id.to_owned();
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = jfc_knowledge::KnowledgeStore::open_default()
                .await
                .map_err(io_other)?;
            store
                .clear_session_artifact_events(&session_id, INBOX_KIND, Some(INBOX_KEY))
                .await
                .map_err(io_other)?;
            Ok::<_, std::io::Error>(())
        })
    })
    .await
    .map_err(io_other)?
}

fn io_invalid(error: impl std::error::Error + Send + Sync + 'static) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error)
}

fn io_other(error: impl std::error::Error + Send + Sync + 'static) -> std::io::Error {
    std::io::Error::other(error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    struct TempKnowledgeDb {
        _dir: TempDir,
        prior: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl TempKnowledgeDb {
        fn new() -> Self {
            let guard = crate::TEST_ENV_LOCK
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            let dir = TempDir::new().expect("tempdir");
            let prior = std::env::var("JFC_KNOWLEDGE_DB").ok();
            unsafe {
                std::env::set_var("JFC_KNOWLEDGE_DB", dir.path().join("knowledge.db"));
            }
            Self {
                _dir: dir,
                prior,
                _guard: guard,
            }
        }
    }

    impl Drop for TempKnowledgeDb {
        fn drop(&mut self) {
            unsafe {
                match self.prior.take() {
                    Some(prev) => std::env::set_var("JFC_KNOWLEDGE_DB", prev),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
            }
        }
    }

    #[tokio::test]
    async fn write_then_read_round_trip_normal() {
        let _g = TempKnowledgeDb::new();
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
        let _g = TempKnowledgeDb::new();
        write_message("ses_abc", None, "m1").await.unwrap();
        clear_inbox("ses_abc").await.unwrap();
        let msgs = read_messages("ses_abc").await;
        assert!(msgs.is_empty());
    }
}
