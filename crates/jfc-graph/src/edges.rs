//! Graph edge types and relationship semantics.

use serde::{Deserialize, Serialize};

use crate::nodes::Span;

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
}
