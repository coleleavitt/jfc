//! Session catalog and path helpers.
//!
//! Full transcript serialization still lives in `jfc` while message/tool
//! types are being untangled. This crate owns the provider-neutral session
//! index surface: paths, IDs, metadata listing, and picker helpers.

use std::path::PathBuf;

use jfc_core::SessionId;
use tracing::debug;

mod catalog;
mod git_commits;
mod search;
mod soft_match;
mod task_history;
mod task_store;

pub use catalog::{
    SessionMetadata, cwd_mismatch_message, format_session_id_timestamp, group_by_cwd,
    list_session_ids_only, list_sessions, list_sessions_filtered, list_sessions_with_metadata,
    load_session_metadata, most_recent_session, most_recent_session_for_cwd, relative_time,
    shorten_cwd,
};
pub use git_commits::{CommitHit, search as search_commits};
pub use search::{
    SessionBrief, SessionHit, SessionMessage, browse as browse_sessions,
    discover as search_sessions, discover_excluding as search_sessions_excluding,
    scroll as scroll_session,
};
pub use task_history::{TaskHistoryRecord, history_path_for, read_records as read_task_history};
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

pub fn generate_session_id() -> SessionId {
    let now = chrono::Utc::now();
    let id = SessionId::new(format!("ses_{}", now.format("%Y%m%d_%H%M%S")));
    debug!(target: "jfc::session", %id, "generated session id");
    id
}

/// Remove sessions older than `max_age_days`. Keep at least `min_keep`
/// most-recent sessions regardless of age. Returns the number of files
/// deleted. Passes gracefully when the sessions directory does not exist yet.
pub async fn gc_old_sessions(max_age_days: u64, min_keep: usize) -> std::io::Result<usize> {
    if max_age_days == 0 {
        return Ok(0);
    }
    let entries = collect_session_mtimes().await;
    if entries.is_empty() {
        return Ok(0);
    }
    // Sort newest-first; entries[0..min_keep] are exempt from deletion.
    let mut sorted = entries;
    sorted.sort_by(|a, b| b.0.cmp(&a.0));
    let cutoff = std::time::SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(max_age_days * 86400))
        .unwrap_or(std::time::UNIX_EPOCH);

    let mut deleted = 0usize;
    for (i, (mtime, path)) in sorted.iter().enumerate() {
        if i < min_keep || *mtime >= cutoff {
            continue;
        }
        if tokio::fs::remove_file(path).await.is_ok() {
            deleted += 1;
            debug!(
                target: "jfc::session::gc",
                path = %path.display(),
                "gc_old_sessions: removed stale session"
            );
        }
    }
    Ok(deleted)
}

/// Collect `(mtime, path)` pairs for every `*.json` file in the sessions dir.
async fn collect_session_mtimes() -> Vec<(std::time::SystemTime, std::path::PathBuf)> {
    let mut out = Vec::new();
    let mut rd = match tokio::fs::read_dir(sessions_dir()).await {
        Ok(rd) => rd,
        Err(_) => return out,
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("json") {
            continue;
        }
        let Ok(meta) = entry.metadata().await else { continue };
        let Ok(modified) = meta.modified() else { continue };
        out.push((modified, path));
    }
    out
}

/// Fork an existing session into a new parallel branch.
///
/// Copies the source session's JSON file to a fresh session ID, then patches
/// the embedded `id` field so the new file is self-consistent. The source
/// session is left untouched on disk.
///
/// Returns the new session ID, or an `io::Error` if the source session does
/// not exist or cannot be copied.
pub async fn fork_session(source_id: &str, description: &str) -> std::io::Result<String> {
    let dir = sessions_dir();
    let src_path = dir.join(format!("{source_id}.json"));
    let content = tokio::fs::read_to_string(&src_path).await?;

    // Parse and patch the `id` field so the fork is self-consistent.
    let mut value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    let fork_id = generate_session_id();
    let fork_id_str = fork_id.as_str().to_owned();

    if let Some(obj) = value.as_object_mut() {
        obj.insert("id".to_owned(), serde_json::Value::String(fork_id_str.clone()));
        // Record when the fork was created.
        let now = chrono::Utc::now().to_rfc3339();
        obj.insert("updated_at".to_owned(), serde_json::Value::String(now.clone()));
        // If a fork description is provided, stash it in the title field so
        // the session picker can distinguish the fork from the original.
        if !description.is_empty() {
            obj.insert(
                "title".to_owned(),
                serde_json::Value::String(format!("[fork] {description}")),
            );
        }
    }

    let patched = serde_json::to_string_pretty(&value)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

    tokio::fs::create_dir_all(&dir).await?;
    let dst_path = dir.join(format!("{fork_id_str}.json"));
    tokio::fs::write(&dst_path, patched).await?;

    debug!(
        target: "jfc::session",
        source_id,
        fork_id = %fork_id_str,
        description,
        "fork_session: created fork"
    );

    Ok(fork_id_str)
}

/// Delete a session file from the sessions directory.
/// Returns `true` if the file was found and removed, `false` if it didn't exist.
pub async fn delete_session(session_id: &str) -> std::io::Result<bool> {
    let path = sessions_dir().join(format!("{session_id}.json"));
    match tokio::fs::remove_file(&path).await {
        Ok(()) => {
            debug!(target: "jfc::session", session_id, "deleted session file");
            Ok(true)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}
