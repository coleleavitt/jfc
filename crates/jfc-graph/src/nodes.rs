//! Graph node types and their properties.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;

// Phase 9: typed-metadata projection helper.
//
// `NodeData::kind_data()` lives in `crate::kind_specific` because it
// imports the marker types from there. We re-export the constructor
// here so callers can use `node.kind_data()` directly.

/// Exactly 5 node kinds — no more, no less.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum NodeKind {
    Function,
    Struct,
    Enum,
    Module,
    Trait,
}

/// Visibility of a code item.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Visibility {
    Public,
    Crate,
    Super,
    Private,
}

/// Source location span.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Span {
    pub file: PathBuf,
    pub start_line: u32,
    pub start_col: u32,
    pub end_line: u32,
    pub end_col: u32,
    pub byte_range: Range<usize>,
}

/// Deterministic node identifier: hash(file_path + ":" + qualified_name + ":" + kind)
///
/// Implements `Ord` so callers (notably [`crate::fingerprint`]) can sort
/// containers of `NodeId` into a canonical order before hashing — this is
/// what makes graph fingerprints iteration-order-independent.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u64);

impl NodeId {
    pub fn new(file_path: &str, qualified_name: &str, kind: NodeKind) -> Self {
        let mut hasher = DefaultHasher::new();
        file_path.hash(&mut hasher);
        ":".hash(&mut hasher);
        qualified_name.hash(&mut hasher);
        ":".hash(&mut hasher);
        kind.hash(&mut hasher);
        Self(hasher.finish())
    }
}

