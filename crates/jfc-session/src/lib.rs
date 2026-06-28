//! Session catalog and path helpers.
//!
use std::path::PathBuf;
#[cfg(test)]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

use jfc_core::SessionId;
use jfc_knowledge::{
    KnowledgeStore, SessionMessage as KnowledgeSessionMessage, SessionRow as KnowledgeSessionRow,
};
use serde_json::json;
use tracing::debug;

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
fn test_env_lock() -> std::sync::MutexGuard<'static, ()> {
    TEST_ENV_LOCK.lock().unwrap_or_else(|err| err.into_inner())
}

#[cfg(test)]
pub(crate) struct TestKnowledgeDb {
    root: tempfile::TempDir,
    previous_db: Option<std::ffi::OsString>,
    _lock: std::sync::MutexGuard<'static, ()>,
}

#[cfg(test)]
impl TestKnowledgeDb {
    pub(crate) fn new() -> Self {
        let lock = test_env_lock();
        let root = tempfile::tempdir().expect("tempdir");
        let previous_db = std::env::var_os("JFC_KNOWLEDGE_DB");
        unsafe {
            std::env::set_var("JFC_KNOWLEDGE_DB", root.path().join("knowledge.db"));
        }
        Self {
            root,
            previous_db,
            _lock: lock,
        }
    }

    pub(crate) fn root(&self) -> &std::path::Path {
        self.root.path()
    }
}

#[cfg(test)]
impl Drop for TestKnowledgeDb {
    fn drop(&mut self) {
        unsafe {
            match self.previous_db.take() {
                Some(value) => std::env::set_var("JFC_KNOWLEDGE_DB", value),
                None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
            }
        }
    }
}

pub(crate) async fn open_default_knowledge_store() -> jfc_knowledge::Result<KnowledgeStore> {
    #[cfg(test)]
    {
        static TEST_DB_OPEN_LOCK: OnceLock<tokio::sync::Mutex<()>> = OnceLock::new();
        let lock = TEST_DB_OPEN_LOCK.get_or_init(|| tokio::sync::Mutex::new(()));
        let _guard = lock.lock().await;
        KnowledgeStore::open_default().await
    }

    #[cfg(not(test))]
    {
        KnowledgeStore::open_default().await
    }
}

mod catalog;
mod entry;
mod git_commits;
mod inbox;
mod search;
mod session_entry;
mod soft_match;
mod store;
mod task_history;
mod task_store;
pub mod transcript;

pub use catalog::{
    SessionMetadata, cwd_mismatch_message, format_session_id_timestamp, group_by_cwd,
    has_any_session, list_session_ids_only, list_sessions, list_sessions_filtered,
    list_sessions_with_metadata, load_session_metadata, most_recent_session,
    most_recent_session_for_cwd, relative_time, shorten_cwd,
};
pub use git_commits::{CommitHit, search as search_commits};
pub use inbox::{
    SessionInboxMessage, clear_inbox as clear_inbox_for_session,
    read_messages as read_inbox_for_session, write_message as write_inbox_message,
};
pub use jfc_core::{TaskBountyRef, TaskExecutionMetadata, TaskExecutionMode};
pub use search::{
    SessionBrief, SessionHit, SessionMessage, browse as browse_sessions,
    discover as search_sessions, discover_excluding as search_sessions_excluding,
    prior_user_prompts, scroll as scroll_session,
};
pub use session_entry::{
    BranchForkSummary, CompactionBoundary, ContextEvent, CustomPluginEntry, LabelEntry,
    MessageContentPart, MessageMetadata, ModelChange, SessionEntry, SessionEntryId,
    SessionEntryKind, SessionEntryValidationError, ThinkingChange, ToolResult, ToolUse,
};
pub use store::{
    AutosaveOutcome, AutosaveRequest, DefaultSessionStore, ListSessionsRequest,
    SaveTranscriptRequest, SearchSessionsRequest, SessionStore, SessionTranscript,
    StoredSessionMessage, default_session_store,
};
pub use task_history::{
    TaskHistoryRecord, history_key_for_store_path, read_records as read_task_history,
    session_history_key,
};
pub use task_store::{
    DeletedFilter, FactoryMetrics, FailureRecovery, Task, TaskCounts, TaskError, TaskId, TaskKind,
    TaskPatch, TaskRisk, TaskStatus, TaskStore, TaskValidation, is_transient_failure,
    task_store_path, task_stores_dir, team_task_store_path, team_tasks_dir,
};
pub use transcript::{
    SerializedDiffHunk, SerializedDiffLine, SerializedMessage, SerializedPart, SerializedSession,
    SerializedToolInput, SerializedToolOutput, SerializedToolPart,
};

