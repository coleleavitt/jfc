//! Session catalog and path helpers.
//!
//! Full transcript serialization still lives in `jfc` while message/tool
//! types are being untangled. This crate owns the provider-neutral session
//! index surface: paths, IDs, metadata listing, and picker helpers.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use jfc_core::SessionId;
use tracing::debug;

#[cfg(test)]
pub(crate) static TEST_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

mod catalog;
mod git_commits;
mod inbox;
mod search;
mod soft_match;
mod task_history;
mod task_store;

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
pub use search::{
    SessionBrief, SessionHit, SessionMessage, browse as browse_sessions,
    discover as search_sessions, discover_excluding as search_sessions_excluding,
    prior_user_prompts, scroll as scroll_session,
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
        let mut store = match jfc_knowledge::KnowledgeStore::open_default() {
            Ok(store) => store,
            Err(_) => return Ok(0),
        };
        let mut rows = store
            .list_sessions(None, 100_000)
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
            if store.delete_session(&id).map_err(io_other)? > 0 {
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

#[cfg(test)]
mod tests {
    use super::generate_session_id;

    #[test]
    fn generated_session_ids_are_unique_within_one_second_regression() {
        let first = generate_session_id();
        let second = generate_session_id();

        assert_ne!(first, second);
        assert!(first.as_str().starts_with("ses_"));
        assert!(second.as_str().starts_with("ses_"));
    }
}

fn fork_session_in_db(source_id: &str, fork_id: &str, description: &str) -> std::io::Result<bool> {
    let mut store = match jfc_knowledge::KnowledgeStore::open_default() {
        Ok(store) => store,
        Err(_) => return Ok(false),
    };
    let Some(mut row) = store.get_session(source_id).map_err(io_other)? else {
        return Ok(false);
    };
    let transcript = store.load_transcript(source_id).map_err(io_other)?;
    row.id = fork_id.to_owned();
    row.updated_at = Some(chrono::Utc::now().to_rfc3339());
    if !description.is_empty() {
        row.title = Some(format!("[fork] {description}"));
    }
    store
        .replace_transcript(&row, &transcript)
        .map_err(io_other)?;
    Ok(true)
}

pub async fn delete_session(session_id: &str) -> std::io::Result<bool> {
    delete_session_from_db(session_id)
}

fn delete_session_from_db(session_id: &str) -> std::io::Result<bool> {
    let mut store = match jfc_knowledge::KnowledgeStore::open_default() {
        Ok(store) => store,
        Err(_) => return Ok(false),
    };
    let deleted = store.delete_session(session_id).map_err(io_other)?;
    Ok(deleted > 0)
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

pub async fn fsck_sessions(_quarantine: bool) -> std::io::Result<SessionFsckReport> {
    tokio::task::spawn_blocking(move || {
        let store = match jfc_knowledge::KnowledgeStore::open_default() {
            Ok(store) => store,
            Err(_) => return Ok(SessionFsckReport::default()),
        };
        let mut report = SessionFsckReport::default();
        for row in store.list_sessions(None, 100_000).map_err(io_other)? {
            report.checked += 1;
            let transcript = store.load_transcript(&row.id).map_err(io_other)?;
            if transcript.is_empty() && row.message_count > 0 {
                report.issues.push(SessionFsckIssue {
                    path: PathBuf::from(format!("db/{}", row.id)),
                    reason: "session row has no transcript messages".to_owned(),
                    quarantined_to: None,
                });
                continue;
            }
            if row.message_count >= 0 && row.message_count as usize != transcript.len() {
                report.issues.push(SessionFsckIssue {
                    path: PathBuf::from(format!("db/{}", row.id)),
                    reason: format!(
                        "message_count {} does not match transcript length {}",
                        row.message_count,
                        transcript.len()
                    ),
                    quarantined_to: None,
                });
                continue;
            }
            report.ok += 1;
        }
        Ok(report)
    })
    .await
    .unwrap_or_else(|err| Err(io_other(err)))
}
