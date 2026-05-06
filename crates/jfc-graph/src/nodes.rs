//! Graph node types and their properties.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::ops::Range;
use std::path::PathBuf;

/// Exactly 5 node kinds — no more, no less.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
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
        };

        assert_eq!(node.id, id);
        assert_eq!(node.kind, NodeKind::Function);
        assert_eq!(node.name, "main");
        assert_eq!(node.qualified_name, "crate::main");
        assert_eq!(node.file_path, PathBuf::from("src/main.rs"));
        assert_eq!(node.visibility, Visibility::Public);
        assert_eq!(node.metadata.get("async"), Some(&"false".to_string()));
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
