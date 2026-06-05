//! Graph query history — stores recent queries and results for inspection.

use std::collections::VecDeque;

use crate::dsl::QueryResult;

/// A recorded graph query with its result.
#[derive(Debug, Clone)]
pub struct QueryRecord {
    pub query_text: String,
    pub result_node_count: usize,
    pub was_truncated: bool,
    pub cycles_detected: usize,
    pub timestamp_ms: u64,
}

/// Stores the last N graph queries for inspection and replay.
pub struct GraphHistory {
    records: VecDeque<QueryRecord>,
    max_records: usize,
}

impl GraphHistory {
    pub fn new(max_records: usize) -> Self {
        Self {
            records: VecDeque::new(),
            max_records,
        }
    }

    /// Record a query execution.
    pub fn record(&mut self, query_text: &str, result: &QueryResult) {
        if self.records.len() >= self.max_records {
            self.records.pop_front();
        }
        self.records.push_back(QueryRecord {
            query_text: query_text.to_string(),
            result_node_count: result.nodes.len(),
            was_truncated: result.was_truncated,
            cycles_detected: result.cycles_detected.len(),
            timestamp_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        });
    }

    /// Get all records (most recent last).
    pub fn all(&self) -> &VecDeque<QueryRecord> {
        &self.records
    }

    /// Get the last N records (most recent first).
    pub fn last_n(&self, n: usize) -> Vec<&QueryRecord> {
        self.records.iter().rev().take(n).collect()
    }

    /// Get total query count.
    pub fn len(&self) -> usize {
        self.records.len()
    }

    pub fn is_empty(&self) -> bool {
        self.records.is_empty()
    }

    /// Clear all history.
    pub fn clear(&mut self) {
        self.records.clear();
    }
}

impl Default for GraphHistory {
    fn default() -> Self {
        Self::new(50)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl::QueryResult;
    use crate::nodes::NodeId;

    fn mock_result(count: usize) -> QueryResult {
        QueryResult {
            nodes: (0..count)
                .map(|i| {
                    NodeId::new(
                        "test.rs",
                        &format!("crate::fn_{i}"),
                        crate::nodes::NodeKind::Function,
                    )
                })
                .collect(),
            edges: vec![],
            was_truncated: false,
            total_before_truncation: count,
            cycles_detected: vec![],
            metadata: vec![],
        }
    }

    #[test]
    fn test_history_record() {
        let mut history = GraphHistory::new(10);
        history.record("fn(\"foo\") | callees", &mock_result(3));
        assert_eq!(history.len(), 1);
        assert_eq!(history.all()[0].query_text, "fn(\"foo\") | callees");
        assert_eq!(history.all()[0].result_node_count, 3);
    }

    #[test]
    fn test_history_max_records() {
        let mut history = GraphHistory::new(3);
        for i in 0..5 {
            history.record(&format!("query_{i}"), &mock_result(i));
        }
        assert_eq!(history.len(), 3);
        assert_eq!(history.all()[0].query_text, "query_2");
    }

    #[test]
    fn test_history_last_n() {
        let mut history = GraphHistory::new(10);
        for i in 0..5 {
            history.record(&format!("q{i}"), &mock_result(i));
        }
        let last_2 = history.last_n(2);
        assert_eq!(last_2.len(), 2);
        assert_eq!(last_2[0].query_text, "q4");
    }

    #[test]
    fn test_history_clear() {
        let mut history = GraphHistory::new(10);
        history.record("q1", &mock_result(1));
        history.record("q2", &mock_result(2));
        assert_eq!(history.len(), 2);
        history.clear();
        assert!(history.is_empty());
    }
}

// ─── Per-node revision tracking tests ─────────────────────────────────────
//
// These exercise the temporal dimension wired into [`crate::graph::CodeGraph`]
// and [`crate::nodes::NodeData`]. They live in this module per the
// "history" theme; the methods under test are on `CodeGraph` (which is the
// natural home for revision state — `GraphHistory` above is a different
// concept, recording DSL queries).
#[cfg(test)]
mod revision_tracking_tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use crate::edges::{EdgeData, EdgeKind};
    use crate::graph::CodeGraph;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

    fn span() -> Span {
        Span {
            file: PathBuf::from("t.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 5,
            end_col: 1,
            byte_range: 0..50,
        }
    }

    fn node(name: &str) -> NodeData {
        NodeData {
            id: NodeId::new("t.rs", &format!("crate::{name}"), NodeKind::Function),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("t.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            // Stamped on insert by `add_node`; literal values are
            // overwritten with the graph's current revision.
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn call_edge() -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: span(),
            weight: 1.0,
        }
    }

    // Normal: revisions stamp monotonically. Three nodes inserted in
    // sequence end up at revisions 1, 2, 3. Querying `since 2` returns
    // nodes 2 and 3 (the inclusive `>=` cutoff is intentional — it lets
    // callers pass `current_revision()` taken before a batch and capture
    // every node touched in the batch).
    #[test]
    fn nodes_changed_since_returns_recent_normal() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));

