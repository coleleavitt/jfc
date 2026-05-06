//! Token-budgeted output formatting for query results.

use crate::dsl::QueryResult;
use crate::graph::CodeGraph;
use crate::symbols::SymbolTable;

/// Formatted output from a query with token budget tracking.
#[derive(Debug, Clone)]
pub struct FormattedOutput {
    pub text: String,
    pub token_estimate: usize,
    pub was_truncated: bool,
    pub nodes_shown: usize,
    pub nodes_total: usize,
}

/// Format a query result with a token budget.
///
/// Uses a rough chars/4 estimate for token counting. Stops adding nodes
/// once the budget would be exceeded.
pub fn format_query_result(
    result: &QueryResult,
    graph: &CodeGraph,
    symbols: Option<&SymbolTable>,
    budget: usize,
) -> FormattedOutput {
    let mut lines = Vec::new();
    let mut token_count = 0;
    let mut nodes_shown = 0;

    for node_id in &result.nodes {
        if let Some(node) = graph.get_node(node_id) {
            let handle = symbols
                .and_then(|s| s.handle_for_node(node_id))
                .unwrap_or("?");
            let line = format!(
                "[{}] {:?} {} ({}:{})",
                handle,
                node.kind,
                node.name,
                node.file_path.display(),
                node.span.start_line
            );
            let line_tokens = line.len() / 4; // rough estimate
            if token_count + line_tokens > budget {
                break;
            }
            token_count += line_tokens;
            nodes_shown += 1;
            lines.push(line);
        }
    }

    let was_truncated = nodes_shown < result.nodes.len();
    if was_truncated {
        let remaining = result.nodes.len() - nodes_shown;
        lines.push(format!("... and {} more nodes", remaining));
    }

    FormattedOutput {
        text: lines.join("\n"),
        token_estimate: token_count,
        was_truncated,
        nodes_shown,
        nodes_total: result.nodes.len(),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

    fn make_span() -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 10,
            end_col: 1,
            byte_range: 0..100,
        }
    }

    fn make_node(name: &str) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: make_span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
        }
    }

    fn build_test_graph_and_result() -> (CodeGraph, QueryResult) {
        let mut graph = CodeGraph::new();
        let n1 = make_node("alpha");
        let n2 = make_node("beta");
        let n3 = make_node("gamma");

        let id1 = graph.add_node(n1);
        let id2 = graph.add_node(n2);
        let id3 = graph.add_node(n3);

        let result = QueryResult {
            nodes: vec![id1, id2, id3],
            edges: vec![],
            was_truncated: false,
            total_before_truncation: 3,
            cycles_detected: vec![],
        };

        (graph, result)
    }

    #[test]
    fn test_format_basic() {
        let (graph, result) = build_test_graph_and_result();
        let output = format_query_result(&result, &graph, None, 1000);

        assert_eq!(output.nodes_shown, 3);
        assert_eq!(output.nodes_total, 3);
        assert!(!output.was_truncated);
        assert!(output.text.contains("alpha"));
        assert!(output.text.contains("beta"));
        assert!(output.text.contains("gamma"));
    }

    #[test]
    fn test_format_truncation() {
        let (graph, result) = build_test_graph_and_result();
        // Each line is ~50 chars → ~12 tokens. Budget of 10 fits 0 full nodes,
        // budget of 15 fits ~1 node.
        let output = format_query_result(&result, &graph, None, 15);

        assert!(output.was_truncated);
        assert!(output.nodes_shown < 3);
        assert!(output.text.contains("more nodes"));
    }

    #[test]
    fn test_format_empty() {
        let graph = CodeGraph::new();
        let result = QueryResult {
            nodes: vec![],
            edges: vec![],
            was_truncated: false,
            total_before_truncation: 0,
            cycles_detected: vec![],
        };

        let output = format_query_result(&result, &graph, None, 1000);
        assert_eq!(output.nodes_shown, 0);
        assert_eq!(output.nodes_total, 0);
        assert!(!output.was_truncated);
        assert!(output.text.is_empty());
    }
}
