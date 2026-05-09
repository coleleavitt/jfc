//! Why-provenance for query results (Phase 10-4).
//!
//! ## Overview
//!
//! A standard query returns `Vec<NodeId>` — the *what*. Provenance
//! answers *why*: for each result node, which operators, source nodes,
//! and edges caused it to appear. This is the Datalog *why-provenance*
//! concept applied to the DSL pipeline.
//!
//! ## Data model
//!
//! - [`Provenance`] — per-node explanation: which op produced it, from
//!   which predecessors, at which depth.
//! - [`ProvenanceTrace`] — the full trace for one query execution.
//!
//! ## Integration
//!
//! The trace is an **opt-in layer** over the executor. The regular
//! executor doesn't pay for provenance unless the caller asks for it.
//! Callers pass `trace: true` in [`QueryConfig`] (future) or call
//! [`trace_query`] directly.

use std::collections::HashMap;

use crate::dsl::{DslOp, Expr, QueryConfig, QueryEngine, QueryError, QueryResult, parse_expr};
use crate::graph::CodeGraph;
use crate::nodes::NodeId;

/// One step of provenance for a single result node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProvenanceStep {
    /// Which DSL op produced this node in the working set.
    pub op_name: String,
    /// Which node(s) in the *previous* working set caused this node
    /// to be added (via edge traversal). Empty for seed ops like
    /// `fn("X")`.
    pub predecessors: Vec<NodeId>,
    /// BFS depth at which the node was discovered (0 for seeds).
    pub depth: usize,
}

/// Full provenance for one node: a chain of steps from the query's
/// root selection through each pipe op that preserved/produced it.
#[derive(Debug, Clone, Default)]
pub struct Provenance {
    pub steps: Vec<ProvenanceStep>,
}

/// Complete provenance trace for a query execution.
#[derive(Debug, Clone, Default)]
pub struct ProvenanceTrace {
    /// NodeId → Provenance. Only nodes in the final result set have
    /// entries; intermediate nodes that were filtered out are not
    /// recorded (that would be the "how-provenance" variant).
    pub entries: HashMap<NodeId, Provenance>,
    /// The final result node set, duplicated here for convenience.
    pub result_nodes: Vec<NodeId>,
}