        // Insertion-order revisions: a=1, b=2, c=3.
        assert_eq!(g.get_node(&a).unwrap().last_modified_revision, 1);
        assert_eq!(g.get_node(&b).unwrap().last_modified_revision, 2);
        assert_eq!(g.get_node(&c).unwrap().last_modified_revision, 3);

        let recent = g.nodes_changed_since(2);
        assert_eq!(recent.len(), 2);
        assert!(recent.contains(&b));
        assert!(recent.contains(&c));
        assert!(!recent.contains(&a));
    }

    // Robust: a `since` cutoff in the future returns the empty set.
    // Off-by-one safety: the cutoff is `>=`, so `since current_revision + 1`
    // must yield zero results even if the graph has been mutated.
    #[test]
    fn nodes_changed_since_returns_empty_when_future_robust() {
        let mut g = CodeGraph::new();
        let _a = g.add_node(node("a"));
        let _b = g.add_node(node("b"));
        let cutoff = g.current_revision() + 1;
        let recent = g.nodes_changed_since(cutoff);
        assert!(recent.is_empty(), "expected empty for future cutoff");
    }

    // Normal: a node modified at revision R, and its neighbor (untouched
    // since revision R-1) is included by `nodes_changed_within_depth(R, 1)`.
    // That's the "what's near the change?" use-case the public method is
    // designed for. Note: edge insertion bumps both endpoints' last-mod,
    // so `b`'s last-mod ends up at the post-edge revision; we mutate `c`
    // (an isolated node) at a later revision to construct the seed +
    // depth-1 expansion explicitly.
    #[test]
    fn nodes_changed_within_depth_includes_neighbors_normal() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let _c = g.add_node(node("c")); // isolated
        g.add_edge(&a, &b, call_edge()).unwrap();
        // Up to here every existing node has been touched. Capture the
        // cutoff and then mutate ONLY `a` (overwriting its metadata
        // bumps the revision and stamps `a` alone).
        let cutoff = g.current_revision() + 1;
        let mut a2 = node("a");
        a2.metadata.insert("flag".to_string(), "1".to_string());
        g.add_node(a2);

        // Seed set: just `a`. Depth-1 expansion: `a` plus its neighbor `b`.
        let seed = g.nodes_changed_since(cutoff);
        assert_eq!(seed.len(), 1, "seed = {seed:?}");
        assert!(seed.contains(&a));
        let expanded = g.nodes_changed_within_depth(cutoff, 1);
        assert!(expanded.contains(&a));
        assert!(expanded.contains(&b));
        assert!(!expanded.contains(&_c));
    }

    // Normal: re-adding a node with the same `NodeId` is the "update"
    // path. `last_modified_revision` advances; `birth_revision` is
    // preserved (the node was born when it first appeared, not at the
    // overwrite).
    #[test]
    fn update_node_bumps_last_modified_normal() {
        let mut g = CodeGraph::new();
        let id = g.add_node(node("a"));
        let original = g.get_node(&id).unwrap().clone();
        assert_eq!(original.birth_revision, 1);
        assert_eq!(original.last_modified_revision, 1);

        // Overwrite path: same NodeId, different metadata.
        let mut updated = node("a");
        updated
            .metadata
            .insert("updated".to_string(), "y".to_string());
        g.add_node(updated);
        let after = g.get_node(&id).unwrap();
        assert_eq!(after.birth_revision, 1, "birth must be preserved");
        assert!(after.last_modified_revision > original.last_modified_revision);
    }

    // Normal: every mutation bumps `current_revision` by 1.
    #[test]
    fn current_revision_increments_on_mutation_normal() {
        let mut g = CodeGraph::new();
        assert_eq!(g.current_revision(), 0);
        g.add_node(node("a"));
        assert_eq!(g.current_revision(), 1);
        g.add_node(node("b"));
        assert_eq!(g.current_revision(), 2);
        let a = NodeId::new("t.rs", "crate::a", NodeKind::Function);
        let b = NodeId::new("t.rs", "crate::b", NodeKind::Function);
        g.add_edge(&a, &b, call_edge()).unwrap();
        assert_eq!(g.current_revision(), 3);
        g.remove_node(&b);
        assert_eq!(g.current_revision(), 4);
    }
}
