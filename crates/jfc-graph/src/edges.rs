//! Graph edge types and relationship semantics.

use serde::{Deserialize, Serialize};

use crate::nodes::{NodeKind, Span};

/// Edge kinds connecting nodes in the code graph.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EdgeKind {
    /// Resolved function call (caller → callee)
    Calls,
    /// Unresolved call (name only, cross-file, needs LSP to resolve)
    UnresolvedCall(String),
    /// Uses a type (function → struct/enum/trait it references)
    UsesType,
    /// References (any reference relationship)
    References,
    /// Contains (module → function, struct → impl methods)
    Contains,
    /// Implements (struct → trait)
    Implements,
    /// Call to external crate (crate_name, path)
    ExternalCall(String, String),
}

impl EdgeKind {
    /// Returns true if this edge kind is allowed between the given source and
    /// target node kinds.
    ///
    /// Per-edge-kind invariants (the table downstream traversal relies on):
    ///
    /// | EdgeKind          | Source                         | Target                                  |
    /// |-------------------|--------------------------------|-----------------------------------------|
    /// | `Calls`           | Function                       | Function                                |
    /// | `UnresolvedCall`  | Function                       | any (placeholder NodeId, not yet bound) |
    /// | `UsesType`        | Function                       | Struct \| Enum \| Trait                 |
    /// | `References`      | any                            | any (relaxed)                           |
    /// | `Contains`        | Module \| Struct \| Enum \| Trait | Function \| Struct \| Enum \| Trait \| Module |
    /// | `Implements`      | Struct \| Enum                 | Trait                                   |
    /// | `ExternalCall`    | Function                       | any (placeholder for external symbol)   |
    pub fn valid_for(&self, source: NodeKind, target: NodeKind) -> bool {
        match self {
            EdgeKind::Calls => source == NodeKind::Function && target == NodeKind::Function,
            EdgeKind::UnresolvedCall(_) => source == NodeKind::Function,
            EdgeKind::UsesType => {
                source == NodeKind::Function
                    && matches!(
                        target,
                        NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
                    )
            }
            EdgeKind::References => true,
            EdgeKind::Contains => {
                matches!(
                    source,
                    NodeKind::Module | NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
                ) && matches!(
                    target,
                    NodeKind::Function
                        | NodeKind::Struct
                        | NodeKind::Enum
                        | NodeKind::Trait
                        | NodeKind::Module
                )
            }
            EdgeKind::Implements => {
                matches!(source, NodeKind::Struct | NodeKind::Enum)
                    && target == NodeKind::Trait
            }
            EdgeKind::ExternalCall(_, _) => source == NodeKind::Function,
        }
    }
}

/// Edge data stored on graph edges.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EdgeData {
    pub kind: EdgeKind,
    pub source_span: Span,
    pub weight: f32,
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn sample_span() -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: 5,
            start_col: 4,
            end_line: 5,
            end_col: 20,
            byte_range: 50..80,
        }
    }

    #[test]
    fn test_edge_data_creation() {
        let span = sample_span();

        let variants = [
            EdgeKind::Calls,
            EdgeKind::UnresolvedCall("maybe_foo".to_string()),
            EdgeKind::UsesType,
            EdgeKind::References,
            EdgeKind::Contains,
            EdgeKind::Implements,
            EdgeKind::ExternalCall("serde".to_string(), "serde::Serialize".to_string()),
        ];

        for kind in variants {
            let edge = EdgeData {
                kind: kind.clone(),
                source_span: span.clone(),
                weight: 1.0,
            };
            assert_eq!(edge.kind, kind);
            assert_eq!(edge.source_span, span);
            assert_eq!(edge.weight, 1.0);
        }
    }

    #[test]
    fn test_serde_roundtrip_edge() {
        let edges = vec![
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: sample_span(),
                weight: 1.0,
            },
            EdgeData {
                kind: EdgeKind::UnresolvedCall("unknown_fn".to_string()),
                source_span: sample_span(),
                weight: 0.5,
            },
            EdgeData {
                kind: EdgeKind::ExternalCall("tokio".to_string(), "tokio::spawn".to_string()),
                source_span: sample_span(),
                weight: 0.8,
            },
        ];

        for edge in &edges {
            let json = serde_json::to_string(edge).expect("serialize");
            let deserialized: EdgeData = serde_json::from_str(&json).expect("deserialize");

            assert_eq!(deserialized.kind, edge.kind);
            assert_eq!(deserialized.source_span, edge.source_span);
            assert_eq!(deserialized.weight, edge.weight);
        }
    }

    #[test]
    fn test_valid_for_calls() {
        assert!(EdgeKind::Calls.valid_for(NodeKind::Function, NodeKind::Function));
        assert!(!EdgeKind::Calls.valid_for(NodeKind::Module, NodeKind::Function));
        assert!(!EdgeKind::Calls.valid_for(NodeKind::Function, NodeKind::Struct));
    }

    #[test]
    fn test_valid_for_implements() {
        assert!(EdgeKind::Implements.valid_for(NodeKind::Struct, NodeKind::Trait));
        assert!(EdgeKind::Implements.valid_for(NodeKind::Enum, NodeKind::Trait));
        assert!(!EdgeKind::Implements.valid_for(NodeKind::Function, NodeKind::Trait));
        assert!(!EdgeKind::Implements.valid_for(NodeKind::Struct, NodeKind::Function));
    }

    #[test]
    fn test_valid_for_contains() {
        assert!(EdgeKind::Contains.valid_for(NodeKind::Module, NodeKind::Function));
        assert!(EdgeKind::Contains.valid_for(NodeKind::Struct, NodeKind::Function));
        assert!(EdgeKind::Contains.valid_for(NodeKind::Module, NodeKind::Module));
        assert!(!EdgeKind::Contains.valid_for(NodeKind::Function, NodeKind::Function));
    }

    #[test]
    fn test_valid_for_uses_type() {
        assert!(EdgeKind::UsesType.valid_for(NodeKind::Function, NodeKind::Struct));
        assert!(EdgeKind::UsesType.valid_for(NodeKind::Function, NodeKind::Enum));
        assert!(EdgeKind::UsesType.valid_for(NodeKind::Function, NodeKind::Trait));
        assert!(!EdgeKind::UsesType.valid_for(NodeKind::Function, NodeKind::Function));
        assert!(!EdgeKind::UsesType.valid_for(NodeKind::Struct, NodeKind::Struct));
    }

    #[test]
    fn test_valid_for_unresolved_and_external() {
        let unresolved = EdgeKind::UnresolvedCall("foo".into());
        assert!(unresolved.valid_for(NodeKind::Function, NodeKind::Function));
        // UnresolvedCall is a placeholder — target may not exist with proper kind yet,
        // so we only require the source to be a Function.
        assert!(unresolved.valid_for(NodeKind::Function, NodeKind::Module));
        assert!(!unresolved.valid_for(NodeKind::Module, NodeKind::Function));

        let external = EdgeKind::ExternalCall("serde".into(), "Serialize".into());
        assert!(external.valid_for(NodeKind::Function, NodeKind::Trait));
        assert!(!external.valid_for(NodeKind::Struct, NodeKind::Trait));
    }

    #[test]
    fn test_valid_for_references_relaxed() {
        // References is intentionally relaxed — accepts any pairing.
        assert!(EdgeKind::References.valid_for(NodeKind::Module, NodeKind::Function));
        assert!(EdgeKind::References.valid_for(NodeKind::Function, NodeKind::Module));
    }
}
