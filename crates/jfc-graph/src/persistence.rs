//! Event-sourced persistence for the code graph.
//!
//! Graph state is reconstructable from a base snapshot + ordered events.
//! Supports undo via event replay.
//!
//! # On-disk schema versioning
//!
//! Every event written to disk MUST be wrapped in [`VersionedEvent`] and tagged
//! with [`PERSISTENCE_SCHEMA_VERSION`]. Readers verify the tag and reject
//! mismatches with [`PersistenceError::SchemaMismatch`]. The current schema is
//! a clean break: there is no V1 legacy format to read — earlier in-memory
//! event logs were not persisted to disk. When the on-disk format evolves,
//! bump [`PERSISTENCE_SCHEMA_VERSION`] and either add a migration path here or
//! continue treating older versions as a clean break (cache must be deleted).

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId};

/// Current on-disk schema version for [`VersionedEvent`].
///
/// ## Version history
///
/// - **v1** — original wire format.
/// - **v2** — Phase 9: typed metadata (`KindData`) is populated by
///   the rust adapter into `metadata` keys (`param_count`,
///   `field_count`, `variant_count`, `method_count`, `async`, etc.).
///   The wire format itself is **unchanged** — `metadata` was already
///   `HashMap<String, String>` and absorbs the new keys
///   transparently. We bump the version anyway so v1 readers can
///   *opt to* re-index for the typed projections without seeing
///   stale partial-metadata caches.
///
/// Bump when the wire format of [`GraphEvent`], [`EditReason`], or
/// [`EventEntry`] changes incompatibly. Readers reject any other value with
/// [`PersistenceError::SchemaMismatch`] unless [`migrate_event`] knows
/// how to upgrade.
pub const PERSISTENCE_SCHEMA_VERSION: u32 = 2;

/// Errors raised by persistence read/write paths.
#[derive(Debug, Error)]
pub enum PersistenceError {
    /// Encountered an event whose `schema_version` doesn't match
    /// [`PERSISTENCE_SCHEMA_VERSION`]. The on-disk cache must be regenerated.
    #[error(
        "persistence schema mismatch: expected version {expected}, found {found} \
         (delete the on-disk cache and reindex)"
    )]
    SchemaMismatch { expected: u32, found: u32 },
}

/// Best-effort upgrade of an `EventEntry` from `from_version` to
/// [`PERSISTENCE_SCHEMA_VERSION`]. Currently a no-op for v1→v2
/// because metadata is bag-shaped and v1 entries simply lack the
/// new keys; downstream code falls through to `KindData::default()`
/// for missing fields. Returns `Ok` for any handled version,
/// `Err(SchemaMismatch)` for the unhandled future.
pub fn migrate_event(entry: EventEntry, from_version: u32) -> Result<EventEntry, PersistenceError> {
    match from_version {
        PERSISTENCE_SCHEMA_VERSION => Ok(entry),
        // v1 → v2: no migration needed (metadata bag absorbs new keys).
        1 => Ok(entry),
        other => Err(PersistenceError::SchemaMismatch {
            expected: PERSISTENCE_SCHEMA_VERSION,
            found: other,
        }),
    }
}

/// Unique event identifier.
pub type EventId = u64;

/// Timestamp as milliseconds since epoch.
pub type Timestamp = u64;

fn now_ms() -> Timestamp {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Events that modify the graph state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GraphEvent {
    NodeAdded(NodeData),
    NodeRemoved(NodeId),
    EdgeAdded {
        from: NodeId,
        to: NodeId,
        data: EdgeData,
    },
    EdgeRemoved {
        from: NodeId,
        to: NodeId,
        kind: EdgeKind,
    },
    FileReindexed(PathBuf),
}

/// Optional edit reason metadata (for cascade tracking).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditReason {
    pub description: String,
    pub original_context: String,
    pub parent_event_id: Option<EventId>,
    pub cascade_depth: u8,
}

/// A timestamped event entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventEntry {
    pub id: EventId,
    pub timestamp: Timestamp,
    pub event: GraphEvent,
    pub reason: Option<EditReason>,
}

/// Versioned wrapper for a persisted event entry.
///
/// All on-disk persistence MUST funnel through this type. The
/// `schema_version` field is verified on read; mismatches produce
/// [`PersistenceError::SchemaMismatch`]. See module docs for the migration
/// policy.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VersionedEvent {
    pub schema_version: u32,
    pub event: EventEntry,
}

impl VersionedEvent {
    /// Wrap an entry with the current [`PERSISTENCE_SCHEMA_VERSION`] for
    /// serialization.
    pub fn wrap(event: EventEntry) -> Self {
        Self {
            schema_version: PERSISTENCE_SCHEMA_VERSION,
            event,
        }
    }

