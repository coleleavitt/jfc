//! Subgraph expansion strategies for context queries.
//!
//! Built on top of the BFS traversal in `crate::traversal`, these
//! routines layer the higher-level shaping that an agent-friendly
//! context call needs:
//!
//! - **Type hierarchy expansion** — follow `Implements` edges from
//!   trait / struct entry points to find parent traits and sibling
//!   implementations. BFS often exhausts its budget on contained
//!   methods before reaching `Implements` neighbours; a dedicated pass
//!   guarantees the hierarchy appears in results.
//! - **Per-file diversity cap** — prevent any single file from
//!   monopolising the node budget (BFS through `Contains` collapses
//!   onto the parent module).
//! - **Test-file deprioritisation** — cap test-path nodes to ~15% of
//!   budget unless the user asked about tests.
//! - **Edge recovery** — after node trimming, surface every retained
//!   edge between still-included nodes so the relationship map is
//!   complete even when BFS pruned aggressively.
//! - **Co-location boost** — when multiple entry-point candidates
//!   live in the same file, boost them so a cohesive cluster ranks
//!   over a scatter of unrelated single hits.

use std::collections::{HashMap, HashSet};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind};

/// Result of an expansion pass: nodes (entry-point first) plus the
/// edges connecting them that are worth showing.
#[derive(Debug, Clone, Default)]
pub struct ExpandedSubgraph {
    pub nodes: Vec<NodeId>,
    pub edges: Vec<(NodeId, NodeId, EdgeKind)>,
    pub roots: Vec<NodeId>,
}

