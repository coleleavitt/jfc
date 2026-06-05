//! Append-only task history — the durable "everything we've worked on" log.
//!
//! The live task store (`tasks.json`) is *working memory*: bounded, hot, and
//! re-read every UI tick. When terminal tasks age out of that working set
//! (see `TaskStore::prune_terminal_tasks`), they are not simply discarded —
//! each is distilled into a one-line record appended to a sibling
//! `*-history.jsonl` file. That file is *archival memory*: it grows
//! monotonically, is never loaded on the hot path, and is read back only on
//! demand (e.g. `TaskList { include_history: true }`).
//!
//! This mirrors the working-memory / archival-memory split from the agent
//! memory literature (MemGPT, arXiv 2310.08560): keep the in-context set
//! small and retrieve long-term history through an explicit query rather than
//! always-loading it.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::Task;

/// A distilled, immutable record of one completed/failed/deleted task.
///
/// Deliberately small — just enough to answer "have we worked on X, and how
/// did it turn out?" without rehydrating the full task. Unknown fields are
/// ignored on read so the schema can grow without breaking old logs.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskHistoryRecord {
    /// Original task id (e.g. `t42`). Not unique across sessions — history is
    /// an append log, not a keyed store.
    pub id: String,
    pub subject: String,
    /// Terminal status as a lowercase string: `completed` | `failed` | `deleted`.
    pub status: String,
    /// Unix ms when the task was originally created.
    pub created_at_ms: u64,
    /// Unix ms when the task was archived into history (i.e. pruned).
    pub archived_at_ms: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

impl TaskHistoryRecord {
    /// Distill a live task into an archival record stamped at `archived_at_ms`.
    pub fn from_task(task: &Task, archived_at_ms: u64) -> Self {
        let status = serde_json::to_value(task.status)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_else(|| "unknown".to_owned());
        Self {
            id: task.id.as_str().to_owned(),
            subject: task.subject.clone(),
            status,
            created_at_ms: task.created_at_ms,
            archived_at_ms,
            tags: task.tags.clone(),
        }
    }
}

/// Derive the history-log path that sits beside a task-store file.
/// `<dir>/<stem>.json` → `<dir>/<stem>-history.jsonl`. An empty path (the
/// in-memory store) yields an empty path, which all callers treat as a no-op.
pub fn history_path_for(store_path: &Path) -> PathBuf {
    if store_path.as_os_str().is_empty() {
        return PathBuf::new();
    }
    let stem = store_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("tasks");
    let file_name = format!("{stem}-history.jsonl");
    match store_path.parent() {
        Some(parent) => parent.join(file_name),
        None => PathBuf::from(file_name),
    }
}

/// Append distilled records to the JSONL history log (one JSON object per
/// line). Best-effort and never fails the caller: archival is a hygiene step,
/// not a correctness invariant, so I/O errors are logged and swallowed.
pub fn append_records(path: &Path, records: &[TaskHistoryRecord]) {
    if path.as_os_str().is_empty() || records.is_empty() {
        return;
    }
    if let Some(parent) = path.parent()
        && let Err(e) = std::fs::create_dir_all(parent)
    {
        tracing::warn!(
            target: "jfc::tasks",
            error = %e,
            path = %parent.display(),
            "failed to create task history directory"
        );
        return;
    }

    let mut buf = String::with_capacity(records.len() * 96);
    for rec in records {
        match serde_json::to_string(rec) {
            Ok(line) => {
                buf.push_str(&line);
                buf.push('\n');
            }
            Err(e) => {
                tracing::warn!(
                    target: "jfc::tasks",
                    error = %e,
                    id = %rec.id,
                    "failed to serialize task history record"
                );
            }
        }
    }

    match std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        Ok(mut f) => {
            if let Err(e) = f.write_all(buf.as_bytes()) {
                tracing::warn!(
                    target: "jfc::tasks",
                    error = %e,
                    path = %path.display(),
                    "failed to append task history"
                );
            }
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::tasks",
                error = %e,
                path = %path.display(),
                "failed to open task history log"
            );
        }
    }
}

/// Read the most-recent history records, newest first, optionally filtered by
/// a case-insensitive substring match against the subject/id/tags.
///
/// Streams the file line-by-line (the log is append-only and may be large) and
/// keeps only the tail up to `limit`. Malformed lines are skipped so a single
/// corrupt entry can't poison retrieval.
pub fn read_records(path: &Path, limit: usize, query: Option<&str>) -> Vec<TaskHistoryRecord> {
    if path.as_os_str().is_empty() || limit == 0 {
        return Vec::new();
    }
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return Vec::new(), // no history yet — normal
    };
    let needle = query.map(|q| q.to_lowercase());
    let matches = |rec: &TaskHistoryRecord| -> bool {
        let Some(ref n) = needle else {
            return true;
        };
        rec.subject.to_lowercase().contains(n)
            || rec.id.to_lowercase().contains(n)
            || rec.tags.iter().any(|t| t.to_lowercase().contains(n))
    };

    let reader = std::io::BufReader::new(file);
    let mut out: Vec<TaskHistoryRecord> = Vec::new();
    for line in reader.lines().map_while(Result::ok) {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if let Ok(rec) = serde_json::from_str::<TaskHistoryRecord>(trimmed)
            && matches(&rec)
        {
            out.push(rec);
        }
    }
    // Newest first, then cap.
    out.reverse();
    out.truncate(limit);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn rec(id: &str, subject: &str, archived: u64) -> TaskHistoryRecord {
        TaskHistoryRecord {
            id: id.into(),
            subject: subject.into(),
            status: "completed".into(),
            created_at_ms: 1,
            archived_at_ms: archived,
            tags: Vec::new(),
        }
    }

    #[test]
    fn history_path_is_sibling_jsonl_normal() {
        let p = history_path_for(Path::new("/home/u/.jfc/tasks.json"));
        assert_eq!(p, PathBuf::from("/home/u/.jfc/tasks-history.jsonl"));
    }

    #[test]
    fn empty_store_path_yields_empty_history_path_robust() {
        assert_eq!(history_path_for(Path::new("")), PathBuf::new());
    }

    #[test]
    fn append_then_read_newest_first_normal() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("tasks-history.jsonl");
        append_records(&path, &[rec("t1", "first", 100)]);
        append_records(&path, &[rec("t2", "second", 200)]);

        let got = read_records(&path, 10, None);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "t2"); // newest first
        assert_eq!(got[1].id, "t1");
    }

    #[test]
    fn read_applies_limit_and_query_normal() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("h.jsonl");
        append_records(
            &path,
            &[
                rec("t1", "fix auth bug", 1),
                rec("t2", "add caching", 2),
                rec("t3", "fix auth retry", 3),
            ],
        );

        // Query filters to the two "auth" records, newest first.
        let auth = read_records(&path, 10, Some("AUTH"));
        assert_eq!(auth.len(), 2);
        assert_eq!(auth[0].id, "t3");
        assert_eq!(auth[1].id, "t1");

        // Limit caps the tail.
        let one = read_records(&path, 1, None);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, "t3");
    }

    #[test]
    fn malformed_lines_are_skipped_robust() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("h.jsonl");
        std::fs::write(&path, "not json\n{\"broken\":\n").unwrap();
        append_records(&path, &[rec("t9", "ok", 9)]);
        let got = read_records(&path, 10, None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "t9");
    }

    #[test]
    fn read_missing_file_is_empty_robust() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("does-not-exist.jsonl");
        assert!(read_records(&path, 10, None).is_empty());
    }
}
