//! Unified, queryable runtime audit ledger.
//!
//! A single append-only event store recording what agents actually did — tool
//! calls, approvals, provider/model calls, file writes, cancellations, costs,
//! and failures — so the question Dolt's pitch poses ("what did this agent do,
//! when, and why") has one authoritative answer. Each event optionally carries
//! a `change_id`, so the ledger is queryable per change-set.
//!
//! Persistence reuses the same JSONL + flock discipline as [`crate::store`]:
//! one self-describing line per event under `.jfc/audit/runtime.jsonl`. Unlike
//! the change-set store this is **append-only** — events are immutable facts,
//! never rewritten — so writes are a cheap `O(1)` line append, not a full
//! rewrite.

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::error::{ChangeSetError, Result};

/// Classification of a ledger event. Kept coarse so callers don't have to
/// thread a giant enum; specifics live in [`LedgerEvent::detail`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventKind {
    /// A tool was dispatched (Bash/Read/Edit/Write/…).
    ToolCall,
    /// A file was created or modified.
    FileWrite,
    /// A tool/agent action was approved or denied.
    Approval,
    /// A provider/model call was made.
    ProviderCall,
    /// A daemon background job event.
    DaemonJob,
    /// An agent run was cancelled/interrupted.
    Cancellation,
    /// Token/cost usage was recorded.
    Usage,
    /// An error/failure occurred.
    Failure,
}

impl EventKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::ToolCall => "tool_call",
            Self::FileWrite => "file_write",
            Self::Approval => "approval",
            Self::ProviderCall => "provider_call",
            Self::DaemonJob => "daemon_job",
            Self::Cancellation => "cancellation",
            Self::Usage => "usage",
            Self::Failure => "failure",
        }
    }
}

/// A single immutable audit fact.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LedgerEvent {
    /// Unix-epoch millis when the event was recorded.
    pub at_ms: u64,
    pub kind: EventKind,
    /// Short subject — the tool name, model id, file path, etc.
    pub subject: String,
    /// Free-form human-readable detail.
    pub detail: String,
    /// Change-set this event belongs to, if any (links the ledger to a
    /// reviewable proposal).
    pub change_id: Option<String>,
    /// Originating task, if any.
    pub task_id: Option<String>,
    /// Originating session, if any.
    pub session_id: Option<String>,
}

impl LedgerEvent {
    /// Construct a minimal event; chain the `with_*` setters for context.
    pub fn new(at_ms: u64, kind: EventKind, subject: impl Into<String>) -> Self {
        Self {
            at_ms,
            kind,
            subject: subject.into(),
            detail: String::new(),
            change_id: None,
            task_id: None,
            session_id: None,
        }
    }

    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = detail.into();
        self
    }

    pub fn with_change_id(mut self, change_id: Option<String>) -> Self {
        self.change_id = change_id;
        self
    }

    pub fn with_task_id(mut self, task_id: Option<String>) -> Self {
        self.task_id = task_id;
        self
    }

    pub fn with_session_id(mut self, session_id: Option<String>) -> Self {
        self.session_id = session_id;
        self
    }
}

/// Filter for querying ledger events.
#[derive(Debug, Default, Clone)]
pub struct LedgerFilter {
    pub kind: Option<EventKind>,
    pub change_id: Option<String>,
    pub task_id: Option<String>,
    /// Only events at or after this unix-epoch-ms timestamp.
    pub since_ms: Option<u64>,
}

impl LedgerFilter {
    fn matches(&self, e: &LedgerEvent) -> bool {
        if let Some(kind) = self.kind
            && e.kind != kind
        {
            return false;
        }
        if let Some(cid) = &self.change_id
            && e.change_id.as_deref() != Some(cid.as_str())
        {
            return false;
        }
        if let Some(tid) = &self.task_id
            && e.task_id.as_deref() != Some(tid.as_str())
        {
            return false;
        }
        if let Some(since) = self.since_ms
            && e.at_ms < since
        {
            return false;
        }
        true
    }
}

/// Append-only audit ledger persisted at `.jfc/audit/runtime.jsonl`.
pub struct LedgerStore {
    events_path: PathBuf,
    lock_path: PathBuf,
}

impl LedgerStore {
    /// Open (or create) the ledger under a project root.
    pub fn open_project(root: impl AsRef<Path>) -> Result<Self> {
        let dir = root.as_ref().join(".jfc").join("audit");
        fs::create_dir_all(&dir)
            .map_err(|e| ChangeSetError::io(e, format!("creating {}", dir.display())))?;
        let events_path = dir.join("runtime.jsonl");
        let lock_path = dir.join("runtime.lock");
        if !events_path.exists() {
            File::create(&events_path)
                .map_err(|e| ChangeSetError::io(e, "creating runtime.jsonl"))?;
        }
        Ok(Self {
            events_path,
            lock_path,
        })
    }

