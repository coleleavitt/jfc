//! Dataflow-guided retrieval seeding (DRACO).
//!
//! From *DRACO: Dataflow-guided Retrieval Augmentation for Code Completion*
//! (arXiv:2405.17337): seed repository retrieval from the cursor's **actual
//! data/type dependencies** rather than text similarity. Walk the
//! type-sensitive dependency edges out of the symbol under the cursor, then hand
//! that dependency-ordered seed set to the existing context expander — DRACO
//! reports this is far cheaper than iterative RepoCoder-style retrieval (no
//! repeated LLM round-trips) while keeping the seeds semantically on-point.
//!
//! Here the "dataflow dependencies" of a symbol are the graph edges that carry
//! type/value flow: [`EdgeKind::UsesType`], [`Returns`], [`TypeOf`], and direct
//! [`Calls`]. [`seed_from_dataflow`] does a bounded BFS over exactly those edge
//! kinds and returns the discovered nodes in **dependency order** (nearest
//! dependencies first, deterministic within a layer), suitable as entry points
//! for [`crate::context::expansion`]. Text-similar but dataflow-unrelated
//! symbols are excluded by construction.
//!
//! [`Returns`]: crate::edges::EdgeKind::Returns
//! [`TypeOf`]: crate::edges::EdgeKind::TypeOf
//! [`Calls`]: crate::edges::EdgeKind::Calls

use std::collections::HashSet;

use crate::context::resolver::resolve_symbol;
use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::NodeId;

/// Returns true for edge kinds that carry type/value dataflow — the edges DRACO
/// follows when building a dependency seed.
fn is_dataflow_edge(kind: &EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::UsesType | EdgeKind::Returns | EdgeKind::TypeOf | EdgeKind::Calls
    )
}

/// Seed retrieval from the dataflow dependencies of `cursor_symbol`.
///
/// Resolves the symbol, then does a layer-by-layer (BFS) walk over outgoing
/// dataflow edges up to `max_depth` hops, returning the discovered dependency
/// nodes in dependency order (closest first; within a layer, in graph
/// adjacency order with duplicates removed). The cursor symbol(s) themselves are
/// not included — the result is the *context to retrieve around* the cursor.
///
/// `max_depth = 0` returns an empty seed (no dependencies walked). An
/// unresolvable symbol returns an empty seed rather than erroring.
pub fn seed_from_dataflow(graph: &CodeGraph, cursor_symbol: &str, max_depth: u8) -> Vec<NodeId> {
    let roots = resolve_symbol(graph, cursor_symbol);
    seed_from_nodes(graph, &roots, max_depth)
}

/// Lower-level variant of [`seed_from_dataflow`] taking pre-resolved root
/// node ids (e.g. the cursor's enclosing function). Same ordering/semantics.
pub fn seed_from_nodes(graph: &CodeGraph, roots: &[NodeId], max_depth: u8) -> Vec<NodeId> {
    let mut visited: HashSet<NodeId> = roots.iter().cloned().collect();
    let mut ordered: Vec<NodeId> = Vec::new();
    let mut layer: Vec<NodeId> = roots.iter().filter(|n| graph.contains_node(n)).cloned().collect();

    for _ in 0..max_depth {
        let mut next: Vec<NodeId> = Vec::new();
        for node in &layer {
            for (dep, edge) in graph.get_edges_from(node) {
                if is_dataflow_edge(&edge.kind) && visited.insert(dep.clone()) {
                    ordered.push(dep.clone());
                    next.push(dep.clone());
                }
            }
        }
        if next.is_empty() {
            break;
        }
        layer = next;
    }
    ordered
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
        EdgeData { kind: k, source_span: span(), weight: 1.0 }
    }

    // Normal: seeds follow type/dataflow edges out of the cursor symbol.
    #[test]
    fn seeds_include_type_dependencies_normal() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("handler", NodeKind::Function));
        let req = g.add_node(mk("Request", NodeKind::Struct));
        let resp = g.add_node(mk("Response", NodeKind::Struct));
        g.add_edge(&f, &req, ed(EdgeKind::UsesType)).unwrap();
        g.add_edge(&f, &resp, ed(EdgeKind::Returns)).unwrap();

        let seeds = seed_from_dataflow(&g, "handler", 2);
        assert!(seeds.contains(&req));
        assert!(seeds.contains(&resp));
        // The cursor symbol itself is not a seed.
        assert!(!seeds.contains(&f));
    }

    // Robust: a text-similar but dataflow-unrelated symbol is excluded.
    #[test]
    fn excludes_unrelated_symbol_robust() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("handler", NodeKind::Function));
        let dep = g.add_node(mk("Request", NodeKind::Struct));
        // `handler_helper` shares a textual prefix but has no dataflow edge.
        let unrelated = g.add_node(mk("handler_helper", NodeKind::Function));
        g.add_edge(&f, &dep, ed(EdgeKind::UsesType)).unwrap();
        let _ = unrelated;

        let seeds = seed_from_dataflow(&g, "handler", 2);
        assert!(seeds.contains(&dep));
        assert!(!seeds.iter().any(|n| *n == NodeId::new("t.rs", "handler_helper", NodeKind::Function)));
    }

    // Normal: BFS visits transitive dependencies in dependency order (depth 1
    // before depth 2).
    #[test]
    fn transitive_deps_in_dependency_order_normal() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        g.add_edge(&f, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();

        let seeds = seed_from_dataflow(&g, "a", 3);
        // b is a direct dependency, c is transitive -> b before c.
        let bi = seeds.iter().position(|n| *n == b).unwrap();
        let ci = seeds.iter().position(|n| *n == c).unwrap();
        assert!(bi < ci);
    }

    // Robust: max_depth bounds the walk.
    #[test]
    fn max_depth_bounds_walk_robust() {
        let mut g = CodeGraph::new();
        let f = g.add_node(mk("a", NodeKind::Function));
        let b = g.add_node(mk("b", NodeKind::Function));
        let c = g.add_node(mk("c", NodeKind::Function));
        g.add_edge(&f, &b, ed(EdgeKind::Calls)).unwrap();
        g.add_edge(&b, &c, ed(EdgeKind::Calls)).unwrap();

        // depth 1 reaches b but not c.
        let seeds = seed_from_dataflow(&g, "a", 1);
        assert!(seeds.contains(&b));
        assert!(!seeds.contains(&c));
    }

    // Robust: an unresolvable symbol yields an empty seed, no panic.
    #[test]
    fn unresolvable_symbol_is_empty_robust() {
        let g = CodeGraph::new();
        assert!(seed_from_dataflow(&g, "nonexistent", 2).is_empty());
    }

    // Robust: non-dataflow edges (e.g. Contains) are not followed.
    #[test]
    fn ignores_non_dataflow_edges_robust() {
        let mut g = CodeGraph::new();
        let m = g.add_node(mk("m", NodeKind::Module));
        let f = g.add_node(mk("f", NodeKind::Function));
        g.add_edge(&m, &f, ed(EdgeKind::Contains)).unwrap();

        let seeds = seed_from_nodes(&g, std::slice::from_ref(&m), 2);
        // Contains is structural, not dataflow -> f is not seeded.
        assert!(!seeds.contains(&f));
    }
}