pub fn sessions_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("jfc")
        .join("sessions")
}

static SESSION_ID_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn generate_session_id() -> SessionId {
    let now = chrono::Utc::now();
    let counter = SESSION_ID_COUNTER.fetch_add(1, Ordering::Relaxed) & 0xffff;
    let id = SessionId::new(format!(
        "ses_{}_{:06}_{counter:04x}",
        now.format("%Y%m%d_%H%M%S"),
        now.timestamp_subsec_micros()
    ));
    debug!(target: "jfc::session", %id, "generated session id");
    id
}

/// Remove DB sessions older than `max_age_days`. Keep at least `min_keep`
/// most-recent sessions regardless of age.
pub async fn gc_old_sessions(max_age_days: u64, min_keep: usize) -> std::io::Result<usize> {
    if max_age_days == 0 {
        return Ok(0);
    }
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let store = match open_default_knowledge_store().await {
                Ok(store) => store,
                Err(_) => return Ok(0),
            };
            let mut rows = store
                .list_sessions(None, 100_000)
                .await
                .map_err(io_other)?
                .into_iter()
                .map(|row| {
                    let timestamp = row.updated_at.clone().or_else(|| row.created_at.clone());
                    let parsed = timestamp
                        .as_deref()
                        .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                        .map(|value| value.with_timezone(&chrono::Utc));
                    (parsed, row.id)
                })
                .collect::<Vec<_>>();
            rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
            let cutoff = chrono::Utc::now()
                - chrono::Duration::days(i64::try_from(max_age_days).unwrap_or(i64::MAX));
            let mut deleted = 0usize;
            for (index, (timestamp, id)) in rows.into_iter().enumerate() {
                if index < min_keep || timestamp.is_none_or(|value| value >= cutoff) {
                    continue;
                }
                if store.delete_session(&id).await.map_err(io_other)? > 0 {
                    deleted += 1;
                    debug!(
                        target: "jfc::session::gc",
                        session_id = id,
                        "gc_old_sessions: removed stale DB session"
                    );
                }
            }
            Ok(deleted)
        })
    })
    .await
    .unwrap_or_else(|err| Err(io_other(err)))
}

pub async fn fork_session(source_id: &str, description: &str) -> std::io::Result<String> {
    let fork_id = generate_session_id();
    let fork_id_str = fork_id.as_str().to_owned();

    if fork_session_in_db(source_id, &fork_id_str, description)? {
        debug!(
            target: "jfc::session",
            source_id,
            fork_id = %fork_id_str,
            description,
            "fork_session: created DB fork"
        );
        return Ok(fork_id_str);
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!("session `{source_id}` not found"),
    ))
}

fn io_other(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::other(error.to_string())
}

const FSCK_QUARANTINE_SESSION_ID: &str = "__session_fsck__";
const FSCK_QUARANTINE_KIND: &str = "quarantined_session";

#[cfg(test)]
mod tests {
    use jfc_knowledge::{
        SessionMessage as KnowledgeSessionMessage, SessionRow as KnowledgeSessionRow,
    };

    use super::{
        FSCK_QUARANTINE_KIND, FSCK_QUARANTINE_SESSION_ID, fsck_sessions, generate_session_id,
    };

    #[test]
    fn generated_session_ids_are_unique_within_one_second_regression() {
        let first = generate_session_id();
        let second = generate_session_id();

        assert_ne!(first, second);
        assert!(first.as_str().starts_with("ses_"));
        assert!(second.as_str().starts_with("ses_"));
    }