    /// Append one event under an exclusive lock. Append-only: never rewrites
    /// existing lines, so a concurrent reader sees a consistent prefix.
    pub fn append(&self, event: &LedgerEvent) -> Result<()> {
        let lock = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&self.lock_path)
            .map_err(|e| ChangeSetError::io(e, "opening ledger lock"))?;
        lock.lock_exclusive()
            .map_err(|e| ChangeSetError::io(e, "acquiring ledger lock"))?;

        let result = self.append_locked(event);

        if let Err(e) = FileExt::unlock(&lock) {
            warn!(error = %e, "failed to release ledger lock (will release on drop)");
        }
        result
    }

    fn append_locked(&self, event: &LedgerEvent) -> Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.events_path)
            .map_err(|e| ChangeSetError::io(e, "opening runtime.jsonl for append"))?;
        let json =
            serde_json::to_string(event).map_err(|e| ChangeSetError::serde(e, "encoding event"))?;
        writeln!(file, "{json}").map_err(|e| ChangeSetError::io(e, "writing event line"))?;
        Ok(())
    }

    /// Read all events matching `filter`, oldest first. Corrupt lines are
    /// skipped with a warning rather than aborting the read.
    pub fn query(&self, filter: &LedgerFilter) -> Result<Vec<LedgerEvent>> {
        let file = File::open(&self.events_path)
            .map_err(|e| ChangeSetError::io(e, "opening runtime.jsonl for query"))?;
        let mut out = Vec::new();
        for (n, line) in BufReader::new(file).lines().enumerate() {
            let line = line.map_err(|e| ChangeSetError::io(e, format!("reading line {n}")))?;
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            match serde_json::from_str::<LedgerEvent>(trimmed) {
                Ok(ev) if filter.matches(&ev) => out.push(ev),
                Ok(_) => {}
                Err(e) => warn!(line = n, error = %e, "skipping corrupt ledger line"),
            }
        }
        debug!(count = out.len(), "queried ledger events");
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn ev(at: u64, kind: EventKind, subject: &str) -> LedgerEvent {
        LedgerEvent::new(at, kind, subject)
    }

    // Normal: append then query round-trips events oldest-first.
    #[test]
    fn append_then_query_round_trips_normal() {
        let dir = TempDir::new().unwrap();
        let store = LedgerStore::open_project(dir.path()).unwrap();
        store.append(&ev(100, EventKind::ToolCall, "Bash")).unwrap();
        store
            .append(&ev(200, EventKind::FileWrite, "src/lib.rs"))
            .unwrap();

        let all = store.query(&LedgerFilter::default()).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].at_ms, 100, "oldest first");
        assert_eq!(all[1].subject, "src/lib.rs");
    }

    // Robust: filtering by change_id returns only that change's events — the
    // per-change queryability the audit ledger is for.
    #[test]
    fn query_filters_by_change_id_robust() {
        let dir = TempDir::new().unwrap();
        let store = LedgerStore::open_project(dir.path()).unwrap();
        store
            .append(&ev(100, EventKind::ToolCall, "Edit").with_change_id(Some("cs-a".into())))
            .unwrap();
        store
            .append(&ev(110, EventKind::ToolCall, "Bash").with_change_id(Some("cs-b".into())))
            .unwrap();

        let a = store
            .query(&LedgerFilter {
                change_id: Some("cs-a".into()),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].subject, "Edit");
    }

    // Robust: kind + since filters compose.
    #[test]
    fn query_filters_by_kind_and_since_robust() {
        let dir = TempDir::new().unwrap();
        let store = LedgerStore::open_project(dir.path()).unwrap();
        store.append(&ev(100, EventKind::ToolCall, "old")).unwrap();
        store.append(&ev(300, EventKind::ToolCall, "new")).unwrap();
        store.append(&ev(300, EventKind::Failure, "boom")).unwrap();

        let recent_tools = store
            .query(&LedgerFilter {
                kind: Some(EventKind::ToolCall),
                since_ms: Some(200),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(recent_tools.len(), 1);
        assert_eq!(recent_tools[0].subject, "new");
    }

    // Robust: a corrupt line is skipped, not fatal.
    #[test]
    fn corrupt_line_is_skipped_robust() {
        let dir = TempDir::new().unwrap();
        let store = LedgerStore::open_project(dir.path()).unwrap();
        store.append(&ev(100, EventKind::ToolCall, "ok")).unwrap();
        let path = dir.path().join(".jfc").join("audit").join("runtime.jsonl");
        let mut f = OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(f, "{{garbage").unwrap();
        drop(f);

        let all = store.query(&LedgerFilter::default()).unwrap();
        assert_eq!(all.len(), 1, "good event survives corrupt sibling");
    }
}