/// Execute a query and build its provenance trace.
///
/// Implementation: runs the query through the normal executor, then
/// replays the pipe ops one-by-one recording which nodes enter/exit
/// at each stage. This is O(n × ops) and allocates per-step working
/// sets; appropriate for debugging queries, not hot paths.
pub fn trace_query(
    query: &str,
    graph: &CodeGraph,
    config: &QueryConfig,
) -> Result<ProvenanceTrace, QueryError> {
    let expr = parse_expr(query)?;

    // We only support tracing for Pipe expressions (the common case).
    let ops = match &expr {
        Expr::Pipe(ops) => ops.clone(),
        _ => {
            // For non-pipe exprs, just run and return trivial provenance.
            let engine = QueryEngine::new(graph);
            let result = engine.execute_expr(&expr, config)?;
            let mut trace = ProvenanceTrace::default();
            for id in &result.nodes {
                trace.entries.insert(
                    id.clone(),
                    Provenance {
                        steps: vec![ProvenanceStep {
                            op_name: "expr".into(),
                            predecessors: vec![],
                            depth: 0,
                        }],
                    },
                );
            }
            trace.result_nodes = result.nodes;
            return Ok(trace);
        }
    };

    let engine = QueryEngine::new(graph);
    let mut trace = ProvenanceTrace::default();

    // Replay ops one-by-one, tracking how the working set evolves.
    let mut working_set: Vec<NodeId> = Vec::new();

    for (i, op) in ops.iter().enumerate() {
        let op_name = format!("{:?}", op);

        // Execute up to and including this op.
        let partial_ops = &ops[..=i];
        let partial_result = engine.execute(partial_ops, config)?;

        let prev_set: std::collections::HashSet<NodeId> =
            working_set.iter().cloned().collect();

        // For each new node in this stage's result that wasn't in the
        // previous stage, record provenance.
        for id in &partial_result.nodes {
            if !prev_set.contains(id) {
                let step = ProvenanceStep {
                    op_name: op_name.clone(),
                    predecessors: if i == 0 {
                        vec![]
                    } else {
                        // The predecessor is any node from the previous
                        // working set that has an edge to this node.
                        working_set
                            .iter()
                            .filter(|prev_id| {
                                graph
                                    .get_edges_from(prev_id)
                                    .iter()
                                    .any(|(target, _)| *target == id)
                                    || graph
                                        .get_edges_to(prev_id)
                                        .iter()
                                        .any(|(source, _)| *source == id)
                            })
                            .cloned()
                            .collect()
                    },
                    depth: i,
                };
                trace
                    .entries
                    .entry(id.clone())
                    .or_default()
                    .steps
                    .push(step);
            }
        }

        working_set = partial_result.nodes;
    }

    trace.result_nodes = working_set;
    Ok(trace)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::path::PathBuf;

    fn span() -> Span {
        Span {
            file: PathBuf::from("t.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 0,
            byte_range: 0..0,
        }
    }

    fn mk(name: &str) -> NodeData {
        NodeData {
            id: NodeId::new("t.rs", name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.into(),
            qualified_name: name.into(),
            file_path: PathBuf::from("t.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
        }
    }

    fn ed() -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: span(),
            weight: 1.0,
        }
    }

    #[test]
    fn trace_seed_op_has_no_predecessors() {
        let mut g = CodeGraph::new();
        g.add_node(mk("foo"));
        let trace = trace_query("fn(\"foo\")", &g, &QueryConfig::default()).unwrap();
        assert_eq!(trace.result_nodes.len(), 1);
        let prov = &trace.entries[&trace.result_nodes[0]];
        assert_eq!(prov.steps.len(), 1);
        assert!(prov.steps[0].predecessors.is_empty());
    }

    #[test]
    fn trace_callees_records_predecessor() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a"));
        let b = g.add_node(mk("b"));
        g.add_edge(&a, &b, ed()).unwrap();

        let trace =
            trace_query("fn(\"a\") | callees", &g, &QueryConfig::default()).unwrap();
        // b should be in result with a as predecessor.
        let prov_b = trace.entries.get(&b).expect("b in trace");
        assert!(
            prov_b.steps.iter().any(|s| s.predecessors.contains(&a)),
            "b's provenance should reference a"
        );
    }

    #[test]
    fn trace_records_depth() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a"));
        let b = g.add_node(mk("b"));
        g.add_edge(&a, &b, ed()).unwrap();

        let trace =
            trace_query("fn(\"a\") | callees", &g, &QueryConfig::default()).unwrap();
        // a at depth 0, b at depth 1.
        let prov_a = trace.entries.get(&a).expect("a in trace");
        assert_eq!(prov_a.steps[0].depth, 0);
        let prov_b = trace.entries.get(&b).expect("b in trace");
        assert!(prov_b.steps.iter().any(|s| s.depth == 1));
    }

    #[test]
    fn trace_empty_result_produces_empty_trace() {
        let g = CodeGraph::new();
        let trace =
            trace_query("fn(\"missing\")", &g, &QueryConfig::default()).unwrap();
        assert!(trace.result_nodes.is_empty());
        assert!(trace.entries.is_empty());
    }

    #[test]
    fn trace_non_pipe_expr_returns_trivial() {
        let mut g = CodeGraph::new();
        g.add_node(mk("foo"));
        // entrypoints is a non-Pipe expr.
        let trace =
            trace_query("entrypoints kind=Test", &g, &QueryConfig::default()).unwrap();
        // May or may not have results, but should not crash.
        for (_, prov) in &trace.entries {
            assert_eq!(prov.steps[0].op_name, "expr");
        }
    }
}
