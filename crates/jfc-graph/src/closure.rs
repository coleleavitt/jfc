//! Transitive-closure / fixpoint operations over the call graph
//! (Phase 10-2).
//!
//! Bounded `depth N` queries answer "everything within N hops"; this
//! module adds the **unbounded** variant: walk until the working set
//! stops growing. Cycles are handled implicitly because the visited
//! set never re-admits a node.
//!
//! ## Use cases
//!
//! - `closure(seed=fn("main"), kind=Calls, direction=Outgoing)` —
//!   every function transitively reachable from `main`.
//! - `closure(seed=fn("dangerous_op"), kind=Calls, direction=Incoming)` —
//!   every function that could ever, transitively, call
//!   `dangerous_op`.
//! - `closure(seed=Trait("Display"), kind=Implements, direction=Incoming)` —
//!   every implementor of `Display`, including transitive impls
//!   through generics. (Not currently emitted by the rust adapter
//!   but the primitive is generic.)
//!
//! ## Termination
//!
//! Always terminates because the visited set is bounded by the graph
//! size and we only add to it. Worst case is O(V + E).

use std::collections::HashSet;

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::NodeId;

/// Direction of the transitive walk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClosureDirection {
    /// Follow outgoing edges (e.g. callers → callees).
    Outgoing,
    /// Follow incoming edges (e.g. callees → callers).
    Incoming,
}

/// Compute the transitive closure of `seed` under edges matching
/// `edge_kind` in the given direction. Returns every node reachable
/// from any seed (excluding the seeds themselves unless they're
/// reachable from another seed).
///
/// Cost: O(V + E) worst case. Allocates a `HashSet<NodeId>` for the
/// visited set.
pub fn closure(
    graph: &CodeGraph,
    seeds: &[NodeId],
    edge_kind_match: impl Fn(&EdgeKind) -> bool,
    direction: ClosureDirection,
) -> Vec<NodeId> {
    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut frontier: Vec<NodeId> = Vec::new();

    for seed in seeds {
        if graph.contains_node(seed) {
            visited.insert(seed.clone());
            frontier.push(seed.clone());
        }
    }

    while let Some(curr) = frontier.pop() {
        let neighbours = match direction {
            ClosureDirection::Outgoing => graph.get_edges_from(&curr),
            ClosureDirection::Incoming => graph.get_edges_to(&curr),
        };

        for (other, edge) in neighbours {
            if !edge_kind_match(&edge.kind) {
                continue;
            }
            if visited.insert(other.clone()) {
                frontier.push(other.clone());
            }
        }
    }

    // Strip the seeds out — by convention `closure` returns
    // everything *transitively* reachable, not the seeds themselves.
    // Callers who want them included should `chain` them in.
    let seed_set: HashSet<&NodeId> = seeds.iter().collect();
    visited
        .into_iter()
        .filter(|n| !seed_set.contains(n))
        .collect()
}

/// Convenience: closure over `Calls` edges only (the most common
/// shape).
pub fn calls_closure(
    graph: &CodeGraph,
    seeds: &[NodeId],
    direction: ClosureDirection,
) -> Vec<NodeId> {
    closure(graph, seeds, |k| matches!(k, EdgeKind::Calls), direction)
}

/// Convenience: closure over any edge kind (treats the graph as
/// undirected-by-kind).
pub fn any_closure(
    graph: &CodeGraph,
    seeds: &[NodeId],
    direction: ClosureDirection,
) -> Vec<NodeId> {
    closure(graph, seeds, |_| true, direction)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::EdgeData;
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::collections::HashMap;
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

    fn mk(name: &str, kind: NodeKind) -> NodeData {
        NodeData {
            id: NodeId::new("t.rs", name, kind),
            kind,
            name: name.into(),
            qualified_name: name.into(),
            file_path: PathBuf::from("t.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn ed(k: EdgeKind) -> EdgeData {
        EdgeData {
            kind: k,
            source_span: span(),
            weight: 1.0,
        }
    }

    #[test]
    fn closure_walks_to_fixpoint() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        let d = g.add_node(mk("d", NodeKind::Function));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&c, &d, ed(EdgeKind::Calls)).unwrap();

        let res = calls_closure(&g, std::slice::from_ref(&a), ClosureDirection::Outgoing);
        assert_eq!(res.len(), 3);
        assert!(res.contains(&b));
        assert!(res.contains(&c));
        assert!(res.contains(&d));
        assert!(!res.contains(&a)); // seed excluded
    }

    #[test]
    fn closure_terminates_on_cycle() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&c, &a, ed(EdgeKind::Calls)).unwrap();

        let res = calls_closure(&g, std::slice::from_ref(&a), ClosureDirection::Outgoing);
        assert_eq!(res.len(), 2);
        assert!(res.contains(&b));
        assert!(res.contains(&c));
    }

    #[test]
    fn closure_incoming_finds_callers() {
        let mut g = CodeGraph::new();
        let caller = g.add_node(mk("caller", NodeKind::Function));
        let mid = g.add_node(mk("mid", NodeKind::Function));
        let target = g.add_node(mk("target", NodeKind::Function));
        g.add_edge(&caller, &mid, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&mid, &target, ed(EdgeKind::Calls)).unwrap();

        let res = calls_closure(
            &g,
            std::slice::from_ref(&target),
            ClosureDirection::Incoming,
        );
        assert!(res.contains(&caller));
        assert!(res.contains(&mid));
    }

    #[test]
    fn closure_empty_for_isolated_node() {
        let mut g = CodeGraph::new();
        let solo = g.add_node(mk("solo", NodeKind::Function));
        let res = calls_closure(&g, &[solo], ClosureDirection::Outgoing);
        assert!(res.is_empty());
    }

    #[test]
    fn closure_filters_by_edge_kind() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("f", NodeKind::Function));
        let g_fn = g.add_node(mk("g", NodeKind::Function));
        let s = g.add_node(mk("S", NodeKind::Struct));
        g.add_edge(&f, &g_fn, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&f, &s, ed(EdgeKind::UsesType)).unwrap();

        // Only Calls edges → S is unreachable.
        let calls_only = calls_closure(&g, std::slice::from_ref(&f), ClosureDirection::Outgoing);
        assert!(calls_only.contains(&g_fn));
        assert!(!calls_only.contains(&s));

        // Any edge → S is reachable.
        let any = any_closure(&g, &[f], ClosureDirection::Outgoing);
        assert!(any.contains(&s));
    }

    #[test]
    fn closure_multiple_seeds_unioned() {
        let mut g = CodeGraph::new();
        let a = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        let d = g.add_node(mk("d", NodeKind::Function));
        g.add_edge(&a, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&c, &d, ed(EdgeKind::Calls)).unwrap();
        let res = calls_closure(&g, &[a.clone(), c.clone()], ClosureDirection::Outgoing);
        assert!(res.contains(&b));
        assert!(res.contains(&d));
    }

    #[test]
    fn closure_handles_empty_seeds() {
        let g = CodeGraph::new();
        let res = calls_closure(&g, &[], ClosureDirection::Outgoing);
        assert!(res.is_empty());
    }

    #[test]
    fn closure_skips_unknown_seeds() {
        let g = CodeGraph::new();
        let phantom = NodeId::new("nowhere.rs", "x", NodeKind::Function);
        let res = calls_closure(&g, &[phantom], ClosureDirection::Outgoing);
        assert!(res.is_empty());
    }
}
