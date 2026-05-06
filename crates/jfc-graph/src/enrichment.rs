//! LSP enrichment layer — resolves UnresolvedCall edges using LSP data.
//!
//! The `LspDataProvider` trait is defined here (in jfc-graph) and implemented
//! by jfc-ui's LspClient. This avoids circular dependencies.

use std::path::{Path, PathBuf};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;

/// Location returned by LSP operations.
#[derive(Debug, Clone)]
pub struct LspLocation {
    pub file: PathBuf,
    pub line: u32,
    pub col: u32,
}

/// Trait for providing LSP data. Implemented by jfc-ui's LspClient.
pub trait LspDataProvider: Send + Sync {
    /// Get definition location for symbol at position.
    fn goto_definition(&self, file: &Path, line: u32, col: u32) -> Option<LspLocation>;
    /// Get all references to symbol at position.
    fn find_references(&self, file: &Path, line: u32, col: u32) -> Vec<LspLocation>;
}

/// Statistics from an enrichment pass.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct EnrichmentStats {
    pub total_unresolved: usize,
    pub resolved_internal: usize,
    pub resolved_external: usize,
    pub failed: usize,
}

/// Enriches graph by resolving UnresolvedCall edges via LSP.
pub struct LspEnricher;

impl LspEnricher {
    /// Attempt to resolve UnresolvedCall edges using the LSP provider.
    /// Returns statistics about what was resolved (read-only, no edge mutation in v1).
    pub fn enrich_call_edges(
        graph: &CodeGraph,
        provider: &dyn LspDataProvider,
        workspace_root: &Path,
    ) -> EnrichmentStats {
        let mut stats = EnrichmentStats::default();

        for node_id in graph.all_node_ids() {
            for (_target_id, edge) in graph.get_edges_from(node_id) {
                if let EdgeKind::UnresolvedCall(ref _name) = edge.kind {
                    stats.total_unresolved += 1;

                    let result = provider.goto_definition(
                        &edge.source_span.file,
                        edge.source_span.start_line,
                        edge.source_span.start_col,
                    );

                    match result {
                        Some(loc) if loc.file.starts_with(workspace_root) => {
                            stats.resolved_internal += 1;
                        }
                        Some(_) => {
                            stats.resolved_external += 1;
                        }
                        None => {
                            stats.failed += 1;
                        }
                    }
                }
            }
        }

        stats
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::edges::EdgeData;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

    struct MockProvider {
        definition: Option<LspLocation>,
    }

    impl LspDataProvider for MockProvider {
        fn goto_definition(&self, _file: &Path, _line: u32, _col: u32) -> Option<LspLocation> {
            self.definition.clone()
        }

        fn find_references(&self, _file: &Path, _line: u32, _col: u32) -> Vec<LspLocation> {
            Vec::new()
        }
    }

    fn sample_span(file: &str, start_line: u32, end_line: u32) -> Span {
        Span {
            file: PathBuf::from(file),
            start_line,
            start_col: 0,
            end_line,
            end_col: 1,
            byte_range: 0..100,
        }
    }

    fn make_node(file: &str, name: &str, kind: NodeKind, start: u32, end: u32) -> NodeData {
        let id = NodeId::new(file, &format!("crate::{name}"), kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from(file),
            span: sample_span(file, start, end),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
        }
    }

    fn build_graph_with_unresolved() -> CodeGraph {
        let mut graph = CodeGraph::new();

        let caller = make_node("src/main.rs", "caller", NodeKind::Function, 1, 10);
        let callee = make_node("src/lib.rs", "callee", NodeKind::Function, 5, 15);

        let caller_id = graph.add_node(caller);
        let callee_id = graph.add_node(callee);

        let edge = EdgeData {
            kind: EdgeKind::UnresolvedCall("callee".to_string()),
            source_span: sample_span("src/main.rs", 3, 3),
            weight: 1.0,
        };

        graph.add_edge(&caller_id, &callee_id, edge).unwrap();
        graph
    }

    #[test]
    fn test_enrich_resolves_internal() {
        let graph = build_graph_with_unresolved();

        let provider = MockProvider {
            definition: Some(LspLocation {
                file: PathBuf::from("/workspace/src/lib.rs"),
                line: 7,
                col: 0,
            }),
        };

        let stats = LspEnricher::enrich_call_edges(&graph, &provider, Path::new("/workspace"));

        assert_eq!(stats.total_unresolved, 1);
        assert_eq!(stats.resolved_internal, 1);
        assert_eq!(stats.resolved_external, 0);
        assert_eq!(stats.failed, 0);
    }

    #[test]
    fn test_enrich_fails_gracefully() {
        let graph = build_graph_with_unresolved();

        let provider = MockProvider { definition: None };
        let stats = LspEnricher::enrich_call_edges(&graph, &provider, Path::new("/workspace"));

        assert_eq!(stats.total_unresolved, 1);
        assert_eq!(stats.resolved_internal, 0);
        assert_eq!(stats.resolved_external, 0);
        assert_eq!(stats.failed, 1);
    }

    #[test]
    fn test_enrich_external() {
        let graph = build_graph_with_unresolved();

        let provider = MockProvider {
            definition: Some(LspLocation {
                file: PathBuf::from("/usr/lib/rustlib/std/src/io.rs"),
                line: 100,
                col: 0,
            }),
        };

        let stats = LspEnricher::enrich_call_edges(&graph, &provider, Path::new("/workspace"));

        assert_eq!(stats.total_unresolved, 1);
        assert_eq!(stats.resolved_internal, 0);
        assert_eq!(stats.resolved_external, 1);
        assert_eq!(stats.failed, 0);
    }

    #[test]
    fn test_enrich_no_unresolved_edges() {
        let mut graph = CodeGraph::new();

        let a = make_node("src/lib.rs", "alpha", NodeKind::Function, 1, 10);
        let b = make_node("src/lib.rs", "beta", NodeKind::Function, 11, 20);

        let a_id = graph.add_node(a);
        let b_id = graph.add_node(b);

        let edge = EdgeData {
            kind: EdgeKind::Calls,
            source_span: sample_span("src/lib.rs", 5, 5),
            weight: 1.0,
        };
        graph.add_edge(&a_id, &b_id, edge).unwrap();

        let provider = MockProvider { definition: None };
        let stats = LspEnricher::enrich_call_edges(&graph, &provider, Path::new("/workspace"));

        assert_eq!(stats.total_unresolved, 0);
    }

    #[test]
    fn test_empty_graph() {
        let graph = CodeGraph::new();
        let provider = MockProvider { definition: None };
        let stats = LspEnricher::enrich_call_edges(&graph, &provider, Path::new("/workspace"));
        assert_eq!(stats, EnrichmentStats::default());
    }
}