    /// Unwrap a deserialized entry, verifying the schema tag matches the
    /// running binary's [`PERSISTENCE_SCHEMA_VERSION`]. If the version
    /// is older but a migration is registered (see [`migrate_event`]),
    /// the event is silently upgraded — callers that need to know
    /// whether a migration ran should consult [`Self::needs_migration`]
    /// before unwrapping.
    pub fn unwrap_verified(self) -> Result<EventEntry, PersistenceError> {
        if self.schema_version == PERSISTENCE_SCHEMA_VERSION {
            return Ok(self.event);
        }
        // Try a migration before failing.
        migrate_event(self.event, self.schema_version)
    }

    /// True if the wrapped event was written by an older schema and
    /// would be migrated on read.
    pub fn needs_migration(&self) -> bool {
        self.schema_version != PERSISTENCE_SCHEMA_VERSION
    }
}

/// Append-only event log with snapshot support.
pub struct EventLog {
    entries: Vec<EventEntry>,
    next_id: EventId,
    snapshot_threshold: usize,
}

impl EventLog {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            next_id: 1,
            snapshot_threshold: 100,
        }
    }

    /// Append an event to the log.
    pub fn append(&mut self, event: GraphEvent, reason: Option<EditReason>) -> EventId {
        let id = self.next_id;
        self.next_id += 1;
        self.entries.push(EventEntry {
            id,
            timestamp: now_ms(),
            event,
            reason,
        });
        id
    }

    /// Get all events since a given timestamp.
    pub fn events_since(&self, timestamp: Timestamp) -> &[EventEntry] {
        let start = self.entries.partition_point(|e| e.timestamp < timestamp);
        &self.entries[start..]
    }

    /// Undo the last event (remove from log and return it).
    pub fn undo(&mut self) -> Option<EventEntry> {
        self.entries.pop()
    }

    /// Get total event count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true if the log contains no events.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Check if auto-snapshot threshold reached.
    pub fn should_snapshot(&self) -> bool {
        self.entries.len() >= self.snapshot_threshold
    }

    /// Get event by ID.
    pub fn get_event(&self, id: EventId) -> Option<&EventEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// Trace cascade chain: follow parent_event_id links back to root.
    pub fn trace_cascade(&self, event_id: EventId) -> Vec<&EventEntry> {
        let mut chain = Vec::new();
        let mut current_id = Some(event_id);
        while let Some(id) = current_id {
            if let Some(entry) = self.get_event(id) {
                chain.push(entry);
                current_id = entry.reason.as_ref().and_then(|r| r.parent_event_id);
            } else {
                break;
            }
        }
        chain
    }

    /// Get all entries (for serialization).
    pub fn all_entries(&self) -> &[EventEntry] {
        &self.entries
    }
}