    #[tokio::test]
    async fn session_store_facade_save_title_load_list_normal() {
        let _db = super::TestKnowledgeDb::new();
        let store = super::default_session_store();
        let session_id = super::SessionId::new("ses_20260628_120000_facade");
        let messages = vec![
            super::StoredSessionMessage {
                seq: 0,
                role: "user".to_owned(),
                content: "remember facade ownership".to_owned(),
                meta: Some(r#"{"role":"user","parts":[{"type":"text","content":"remember facade ownership"}]}"#.to_owned()),
            },
            super::StoredSessionMessage {
                seq: 1,
                role: "assistant".to_owned(),
                content: "facade persisted".to_owned(),
                meta: Some(r#"{"role":"assistant","parts":[{"type":"text","content":"facade persisted"}]}"#.to_owned()),
            },
        ];

        super::SessionStore::save_transcript(
            &store,
            super::SaveTranscriptRequest::new(&session_id, &messages)
                .with_cwd(Some("/tmp/jfc-session-facade"))
                .with_model(Some("facade-model"))
                .with_first_prompt(Some("remember facade ownership")),
        )
        .await;
        super::SessionStore::set_title(&store, &session_id, "Facade title").await;

        let loaded = super::SessionStore::load_transcript(&store, &session_id)
            .await
            .expect("facade loads saved transcript");
        let listed = super::SessionStore::list_sessions(
            &store,
            super::ListSessionsRequest {
                cwd_filter: Some("/tmp/jfc-session-facade"),
                limit: Some(1),
            },
        )
        .await;

        assert_eq!(loaded.model.as_deref(), Some("facade-model"));
        assert_eq!(loaded.messages, messages);
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id.as_str(), session_id.as_str());
        assert_eq!(listed[0].title.as_deref(), Some("Facade title"));
        assert_eq!(listed[0].display_title(), "Facade title");
    }

    #[tokio::test]
    async fn session_store_facade_missing_and_empty_behaviors_robust() {
        let _db = super::TestKnowledgeDb::new();
        let store = super::default_session_store();
        let missing = super::SessionId::new("ses_missing_facade");

        assert!(
            super::SessionStore::load_transcript(&store, &missing)
                .await
                .is_none()
        );
        assert!(
            super::SessionStore::list_sessions(&store, super::ListSessionsRequest::all())
                .await
                .is_empty()
        );
        assert!(
            super::SessionStore::search_sessions(
                &store,
                super::SearchSessionsRequest {
                    query: "",
                    limit: 10,
                    window: 1,
                    exclude_session: None,
                },
            )
            .is_empty()
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn fsck_quarantine_moves_bad_db_session_regression() {
        let _db = super::TestKnowledgeDb::new();

        {
            let store = super::open_default_knowledge_store().await.unwrap();
            store
                .replace_transcript(
                    &KnowledgeSessionRow {
                        id: "bad-session".into(),
                        cwd: Some("/tmp/project".into()),
                        model: Some("claude".into()),
                        created_at: Some("2026-01-01T00:00:00Z".into()),
                        updated_at: Some("2026-01-01T00:00:01Z".into()),
                        first_prompt: Some("hello".into()),
                        title: Some("bad".into()),
                        message_count: 2,
                    },
                    &[KnowledgeSessionMessage {
                        seq: 0,
                        role: "user".into(),
                        content: "hello".into(),
                        meta: None,
                    }],
                )
                .await
                .unwrap();
        }

        let report = fsck_sessions(true).await.unwrap();
        let store = super::open_default_knowledge_store().await.unwrap();

        assert_eq!(report.checked, 1);
        assert_eq!(report.ok, 0);
        assert_eq!(report.quarantined(), 1);
        assert!((store.get_session("bad-session").await.unwrap()).is_none());
        let artifacts = store
            .list_session_artifacts(FSCK_QUARANTINE_SESSION_ID, FSCK_QUARANTINE_KIND, 10)
            .await
            .unwrap();
        assert_eq!(artifacts.len(), 1);
        assert!(artifacts[0].value_json.contains("\"id\":\"bad-session\""));
    }
}

fn fork_session_in_db(source_id: &str, fork_id: &str, description: &str) -> std::io::Result<bool> {
    jfc_knowledge::block_on_knowledge(async {
        let store = match open_default_knowledge_store().await {
            Ok(store) => store,
            Err(_) => return Ok(false),
        };
        let Some(mut row) = store.get_session(source_id).await.map_err(io_other)? else {
            return Ok(false);
        };
        let transcript = store.load_transcript(source_id).await.map_err(io_other)?;
        row.id = fork_id.to_owned();
        row.updated_at = Some(chrono::Utc::now().to_rfc3339());
        if !description.is_empty() {
            row.title = Some(format!("[fork] {description}"));
        }
        store
            .replace_transcript(&row, &transcript)
            .await
            .map_err(io_other)?;
        Ok(true)
    })
}

pub async fn delete_session(session_id: &str) -> std::io::Result<bool> {
    delete_session_from_db(session_id)
}

fn delete_session_from_db(session_id: &str) -> std::io::Result<bool> {
    jfc_knowledge::block_on_knowledge(async {
        let store = match open_default_knowledge_store().await {
            Ok(store) => store,
            Err(_) => return Ok(false),
        };
        let deleted = store.delete_session(session_id).await.map_err(io_other)?;
        Ok(deleted > 0)
    })
}

#[derive(Debug, Clone)]
pub struct SessionFsckIssue {
    pub path: PathBuf,
    pub reason: String,
    pub quarantined_to: Option<PathBuf>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionFsckReport {
    pub checked: usize,
    pub ok: usize,
    pub issues: Vec<SessionFsckIssue>,
}

impl SessionFsckReport {
    pub fn quarantined(&self) -> usize {
        self.issues
            .iter()
            .filter(|issue| issue.quarantined_to.is_some())
            .count()
    }
}

pub async fn fsck_sessions(quarantine: bool) -> std::io::Result<SessionFsckReport> {
    tokio::task::spawn_blocking(move || {
        jfc_knowledge::block_on_knowledge(async {
            let mut store = match open_default_knowledge_store().await {
                Ok(store) => store,
                Err(_) => return Ok(SessionFsckReport::default()),
            };
            let mut report = SessionFsckReport::default();
            for row in store.list_sessions(None, 100_000).await.map_err(io_other)? {
                report.checked += 1;
                let transcript = store.load_transcript(&row.id).await.map_err(io_other)?;
                if transcript.is_empty() && row.message_count > 0 {
                    let reason = "session row has no transcript messages".to_owned();
                    let quarantined_to = if quarantine {
                        Some(quarantine_session(&mut store, &row, &transcript, &reason).await?)
                    } else {
                        None
                    };
                    report.issues.push(SessionFsckIssue {
                        path: PathBuf::from(format!("db/{}", row.id)),
                        reason,
                        quarantined_to,
                    });
                    continue;
                }
                if row.message_count >= 0 && row.message_count as usize != transcript.len() {
                    let reason = format!(
                        "message_count {} does not match transcript length {}",
                        row.message_count,
                        transcript.len()
                    );
                    let quarantined_to = if quarantine {
                        Some(quarantine_session(&mut store, &row, &transcript, &reason).await?)
                    } else {
                        None
                    };
                    report.issues.push(SessionFsckIssue {
                        path: PathBuf::from(format!("db/{}", row.id)),
                        reason,
                        quarantined_to,
                    });
                    continue;
                }
                report.ok += 1;
            }
            Ok(report)
        })
    })
    .await
    .unwrap_or_else(|err| Err(io_other(err)))
}

async fn quarantine_session(
    store: &mut KnowledgeStore,
    row: &KnowledgeSessionRow,
    transcript: &[KnowledgeSessionMessage],
    reason: &str,
) -> std::io::Result<PathBuf> {
    let quarantined_at = chrono::Utc::now();
    let key = format!(
        "{}-{}",
        quarantined_at.timestamp_micros(),
        row.id
            .chars()
            .map(|ch| {
                if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
                    ch
                } else {
                    '-'
                }
            })
            .collect::<String>()
    );
    let messages = transcript
        .iter()
        .map(|message| {
            json!({
                "seq": message.seq,
                "role": message.role,
                "content": message.content,
                "meta": message.meta,
            })
        })
        .collect::<Vec<_>>();
    let payload = json!({
        "id": row.id,
        "cwd": row.cwd,
        "model": row.model,
        "created_at": row.created_at,
        "updated_at": row.updated_at,
        "first_prompt": row.first_prompt,
        "title": row.title,
        "message_count": row.message_count,
        "reason": reason,
        "quarantined_at": quarantined_at.to_rfc3339(),
        "transcript": messages,
    });
    store
        .upsert_session_artifact(
            FSCK_QUARANTINE_SESSION_ID,
            FSCK_QUARANTINE_KIND,
            &key,
            &payload.to_string(),
        )
        .await
        .map_err(io_other)?;
    store.delete_session(&row.id).await.map_err(io_other)?;
    Ok(PathBuf::from(format!(
        "db/{FSCK_QUARANTINE_SESSION_ID}/{FSCK_QUARANTINE_KIND}/{key}"
    )))
}