impl ExpandedSubgraph {
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// File-path predicate: is this a test / examples / benches path?
pub fn is_test_path(path: &std::path::Path) -> bool {
    let s = path.to_string_lossy().to_lowercase();
    s.split('/').any(|seg| {
        matches!(
            seg,
            "tests" | "test" | "spec" | "__tests__" | "examples" | "benches"
        )
    }) || s.ends_with("_test.rs")
        || s.ends_with(".test.ts")
        || s.ends_with(".test.tsx")
        || s.ends_with(".spec.ts")
}

/// Walk `Implements` edges outward from `seeds` (struct/enum/trait
/// kinds only). Two passes: seed → parent trait, then parent → sibling
/// implementors. Caps total nodes returned at `budget`.
pub fn expand_type_hierarchy(
    graph: &CodeGraph,
    seeds: &[NodeId],
    budget: usize,
) -> ExpandedSubgraph {
    let mut out = ExpandedSubgraph::default();
    if budget == 0 || seeds.is_empty() {
        return out;
    }

    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut frontier: Vec<NodeId> = Vec::new();

    for seed in seeds {
        let Some(node) = graph.get_node(seed) else {
            continue;
        };
        if !matches!(
            node.kind,
            NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
        ) {
            continue;
        }
        for (tgt, edge) in graph.get_edges_from(seed) {
            if !matches!(edge.kind, EdgeKind::Implements) {
                continue;
            }
            if out.nodes.len() >= budget {
                break;
            }
            if seen.insert(tgt.clone()) {
                out.nodes.push(tgt.clone());
                out.edges
                    .push((seed.clone(), tgt.clone(), edge.kind.clone()));
                frontier.push(tgt.clone());
            }
        }
    }

    // Pass 2: walk back IN to find sibling implementors of the parents.
    for parent in frontier {
        if out.nodes.len() >= budget {
            break;
        }
        for (src, edge) in graph.get_edges_to(&parent) {
            if !matches!(edge.kind, EdgeKind::Implements) {
                continue;
            }
            if out.nodes.len() >= budget {
                break;
            }
            if seeds.contains(src) {
                continue;
            }
            if seen.insert(src.clone()) {
                out.nodes.push(src.clone());
                out.edges
                    .push((src.clone(), parent.clone(), edge.kind.clone()));
            }
        }
    }

    out
}

/// Apply a per-file diversity cap: no single file may contribute more
/// than `max_per_file` nodes. Sorts each file's nodes so entry points
/// and structural kinds (struct/enum/trait/module) win the surviving
/// slots; functions are evicted first when over-budget.
pub fn enforce_file_diversity(
    graph: &CodeGraph,
    nodes: Vec<NodeId>,
    roots: &HashSet<NodeId>,
    max_per_file: usize,
) -> Vec<NodeId> {
    if max_per_file == 0 {
        return nodes;
    }
    let mut by_file: HashMap<std::path::PathBuf, Vec<NodeId>> = HashMap::new();
    let mut order: Vec<NodeId> = Vec::with_capacity(nodes.len());
    for id in &nodes {
        order.push(id.clone());
        if let Some(node) = graph.get_node(id) {
            by_file.entry(node.file_path.clone()).or_default().push(id.clone());
        }
    }

    let mut keep: HashSet<NodeId> = HashSet::new();
    for (_, mut ids) in by_file {
        ids.sort_by_key(|id| priority_key(graph, id, roots));
        for id in ids.into_iter().take(max_per_file) {
            keep.insert(id);
        }
    }
    order.into_iter().filter(|id| keep.contains(id)).collect()
}

/// Lower key = higher priority. Roots win, then struct/enum/trait,
/// then function.
fn priority_key(
    graph: &CodeGraph,
    id: &NodeId,
    roots: &HashSet<NodeId>,
) -> u8 {
    if roots.contains(id) {
        return 0;
    }
    match graph.get_node(id).map(|n| n.kind) {
        Some(NodeKind::Struct)
        | Some(NodeKind::Enum)
        | Some(NodeKind::Trait)
        | Some(NodeKind::Module) => 1,
        Some(NodeKind::Function) => 2,
        None => 3,
    }
}

/// Cap the share of nodes coming from test / example / bench files.
/// `max_non_prod` is an *absolute* cap, computed by the caller as a
/// percentage of total budget. Roots in test files are NOT exempt —
/// if a test entry survives the cap, fine; otherwise it's dropped too,
/// because anchoring exploration results on test files generally
/// pollutes the answer.
pub fn cap_test_files(
    graph: &CodeGraph,
    nodes: Vec<NodeId>,
    max_non_prod: usize,
) -> Vec<NodeId> {
    let mut test_count = 0usize;
    let mut out = Vec::with_capacity(nodes.len());
    for id in nodes {
        let is_test = graph
            .get_node(&id)
            .map(|n| is_test_path(&n.file_path))
            .unwrap_or(false);
        if is_test {
            if test_count >= max_non_prod {
                continue;
            }
            test_count += 1;
        }
        out.push(id);
    }
    out
}

/// Discover every retained-graph edge between the surviving node set
/// using the supplied edge-kind filter. Restores connectivity that
/// BFS pruning may have dropped.
pub fn recover_edges(
    graph: &CodeGraph,
    nodes: &[NodeId],
    kinds: &[EdgeKind],
) -> Vec<(NodeId, NodeId, EdgeKind)> {
    let set: HashSet<&NodeId> = nodes.iter().collect();
    let mut edges = Vec::new();
    let mut seen: HashSet<(NodeId, NodeId, EdgeKind)> = HashSet::new();
    for src in nodes {
        for (tgt, edge) in graph.get_edges_from(src) {
            if !set.contains(tgt) {
                continue;
            }
            if !edge_kind_matches(&edge.kind, kinds) {
                continue;
            }
            let key = (src.clone(), tgt.clone(), edge.kind.clone());
            if seen.insert(key.clone()) {
                edges.push(key);
            }
        }
    }
    edges
}

fn edge_kind_matches(kind: &EdgeKind, allowed: &[EdgeKind]) -> bool {
    if allowed.is_empty() {
        return true;
    }
    allowed.iter().any(|a| std::mem::discriminant(a) == std::mem::discriminant(kind))
}

/// Co-location boost: count how many distinct *seed names* appear in
/// each file across `candidates`. Returns a per-NodeId score boost in
/// units (each extra co-located seed adds 1). Used by callers to
/// re-rank search results before truncation.
pub fn colocation_boost(graph: &CodeGraph, candidates: &[NodeId]) -> HashMap<NodeId, u32> {
    let mut file_names: HashMap<std::path::PathBuf, HashSet<String>> = HashMap::new();
    for id in candidates {
        if let Some(node) = graph.get_node(id) {
            file_names
                .entry(node.file_path.clone())
                .or_default()
                .insert(node.name.to_lowercase());
        }
    }
    let mut boosts = HashMap::new();
    for id in candidates {
        if let Some(node) = graph.get_node(id) {
            let count = file_names
                .get(&node.file_path)
                .map(|s| s.len() as u32)
                .unwrap_or(1);
            // Boost of (count - 1): a unique hit gets 0; co-located
            // duos get +1; trios +2; …
            boosts.insert(id.clone(), count.saturating_sub(1));
        }
    }
    boosts
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::edges::EdgeData;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

    fn span(file: &str) -> Span {
        Span {
            file: PathBuf::from(file),
            start_line: 1,
            start_col: 0,
            end_line: 1,
            end_col: 0,
            byte_range: 0..1,
        }
    }

    fn n(name: &str, kind: NodeKind, file: &str) -> NodeData {
        let id = NodeId::new(file, &format!("crate::{name}"), kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from(file),
            span: span(file),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn is_test_path_detects_common_layouts() {
        assert!(is_test_path(&PathBuf::from("src/foo/tests/bar.rs")));
        assert!(is_test_path(&PathBuf::from("src/__tests__/x.ts")));
        assert!(is_test_path(&PathBuf::from("examples/demo.rs")));
        assert!(is_test_path(&PathBuf::from("benches/bench.rs")));
        assert!(is_test_path(&PathBuf::from("src/foo_test.rs")));
        assert!(!is_test_path(&PathBuf::from("src/foo.rs")));
    }

    #[test]
    fn type_hierarchy_finds_implementors_and_siblings() {
        let mut g = CodeGraph::new();
        let trait_id = g.add_node(n("MyTrait", NodeKind::Trait, "src/lib.rs"));
        let a_id = g.add_node(n("A", NodeKind::Struct, "src/a.rs"));
        let b_id = g.add_node(n("B", NodeKind::Struct, "src/b.rs"));
        g.add_edge(
            &a_id,
            &trait_id,
            EdgeData {
                kind: EdgeKind::Implements,
                source_span: span("src/a.rs"),
                weight: 1.0,
            },
        )
        .unwrap();
        g.add_edge(
            &b_id,
            &trait_id,
            EdgeData {
                kind: EdgeKind::Implements,
                source_span: span("src/b.rs"),
                weight: 1.0,
            },
        )
        .unwrap();

        let result = expand_type_hierarchy(&g, &[a_id.clone()], 10);
        // From struct A: finds trait, then back-edge to sibling B.
        assert!(result.nodes.contains(&trait_id));
        assert!(result.nodes.contains(&b_id));
    }

    #[test]
    fn type_hierarchy_respects_budget() {
        let mut g = CodeGraph::new();
        let trait_id = g.add_node(n("T", NodeKind::Trait, "src/lib.rs"));
        let a_id = g.add_node(n("A", NodeKind::Struct, "src/a.rs"));
        let b_id = g.add_node(n("B", NodeKind::Struct, "src/b.rs"));
        g.add_edge(
            &a_id,
            &trait_id,
            EdgeData {
                kind: EdgeKind::Implements,
                source_span: span("src/a.rs"),
                weight: 1.0,
            },
        )
        .unwrap();
        g.add_edge(
            &b_id,
            &trait_id,
            EdgeData {
                kind: EdgeKind::Implements,
                source_span: span("src/b.rs"),
                weight: 1.0,
            },
        )
        .unwrap();
        let result = expand_type_hierarchy(&g, &[a_id], 1);
        assert_eq!(result.nodes.len(), 1);
    }

    #[test]
    fn file_diversity_evicts_lowest_priority() {
        let mut g = CodeGraph::new();
        let s_id = g.add_node(n("S", NodeKind::Struct, "src/big.rs"));
        let f1 = g.add_node(n("f1", NodeKind::Function, "src/big.rs"));
        let f2 = g.add_node(n("f2", NodeKind::Function, "src/big.rs"));
        let f3 = g.add_node(n("f3", NodeKind::Function, "src/big.rs"));

        let roots: HashSet<NodeId> = vec![s_id.clone()].into_iter().collect();
        let kept = enforce_file_diversity(
            &g,
            vec![s_id.clone(), f1.clone(), f2.clone(), f3.clone()],
            &roots,
            2,
        );
        assert_eq!(kept.len(), 2);
        // The struct (root) should always survive.
        assert!(kept.contains(&s_id));
    }

    #[test]
    fn test_cap_drops_excess_test_nodes() {
        let mut g = CodeGraph::new();
        let prod = g.add_node(n("prod", NodeKind::Function, "src/lib.rs"));
        let t1 = g.add_node(n("t1", NodeKind::Function, "src/tests/a.rs"));
        let t2 = g.add_node(n("t2", NodeKind::Function, "src/tests/b.rs"));
        let t3 = g.add_node(n("t3", NodeKind::Function, "src/tests/c.rs"));
        let kept = cap_test_files(&g, vec![prod.clone(), t1.clone(), t2.clone(), t3.clone()], 1);
        assert_eq!(kept.len(), 2);
        assert!(kept.contains(&prod));
    }

    #[test]
    fn recover_edges_includes_kept_pairs() {
        let mut g = CodeGraph::new();
        let a = g.add_node(n("a", NodeKind::Function, "src/lib.rs"));
        let b = g.add_node(n("b", NodeKind::Function, "src/lib.rs"));
        let c = g.add_node(n("c", NodeKind::Function, "src/lib.rs"));
        g.add_edge(
            &a,
            &b,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span("src/lib.rs"),
                weight: 1.0,
            },
        )
        .unwrap();
        g.add_edge(
            &a,
            &c,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span("src/lib.rs"),
                weight: 1.0,
            },
        )
        .unwrap();
        let recovered = recover_edges(&g, &[a, b], &[EdgeKind::Calls]);
        // a→c excluded because c was trimmed.
        assert_eq!(recovered.len(), 1);
    }

    #[test]
    fn colocation_boost_counts_co_resident_names() {
        let mut g = CodeGraph::new();
        let a = g.add_node(n("alpha", NodeKind::Function, "src/lib.rs"));
        let b = g.add_node(n("beta", NodeKind::Function, "src/lib.rs"));
        let c = g.add_node(n("gamma", NodeKind::Function, "src/other.rs"));
        let boosts = colocation_boost(&g, &[a.clone(), b.clone(), c.clone()]);
        assert_eq!(boosts.get(&a), Some(&1));
        assert_eq!(boosts.get(&b), Some(&1));
        assert_eq!(boosts.get(&c), Some(&0));
    }
}
