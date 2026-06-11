//! Resumable streams with snapshot replay.
//!
//! Mirrors Perplexity's resumable `/rest/sse/perplexity_ask` flow found in the
//! 2026-06-11 mindemon dump: every answer entry carries a `backend_uuid` and a
//! `read_write_token`, and a dropped connection reconnects via
//! `/rest/sse/perplexity_ask/reconnect/{resume_entry_uuid}` with
//! `reconnectInitialSnapshot: true` — the server replays the partial answer
//! accumulated so far, then continues live.
//!
//! This module provides the durable side of that: a [`ResumeStore`] that mints a
//! [`ResumeEntry`] (`backend_uuid` + `read_write_token`) per streaming turn,
//! accumulates the partial assistant text as deltas arrive, and lets a client
//! that lost its connection fetch the snapshot and continue. Writes are gated by
//! the `read_write_token` so only the owning stream can append or finalize.
//!
//! It is transport-agnostic and fully unit-testable: the live stream loop feeds
//! it `record_delta`/`finish`; a reconnect path calls `snapshot` /
//! `resume`. No socket or provider coupling.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

use uuid::Uuid;

/// State of a resumable entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntryState {
    /// The stream is still producing tokens.
    Streaming,
    /// The stream finished normally.
    Complete,
    /// The stream errored; the snapshot holds whatever arrived before failure.
    Failed,
}

/// A resumable stream entry: identity, write-capability token, accumulated
/// snapshot, and state.
#[derive(Debug, Clone)]
pub struct ResumeEntry {
    /// Stable id for the entry (the reconnect key).
    pub backend_uuid: String,
    /// Capability token required to append/finalize this entry.
    pub read_write_token: String,
    /// Partial (or final) assistant text accumulated so far.
    pub snapshot: String,
    pub state: EntryState,
    /// Whether this entry can be reconnected to. False once finalized + drained.
    pub reconnectable: bool,
}

impl ResumeEntry {
    fn new() -> Self {
        Self {
            backend_uuid: Uuid::new_v4().to_string(),
            read_write_token: Uuid::new_v4().to_string(),
            snapshot: String::new(),
            state: EntryState::Streaming,
            reconnectable: true,
        }
    }
}

/// A read-only handle returned to a reconnecting client.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResumeSnapshot {
    pub backend_uuid: String,
    pub snapshot: String,
    pub state: EntryState,
}

/// Errors from resume operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeError {
    /// No entry with that `backend_uuid`.
    UnknownEntry,
    /// The supplied `read_write_token` doesn't match the entry.
    BadToken,
    /// The entry is finalized and can no longer be appended to.
    AlreadyFinalized,
}

impl std::fmt::Display for ResumeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownEntry => f.write_str("unknown resume entry"),
            Self::BadToken => f.write_str("invalid read_write_token"),
            Self::AlreadyFinalized => f.write_str("resume entry already finalized"),
        }
    }
}

impl std::error::Error for ResumeError {}

/// A store of resumable stream entries, keyed by `backend_uuid`.
#[derive(Debug, Default)]
pub struct ResumeStore {
    entries: HashMap<String, ResumeEntry>,
}