impl Default for EventLog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::edges::EdgeData;
    use crate::nodes::{NodeKind, Span, Visibility};

    fn sample_node_data(name: &str) -> NodeData {
        NodeData {
            id: NodeId::new("src/lib.rs", name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: Span {
                file: PathBuf::from("src/lib.rs"),
                start_line: 1,
                start_col: 0,
                end_line: 5,
                end_col: 1,
                byte_range: 0..50,
            },
            visibility: Visibility::Public,
            metadata: Default::default(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn sample_edge_data() -> EdgeData {
        use crate::edges::EdgeKind;
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: Span {
                file: PathBuf::from("src/lib.rs"),
                start_line: 3,
                start_col: 4,
                end_line: 3,
                end_col: 20,
                byte_range: 30..46,
            },
            weight: 1.0,
        }
    }

    #[test]
    fn test_event_log_append() {
        let mut log = EventLog::new();
        for i in 0..5 {
            let node = sample_node_data(&format!("func_{i}"));
            log.append(GraphEvent::NodeAdded(node), None);
        }
        assert_eq!(log.len(), 5);
        assert!(!log.is_empty());
    }

    #[test]
    fn test_event_log_undo() {
        let mut log = EventLog::new();
        log.append(GraphEvent::NodeAdded(sample_node_data("a")), None);
        log.append(GraphEvent::NodeAdded(sample_node_data("b")), None);
        let id3 = log.append(GraphEvent::NodeAdded(sample_node_data("c")), None);

        let undone = log.undo().expect("should have an entry to undo");
        assert_eq!(undone.id, id3);
        assert_eq!(log.len(), 2);
    }

    #[test]
    fn test_event_log_events_since() {
        let mut log = EventLog::new();

        // Append events — timestamps are monotonically increasing from now_ms()
        log.append(GraphEvent::NodeAdded(sample_node_data("a")), None);
        log.append(GraphEvent::NodeAdded(sample_node_data("b")), None);
        log.append(GraphEvent::NodeAdded(sample_node_data("c")), None);

        // All events should have timestamp >= 0
        let all = log.events_since(0);
        assert_eq!(all.len(), 3);

        // Events since a future timestamp should be empty
        let future = now_ms() + 100_000;
        let none = log.events_since(future);
        assert_eq!(none.len(), 0);

        // Events since the timestamp of the second entry
        let ts = log.all_entries()[1].timestamp;
        let since = log.events_since(ts);
        assert!(since.len() >= 2); // at least entries 2 and 3 (same ms possible)
    }

    #[test]
    fn test_event_log_should_snapshot() {
        let mut log = EventLog::new();
        assert!(!log.should_snapshot());

        for i in 0..100 {
            log.append(
                GraphEvent::FileReindexed(PathBuf::from(format!("file_{i}.rs"))),
                None,
            );
        }
        assert!(log.should_snapshot());
    }

    #[test]
    fn test_event_log_trace_cascade() {
        let mut log = EventLog::new();

        // Root event (no parent)
        let id1 = log.append(
            GraphEvent::NodeAdded(sample_node_data("root")),
            Some(EditReason {
                description: "initial add".into(),
                original_context: "user action".into(),
                parent_event_id: None,
                cascade_depth: 0,
            }),
        );

        // Child of root
        let id2 = log.append(
            GraphEvent::NodeAdded(sample_node_data("child")),
            Some(EditReason {
                description: "cascade from root".into(),
                original_context: "auto".into(),
                parent_event_id: Some(id1),
                cascade_depth: 1,
            }),
        );

        // Grandchild
        let id3 = log.append(
            GraphEvent::EdgeAdded {
                from: NodeId::new("src/lib.rs", "root", NodeKind::Function),
                to: NodeId::new("src/lib.rs", "child", NodeKind::Function),
                data: sample_edge_data(),
            },
            Some(EditReason {
                description: "edge from cascade".into(),
                original_context: "auto".into(),
                parent_event_id: Some(id2),
                cascade_depth: 2,
            }),
        );

        let chain = log.trace_cascade(id3);
        assert_eq!(chain.len(), 3);
        assert_eq!(chain[0].id, id3);
        assert_eq!(chain[1].id, id2);
        assert_eq!(chain[2].id, id1);
    }

    #[test]
    fn test_event_log_get_event() {
        let mut log = EventLog::new();
        let id = log.append(GraphEvent::NodeAdded(sample_node_data("target")), None);

        let entry = log.get_event(id).expect("event should exist");
        assert_eq!(entry.id, id);

        assert!(log.get_event(999).is_none());
    }

    // Normal: a VersionedEvent round-trips through the wrap/unwrap helpers
    // without losing the inner entry.
    #[test]
    fn versioned_event_roundtrip_normal() {
        let entry = EventEntry {
            id: 7,
            timestamp: 1234,
            event: GraphEvent::NodeAdded(sample_node_data("rt")),
            reason: None,
        };
        let wrapped = VersionedEvent::wrap(entry.clone());
        assert_eq!(wrapped.schema_version, PERSISTENCE_SCHEMA_VERSION);
        let unwrapped = wrapped.unwrap_verified().expect("schema matches");
        assert_eq!(unwrapped.id, entry.id);
        assert_eq!(unwrapped.timestamp, entry.timestamp);
    }

    // Robust: an entry tagged with a different schema version is rejected
    // with SchemaMismatch carrying the expected/found pair.
    #[test]
    fn versioned_event_rejects_mismatch_robust() {
        let entry = EventEntry {
            id: 1,
            timestamp: 0,
            event: GraphEvent::NodeAdded(sample_node_data("rt")),
            reason: None,
        };
        let bogus = VersionedEvent {
            schema_version: PERSISTENCE_SCHEMA_VERSION + 1,
            event: entry,
        };
        match bogus.unwrap_verified() {
            Err(PersistenceError::SchemaMismatch { expected, found }) => {
                assert_eq!(expected, PERSISTENCE_SCHEMA_VERSION);
                assert_eq!(found, PERSISTENCE_SCHEMA_VERSION + 1);
            }
            Ok(_) => panic!("expected schema mismatch error"),
        }
    }

    #[test]
    fn v1_event_migrates_to_v2_silently() {
        // Phase 9: schema bump v1 → v2 is non-breaking. A v1 entry
        // unwraps cleanly under v2.
        let entry = EventEntry {
            id: 1,
            timestamp: 0,
            event: GraphEvent::NodeAdded(sample_node_data("legacy")),
            reason: None,
        };
        let v1_wrapped = VersionedEvent {
            schema_version: 1,
            event: entry,
        };
        assert!(v1_wrapped.needs_migration());
        let migrated = v1_wrapped.unwrap_verified();
        assert!(migrated.is_ok(), "v1 → v2 migration must succeed");
    }

    #[test]
    fn migrate_event_handles_known_versions() {
        let entry = EventEntry {
            id: 1,
            timestamp: 0,
            event: GraphEvent::NodeAdded(sample_node_data("x")),
            reason: None,
        };
        // Current version is a no-op.
        assert!(migrate_event(entry.clone(), PERSISTENCE_SCHEMA_VERSION).is_ok());
        // v1 is supported.
        assert!(migrate_event(entry.clone(), 1).is_ok());
        // Future versions fail.
        assert!(migrate_event(entry, 999).is_err());
    }
}
