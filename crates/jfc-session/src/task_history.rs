//! DB-backed append-only task history — the durable "everything we've worked on" log.
//!
//! The live task store is *working memory*: bounded and hot. When terminal tasks
//! age out of that working set (see `TaskStore::prune_terminal_tasks`), they are
//! not discarded. Each is distilled into a small DB artifact event keyed by the
//! session id or shared task-store identity. That DB event stream is *archival
//! memory*: it grows monotonically, stays off the hot path, and is read back only
//! on demand (e.g. `TaskList { include_history: true }`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Task;

const TASK_HISTORY_SESSION_ID: &str = "__task_history__";
const TASK_HISTORY_KIND: &str = "task_history";

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

/// Stable DB key for a session-scoped task history stream.
pub fn session_history_key(session_id: &str) -> Option<String> {
    let trimmed = session_id.trim();
    (!trimmed.is_empty()).then(|| format!("session:{trimmed}"))
}

/// Stable DB key for a shared project/team task store.
pub fn history_key_for_store_path(store_path: &Path) -> Option<String> {
    if store_path.as_os_str().is_empty() {
        return None;
    }
    Some(format!("store-path:{}", store_path.display()))
}

/// Append distilled records to the DB history stream.
///
/// Best-effort and never fails the caller: archival is a hygiene step, not a
/// correctness invariant, so DB errors are logged and swallowed.
pub fn append_records(history_key: Option<&str>, records: &[TaskHistoryRecord]) {
    let Some(history_key) = history_key.filter(|key| !key.trim().is_empty()) else {
        return;
    };
    if records.is_empty() {
        return;
    }

    let result = jfc_knowledge::block_on_knowledge(async {
        let store = match crate::open_default_knowledge_store().await {
            Ok(store) => store,
            Err(error) => {
                tracing::warn!(
                    target: "jfc::tasks",
                    %error,
                    "failed to open DB task history store"
                );
                return Ok::<_, jfc_knowledge::KnowledgeError>(());
            }
        };

        for rec in records {
            let json = match serde_json::to_string(rec) {
                Ok(json) => json,
                Err(error) => {
                    tracing::warn!(
                        target: "jfc::tasks",
                        %error,
                        id = %rec.id,
                        "failed to serialize task history record"
                    );
                    continue;
                }
            };
            if let Err(error) = store
                .append_session_artifact_event(
                    TASK_HISTORY_SESSION_ID,
                    TASK_HISTORY_KIND,
                    history_key,
                    &json,
                )
                .await
            {
                tracing::warn!(
                    target: "jfc::tasks",
                    %error,
                    key = history_key,
                    "failed to append DB task history record"
                );
            }
        }
        Ok(())
    });

    if let Err(error) = result {
        tracing::debug!(
            target: "jfc::tasks",
            %error,
            "task history append bridge error"
        );
    }
}

/// Read the most-recent history records, newest first, optionally filtered by
/// a case-insensitive substring match against the subject/id/tags.
pub fn read_records(
    history_key: Option<&str>,
    limit: usize,
    query: Option<&str>,
) -> Vec<TaskHistoryRecord> {
    let Some(history_key) = history_key.filter(|key| !key.trim().is_empty()) else {
        return Vec::new();
    };
    if limit == 0 {
        return Vec::new();
    }

    let rows = jfc_knowledge::block_on_knowledge(async {
        let store = match crate::open_default_knowledge_store().await {
            Ok(store) => store,
            Err(_) => return Vec::new(),
        };

        store
            .list_recent_session_artifact_events(
                TASK_HISTORY_SESSION_ID,
                TASK_HISTORY_KIND,
                Some(history_key),
                limit.saturating_mul(10).clamp(100, 10_000),
            )
            .await
            .unwrap_or_default()
    });

    let needle = query.map(|q| q.to_lowercase());
    let matches = |rec: &TaskHistoryRecord| -> bool {
        let Some(ref n) = needle else {
            return true;
        };
        rec.subject.to_lowercase().contains(n)
            || rec.id.to_lowercase().contains(n)
            || rec.tags.iter().any(|t| t.to_lowercase().contains(n))
    };

    let mut out: Vec<TaskHistoryRecord> = Vec::new();
    for row in rows.into_iter().rev() {
        if let Ok(rec) = serde_json::from_str::<TaskHistoryRecord>(&row.value_json)
            && matches(&rec)
        {
            out.push(rec);
        }
    }
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
    fn store_path_maps_to_db_key_normal() {
        let key = history_key_for_store_path(Path::new("/home/u/.jfc/tasks.json"));
        assert_eq!(key.as_deref(), Some("store-path:/home/u/.jfc/tasks.json"));
    }

    #[test]
    fn empty_store_path_yields_no_history_key_robust() {
        assert_eq!(history_key_for_store_path(Path::new("")), None);
    }

    #[test]
    fn append_then_read_newest_first_normal() {
        let tmp = TempDir::new().unwrap();
        let _guard = test_db(tmp.path());
        let key = Some("test-history:newest");
        append_records(key, &[rec("t1", "first", 100)]);
        append_records(key, &[rec("t2", "second", 200)]);

        let got = read_records(key, 10, None);
        assert_eq!(got.len(), 2);
        assert_eq!(got[0].id, "t2");
        assert_eq!(got[1].id, "t1");
    }

    #[test]
    fn read_applies_limit_and_query_normal() {
        let tmp = TempDir::new().unwrap();
        let _guard = test_db(tmp.path());
        let key = Some("test-history:query");
        append_records(
            key,
            &[
                rec("t1", "fix auth bug", 1),
                rec("t2", "add caching", 2),
                rec("t3", "fix auth retry", 3),
            ],
        );

        let auth = read_records(key, 10, Some("AUTH"));
        assert_eq!(auth.len(), 2);
        assert_eq!(auth[0].id, "t3");
        assert_eq!(auth[1].id, "t1");

        let one = read_records(key, 1, None);
        assert_eq!(one.len(), 1);
        assert_eq!(one[0].id, "t3");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn malformed_rows_are_skipped_robust() {
        let tmp = TempDir::new().unwrap();
        let _guard = test_db(tmp.path());
        let key = Some("test-history:malformed");
        let store = crate::open_default_knowledge_store().await.unwrap();
        store
            .append_session_artifact_event(
                TASK_HISTORY_SESSION_ID,
                TASK_HISTORY_KIND,
                key.unwrap(),
                "not json",
            )
            .await
            .unwrap();
        append_records(key, &[rec("t9", "ok", 9)]);
        let got = read_records(key, 10, None);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].id, "t9");
    }

    #[test]
    fn read_missing_key_is_empty_robust() {
        let tmp = TempDir::new().unwrap();
        let _guard = test_db(tmp.path());
        assert!(read_records(Some("missing"), 10, None).is_empty());
    }

    fn test_db(root: &Path) -> EnvGuard {
        let guard = crate::TEST_ENV_LOCK
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let prior = std::env::var("JFC_KNOWLEDGE_DB").ok();
        unsafe {
            std::env::set_var("JFC_KNOWLEDGE_DB", root.join("knowledge.db"));
        }
        EnvGuard {
            prior,
            _guard: guard,
        }
    }

    struct EnvGuard {
        prior: Option<String>,
        _guard: std::sync::MutexGuard<'static, ()>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prior.take() {
                    Some(prior) => std::env::set_var("JFC_KNOWLEDGE_DB", prior),
                    None => std::env::remove_var("JFC_KNOWLEDGE_DB"),
                }
            }
        }
    }
}