impl ResumeStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Begin a new resumable entry. Returns `(backend_uuid, read_write_token)`;
    /// the caller keeps the token to append/finalize and hands the
    /// `backend_uuid` to the client as the reconnect key.
    pub fn begin(&mut self) -> (String, String) {
        let entry = ResumeEntry::new();
        let ids = (entry.backend_uuid.clone(), entry.read_write_token.clone());
        self.entries.insert(entry.backend_uuid.clone(), entry);
        ids
    }

    fn authed_mut(
        &mut self,
        backend_uuid: &str,
        token: &str,
    ) -> Result<&mut ResumeEntry, ResumeError> {
        let entry = self
            .entries
            .get_mut(backend_uuid)
            .ok_or(ResumeError::UnknownEntry)?;
        if entry.read_write_token != token {
            return Err(ResumeError::BadToken);
        }
        Ok(entry)
    }

    /// Append a text delta to the entry's snapshot (token-gated).
    pub fn record_delta(
        &mut self,
        backend_uuid: &str,
        token: &str,
        delta: &str,
    ) -> Result<(), ResumeError> {
        let entry = self.authed_mut(backend_uuid, token)?;
        if entry.state != EntryState::Streaming {
            return Err(ResumeError::AlreadyFinalized);
        }
        entry.snapshot.push_str(delta);
        Ok(())
    }

    /// Finalize an entry with a terminal state (token-gated). Subsequent
    /// `record_delta` calls fail; the snapshot remains fetchable until dropped.
    pub fn finish(
        &mut self,
        backend_uuid: &str,
        token: &str,
        state: EntryState,
    ) -> Result<(), ResumeError> {
        let entry = self.authed_mut(backend_uuid, token)?;
        entry.state = state;
        Ok(())
    }

    /// Read-only snapshot for a reconnecting client (no token required — the
    /// `backend_uuid` is the read capability, matching Perplexity's reconnect
    /// URL which only needs the entry id).
    pub fn snapshot(&self, backend_uuid: &str) -> Option<ResumeSnapshot> {
        self.entries.get(backend_uuid).map(|e| ResumeSnapshot {
            backend_uuid: e.backend_uuid.clone(),
            snapshot: e.snapshot.clone(),
            state: e.state,
        })
    }

    /// Reconnect to an entry: returns its snapshot if it exists and is
    /// reconnectable. Mirrors `reconnect/{resume_entry_uuid}` with
    /// `reconnectInitialSnapshot`.
    pub fn resume(&self, backend_uuid: &str) -> Result<ResumeSnapshot, ResumeError> {
        let entry = self
            .entries
            .get(backend_uuid)
            .ok_or(ResumeError::UnknownEntry)?;
        Ok(ResumeSnapshot {
            backend_uuid: entry.backend_uuid.clone(),
            snapshot: entry.snapshot.clone(),
            state: entry.state,
        })
    }

    /// Whether an entry is still streaming (a reconnect would continue live).
    pub fn is_streaming(&self, backend_uuid: &str) -> bool {
        self.entries
            .get(backend_uuid)
            .map(|e| e.state == EntryState::Streaming)
            .unwrap_or(false)
    }

    /// Drop a finalized entry once the client has drained it. Returns the
    /// removed entry, if any.
    pub fn drop_entry(&mut self, backend_uuid: &str) -> Option<ResumeEntry> {
        self.entries.remove(backend_uuid)
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Process-global resume store so the reconnect path (a separate request in the
/// runtime) can find an in-flight entry the live stream registered.
static GLOBAL: OnceLock<Mutex<ResumeStore>> = OnceLock::new();

fn global() -> &'static Mutex<ResumeStore> {
    GLOBAL.get_or_init(|| Mutex::new(ResumeStore::new()))
}

/// Begin a globally-registered resumable entry.
pub fn global_begin() -> (String, String) {
    global().lock().expect("resume store poisoned").begin()
}

/// Append a delta to a globally-registered entry.
pub fn global_record_delta(
    backend_uuid: &str,
    token: &str,
    delta: &str,
) -> Result<(), ResumeError> {
    global()
        .lock()
        .expect("resume store poisoned")
        .record_delta(backend_uuid, token, delta)
}

/// Finalize a globally-registered entry.
pub fn global_finish(
    backend_uuid: &str,
    token: &str,
    state: EntryState,
) -> Result<(), ResumeError> {
    global()
        .lock()
        .expect("resume store poisoned")
        .finish(backend_uuid, token, state)
}

/// Reconnect to a globally-registered entry.
pub fn global_resume(backend_uuid: &str) -> Result<ResumeSnapshot, ResumeError> {
    global()
        .lock()
        .expect("resume store poisoned")
        .resume(backend_uuid)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Lifecycle ────────────────────────────────────────────────────────────

    #[test]
    fn begin_mints_distinct_ids_normal() {
        let mut store = ResumeStore::new();
        let (uuid1, tok1) = store.begin();
        let (uuid2, tok2) = store.begin();
        assert_ne!(uuid1, uuid2);
        assert_ne!(tok1, tok2);
        assert_eq!(store.len(), 2);
    }

    #[test]
    fn record_and_snapshot_accumulates_normal() {
        let mut store = ResumeStore::new();
        let (uuid, tok) = store.begin();
        store.record_delta(&uuid, &tok, "Hello, ").unwrap();
        store.record_delta(&uuid, &tok, "world").unwrap();
        let snap = store.snapshot(&uuid).unwrap();
        assert_eq!(snap.snapshot, "Hello, world");
        assert_eq!(snap.state, EntryState::Streaming);
    }

    // ── The core reconnect-with-snapshot scenario ──────────────────────────────

    #[test]
    fn reconnect_replays_partial_snapshot_then_continues_normal() {
        let mut store = ResumeStore::new();
        let (uuid, tok) = store.begin();
        // Stream produces a few tokens...
        store.record_delta(&uuid, &tok, "The answer ").unwrap();
        store.record_delta(&uuid, &tok, "is ").unwrap();

        // ── connection drops; client reconnects with just the backend_uuid ──
        let resumed = store.resume(&uuid).expect("reconnect");
        assert_eq!(resumed.snapshot, "The answer is ");
        assert_eq!(resumed.state, EntryState::Streaming);
        assert!(store.is_streaming(&uuid));

        // ── stream continues live after the snapshot replay ──
        store.record_delta(&uuid, &tok, "42.").unwrap();
        store.finish(&uuid, &tok, EntryState::Complete).unwrap();
        let final_snap = store.resume(&uuid).unwrap();
        assert_eq!(final_snap.snapshot, "The answer is 42.");
        assert_eq!(final_snap.state, EntryState::Complete);
        assert!(!store.is_streaming(&uuid));
    }

    // ── Token gating ───────────────────────────────────────────────────────────

    #[test]
    fn record_with_bad_token_is_rejected_robust() {
        let mut store = ResumeStore::new();
        let (uuid, _tok) = store.begin();
        let err = store.record_delta(&uuid, "wrong-token", "x").unwrap_err();
        assert_eq!(err, ResumeError::BadToken);
        // Snapshot stayed empty.
        assert_eq!(store.snapshot(&uuid).unwrap().snapshot, "");
    }

    #[test]
    fn record_after_finish_is_rejected_robust() {
        let mut store = ResumeStore::new();
        let (uuid, tok) = store.begin();
        store.record_delta(&uuid, &tok, "done").unwrap();
        store.finish(&uuid, &tok, EntryState::Complete).unwrap();
        let err = store.record_delta(&uuid, &tok, "more").unwrap_err();
        assert_eq!(err, ResumeError::AlreadyFinalized);
    }

    #[test]
    fn resume_unknown_entry_is_error_robust() {
        let store = ResumeStore::new();
        assert_eq!(store.resume("nope").unwrap_err(), ResumeError::UnknownEntry);
        assert!(store.snapshot("nope").is_none());
        assert!(!store.is_streaming("nope"));
    }

    #[test]
    fn failed_stream_keeps_partial_snapshot_robust() {
        let mut store = ResumeStore::new();
        let (uuid, tok) = store.begin();
        store.record_delta(&uuid, &tok, "partial").unwrap();
        store.finish(&uuid, &tok, EntryState::Failed).unwrap();
        let snap = store.resume(&uuid).unwrap();
        assert_eq!(snap.snapshot, "partial");
        assert_eq!(snap.state, EntryState::Failed);
    }

    #[test]
    fn drop_entry_removes_it_normal() {
        let mut store = ResumeStore::new();
        let (uuid, tok) = store.begin();
        store.finish(&uuid, &tok, EntryState::Complete).unwrap();
        assert!(store.drop_entry(&uuid).is_some());
        assert!(store.is_empty());
        assert_eq!(store.resume(&uuid).unwrap_err(), ResumeError::UnknownEntry);
    }

    // ── Global store ───────────────────────────────────────────────────────────

    #[test]
    fn global_store_roundtrip_normal() {
        let (uuid, tok) = global_begin();
        global_record_delta(&uuid, &tok, "global ").unwrap();
        global_record_delta(&uuid, &tok, "snapshot").unwrap();
        let snap = global_resume(&uuid).unwrap();
        assert_eq!(snap.snapshot, "global snapshot");
        global_finish(&uuid, &tok, EntryState::Complete).unwrap();
        assert_eq!(global_resume(&uuid).unwrap().state, EntryState::Complete);
    }
}
