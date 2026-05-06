//! Event-sourced persistence for the code graph.
//!
//! Graph state is reconstructable from a base snapshot + ordered events.
//! Supports undo via event replay.

use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId};

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
}