/// Full node data stored in the graph.
///
/// ## Well-known metadata keys
///
/// The `metadata` map is free-form, but the following keys have defined
/// semantics and are populated by specific passes:
///
/// | Key | Populated by | Type | Description |
/// |-----|-------------|------|-------------|
/// | `fields` | `RustAdapter` | JSON array | Struct field names |
/// | `variants` | `RustAdapter` | JSON array | Enum variant names |
/// | `async` | `RustAdapter` | `"true"/"false"` | Whether function is async |
/// | `accessed_fields` | `RustAdapter` | JSON array | Fields accessed in fn body |
/// | `coverage_count` | [`CoveragePass`] | numeric string | Total LCOV hit count across fn span |
/// | `coverage_tested` | [`CoveragePass`] | `"true"/"false"` | Whether coverage_count > 0 |
/// | `possible_input_types` | [`PossibleTypesPass`] | JSON string array | Types that can flow into fn |
/// | `possible_return_types` | [`PossibleTypesPass`] | JSON string array | Types fn can produce |
///
/// [`CoveragePass`]: crate::coverage::CoveragePass
/// [`PossibleTypesPass`]: crate::possible_types::PossibleTypesPass
///
/// ## Revision tracking
///
/// `birth_revision` and `last_modified_revision` give every node a coarse
/// timeline relative to the owning [`crate::graph::CodeGraph::current_revision`].
/// Both default to `0`, which means **"unknown / pre-history"** — that is the
/// value an old serialized graph (without these fields) deserializes to via
/// `#[serde(default)]`. Treat `0` as "this node is at least as old as anything
/// we've seen". The DSL `since N` filter and
/// [`crate::graph::CodeGraph::nodes_changed_since`] use these fields to answer
/// "what changed since revision N?".
///
/// Both fields are populated by [`crate::graph::CodeGraph`] mutation methods —
/// callers building [`NodeData`] literals can leave them at `0`; they will be
/// overwritten with `current_revision` on insert.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeData {
    pub id: NodeId,
    pub kind: NodeKind,
    pub name: String,
    pub qualified_name: String,
    pub file_path: PathBuf,
    pub span: Span,
    pub visibility: Visibility,
    pub metadata: HashMap<String, String>,
    /// Graph revision at which this node was first added. Monotonically
    /// increasing; matches [`crate::graph::CodeGraph::current_revision`] at
    /// the moment of insertion. `0` means "pre-history" — used by old
    /// serialized graphs and by callers building literals before insertion.
    #[serde(default)]
    pub birth_revision: u64,
    /// Graph revision at which this node was most recently modified — either
    /// an `add_node` overwrite (metadata / span / visibility change) or an
    /// edge addition / removal that touches this node. `0` means
    /// "pre-history" (see field-level / struct-level docs).
    #[serde(default)]
    pub last_modified_revision: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_span() -> Span {
        Span {
            file: PathBuf::from("src/main.rs"),
            start_line: 10,
            start_col: 0,
            end_line: 20,
            end_col: 1,
            byte_range: 100..300,
        }
    }

    #[test]
    fn test_node_id_deterministic() {
        let id1 = NodeId::new("src/lib.rs", "crate::foo::bar", NodeKind::Function);
        let id2 = NodeId::new("src/lib.rs", "crate::foo::bar", NodeKind::Function);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_node_id_different() {
        let id_fn = NodeId::new("src/lib.rs", "crate::foo::bar", NodeKind::Function);
        let id_struct = NodeId::new("src/lib.rs", "crate::foo::bar", NodeKind::Struct);
        let id_other_file = NodeId::new("src/other.rs", "crate::foo::bar", NodeKind::Function);
        let id_other_name = NodeId::new("src/lib.rs", "crate::foo::baz", NodeKind::Function);

        assert_ne!(id_fn, id_struct);
        assert_ne!(id_fn, id_other_file);
        assert_ne!(id_fn, id_other_name);
    }

    #[test]
    fn test_node_data_creation() {
        let span = sample_span();
        let id = NodeId::new("src/main.rs", "crate::main", NodeKind::Function);

        let node = NodeData {
            id: id.clone(),
            kind: NodeKind::Function,
            name: "main".to_string(),
            qualified_name: "crate::main".to_string(),
            file_path: PathBuf::from("src/main.rs"),
            span,
            visibility: Visibility::Public,
            metadata: HashMap::from([("async".to_string(), "false".to_string())]),
            birth_revision: 0,
            last_modified_revision: 0,
        };

        assert_eq!(node.id, id);
        assert_eq!(node.kind, NodeKind::Function);
        assert_eq!(node.name, "main");
        assert_eq!(node.qualified_name, "crate::main");
        assert_eq!(node.file_path, PathBuf::from("src/main.rs"));
        assert_eq!(node.visibility, Visibility::Public);
        assert_eq!(node.metadata.get("async"), Some(&"false".to_string()));
    }

    // Robust: graphs serialized before the revision-tracking fields existed
    // must deserialize cleanly with both fields = 0 (interpreted as
    // "pre-history"). Wire-compat: old payloads omit `birth_revision` and
    // `last_modified_revision` entirely.
    #[test]
    fn legacy_node_data_deserializes_with_default_revisions_robust() {
        let legacy_json = r#"{
            "id": 123,
            "kind": "Function",
            "name": "legacy",
            "qualified_name": "crate::legacy",
            "file_path": "src/legacy.rs",
            "span": {
                "file": "src/legacy.rs",
                "start_line": 1,
                "start_col": 0,
                "end_line": 2,
                "end_col": 0,
                "byte_range": { "start": 0, "end": 10 }
            },
            "visibility": "Public",
            "metadata": {}
        }"#;
        let node: NodeData = serde_json::from_str(legacy_json).expect("legacy decode");
        assert_eq!(node.birth_revision, 0);
        assert_eq!(node.last_modified_revision, 0);
        assert_eq!(node.name, "legacy");
    }

    #[test]
    fn test_serde_roundtrip_node() {
        let span = sample_span();
        let id = NodeId::new("src/main.rs", "crate::main", NodeKind::Function);

        let node = NodeData {
            id,
            kind: NodeKind::Function,
            name: "main".to_string(),
            qualified_name: "crate::main".to_string(),
            file_path: PathBuf::from("src/main.rs"),
            span,
            visibility: Visibility::Private,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
        };

        let json = serde_json::to_string(&node).expect("serialize");
        let deserialized: NodeData = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(deserialized.id, node.id);
        assert_eq!(deserialized.kind, node.kind);
        assert_eq!(deserialized.name, node.name);
        assert_eq!(deserialized.qualified_name, node.qualified_name);
        assert_eq!(deserialized.file_path, node.file_path);
        assert_eq!(deserialized.span, node.span);
        assert_eq!(deserialized.visibility, node.visibility);
        assert_eq!(deserialized.metadata, node.metadata);
    }
}
