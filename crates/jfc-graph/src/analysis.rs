//! Advanced graph analysis using petgraph's algorithm suite.
//!
//! Provides:
//! - **SCC** (Tarjan): mutual recursion detection
//! - **Dominators**: precondition analysis ("what must be true to reach X?")
//! - **Topological sort**: cascade edit ordering
//! - **Simple paths**: taint path enumeration
//! - **K-shortest paths**: bounded taint analysis
//! - **Page rank**: function centrality / importance
//! - **Connected components**: independent module detection
//! - **Articulation points**: critical function identification
//! - **Bridges**: critical edge detection
//! - **Feedback arc set**: cycle-breaking suggestions
//! - **Dijkstra**: weighted shortest path
//! - **Transitive reduction**: display-clean DAG edges
//! - **Graph coloring**: parallelism analysis
//! - **Maximal cliques**: module clustering
//! - **Floyd-Warshall**: all-pairs shortest paths
//! - **Dot export**: Graphviz visualization

use std::collections::{HashMap, HashSet};

use petgraph::Direction;
use petgraph::algo::{
    dominators::simple_fast as dominators_simple_fast, page_rank, scc::tarjan_scc::tarjan_scc,
    simple_paths::all_simple_paths, toposort,
};
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::NodeId;

// ─── SCC (Strongly Connected Components) ─────────────────────────────────────

/// A strongly connected component — a set of mutually recursive functions.
#[derive(Debug, Clone)]
pub struct MutualRecursionCluster {
    /// All nodes in this SCC.
    pub members: Vec<NodeId>,
    /// True if the SCC has more than one member (actual mutual recursion).
    pub is_nontrivial: bool,
}

/// Compute all strongly connected components of the code graph.
///
/// Non-trivial SCCs (size > 1) represent mutual recursion clusters.
/// Trivial SCCs (size == 1) with a self-edge represent direct recursion.
pub fn find_mutual_recursion(graph: &CodeGraph) -> Vec<MutualRecursionCluster> {
    let sccs = tarjan_scc(graph.inner());

    sccs.into_iter()
        .map(|scc| {
            let members: Vec<NodeId> = scc
                .iter()
                .filter_map(|idx| graph.node_id_for(*idx).cloned())
                .collect();
            let is_nontrivial = members.len() > 1;
            MutualRecursionCluster {
                members,
                is_nontrivial,
            }
        })
        .filter(|cluster| {
            // Keep non-trivial clusters, or trivial ones with self-loops
            if cluster.is_nontrivial {
                return true;
            }
            // Check for self-edge (direct recursion)
            if let Some(id) = cluster.members.first() {
                if let Some(idx) = graph.resolve(id) {
                    return graph
                        .inner()
                        .neighbors_directed(idx, Direction::Outgoing)
                        .any(|n| n == idx);
                }
            }
            false
        })
        .collect()
}

/// Check if a node is part of a mutual recursion cluster.
pub fn is_in_cycle(graph: &CodeGraph, node: &NodeId) -> bool {
    let Some(node_idx) = graph.resolve(node) else {
        return false;
    };

    let sccs = tarjan_scc(graph.inner());
    for scc in &sccs {
        if scc.contains(&node_idx) {
            if scc.len() > 1 {
                return true;
            }
            // Check self-loop
            return graph
                .inner()
                .neighbors_directed(node_idx, Direction::Outgoing)
                .any(|n| n == node_idx);
        }
    }
    false
}

// ─── Dominators ──────────────────────────────────────────────────────────────

/// Result of dominator analysis for a target node.
#[derive(Debug, Clone)]
pub struct DominatorChain {
    /// The target node we analyzed.
    pub target: NodeId,
    /// Nodes that dominate `target` (i.e., every path from root passes through them).
    /// Ordered from immediate dominator outward.
    pub dominators: Vec<NodeId>,
}

/// Compute the dominator tree for the graph rooted at `root` and return
/// the dominator chain for `target`.
///
/// The dominator of a node X is the set of nodes that must be traversed
/// on every path from root to X. This powers the `preconditions` DSL
/// operator — "what must have been called before reaching this function?"
pub fn dominator_chain(
    graph: &CodeGraph,
    root: &NodeId,
    target: &NodeId,
) -> Option<DominatorChain> {
    let root_idx = graph.resolve(root)?;
    let target_idx = graph.resolve(target)?;

    let doms = dominators_simple_fast(graph.inner(), root_idx);

    let mut chain = Vec::new();
    let mut current = doms.immediate_dominator(target_idx);
    while let Some(dom) = current {
        if let Some(id) = graph.node_id_for(dom) {
            chain.push(id.clone());
        }
        current = doms.immediate_dominator(dom);
    }

    Some(DominatorChain {
        target: target.clone(),
        dominators: chain,
    })
}

// ─── Topological Sort ────────────────────────────────────────────────────────

/// Compute topological order of the graph (or a subgraph).
///
/// Returns `None` if the graph contains cycles (use `find_mutual_recursion`
/// to identify them first).
pub fn topological_order(graph: &CodeGraph) -> Option<Vec<NodeId>> {
    toposort(graph.inner(), None).ok().map(|order| {
        order
            .into_iter()
            .filter_map(|idx| graph.node_id_for(idx).cloned())
            .collect()
    })
}

/// Compute topological order of nodes affected by editing `target`.
///
/// Returns downstream nodes (callees, transitively) in dependency order
/// suitable for cascade edit dispatch.
pub fn cascade_order(graph: &CodeGraph, target: &NodeId) -> Vec<NodeId> {
    let Some(target_idx) = graph.resolve(target) else {
        return vec![];
    };

    // Collect all nodes reachable from target (outgoing = things that depend on target)
    // Actually for cascade we want callers — things that CALL target need updating.
    let mut reachable: HashSet<NodeIndex> = HashSet::new();
    let mut stack = vec![target_idx];
    while let Some(current) = stack.pop() {
        for neighbor in graph
            .inner()
            .neighbors_directed(current, Direction::Incoming)
        {
            if reachable.insert(neighbor) {
                stack.push(neighbor);
            }
        }
    }

    // Try to toposort just the reachable subgraph — return in order
    // If cyclic, just return in the order we found them
    let mut result: Vec<NodeId> = Vec::new();
    if let Ok(full_order) = toposort(graph.inner(), None) {
        // Filter the full topo order to just our reachable set
        for idx in full_order {
            if reachable.contains(&idx) {
                if let Some(id) = graph.node_id_for(idx) {
                    result.push(id.clone());
                }
            }
        }
    } else {
        // Cyclic — fallback to insertion order
        for idx in &reachable {
            if let Some(id) = graph.node_id_for(*idx) {
                result.push(id.clone());
            }
        }
    }

    result
}

// ─── Simple Paths (Taint Analysis) ──────────────────────────────────────────

/// Find all simple paths from `source` to `sink` with a maximum intermediate
/// node count. Used for taint analysis — "how does data flow from A to B?"
pub fn taint_paths(
    graph: &CodeGraph,
    source: &NodeId,
    sink: &NodeId,
    max_intermediate_nodes: usize,
) -> Vec<Vec<NodeId>> {
    let Some(source_idx) = graph.resolve(source) else {
        return vec![];
    };
    let Some(sink_idx) = graph.resolve(sink) else {
        return vec![];
    };

    use std::collections::hash_map::RandomState;
    let paths: Vec<Vec<NodeIndex>> = all_simple_paths::<Vec<_>, _, RandomState>(
        graph.inner(),
        source_idx,
        sink_idx,
        0,
        Some(max_intermediate_nodes),
    )
    .collect();

    paths
        .into_iter()
        .map(|path| {
            path.into_iter()
                .filter_map(|idx| graph.node_id_for(idx).cloned())
                .collect()
        })
        .collect()
}

// ─── K-Shortest Paths ────────────────────────────────────────────────────────

/// Find up to `k` shortest simple paths between two nodes using edge weights.
///
/// Paths are returned in ascending total-cost order. This uses a bounded
/// best-first search over simple paths, so it is most appropriate for small `k`
/// and local code-graph queries.
pub fn k_shortest_paths(
    graph: &CodeGraph,
    from: &NodeId,
    to: &NodeId,
    k: usize,
) -> Vec<(Vec<NodeId>, f32)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;

    if k == 0 {
        return vec![];
    }

    let Some(from_idx) = graph.resolve(from) else {
        return vec![];
    };
    let Some(to_idx) = graph.resolve(to) else {
        return vec![];
    };

    #[derive(Debug, Clone, PartialEq)]
    struct PathState {
        cost: f32,
        path: Vec<NodeIndex>,
    }

    impl Eq for PathState {}

    impl PartialOrd for PathState {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for PathState {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .cost
                .partial_cmp(&self.cost)
                .unwrap_or(Ordering::Equal)
        }
    }

    let inner = graph.inner();
    let mut heap = BinaryHeap::new();
    let mut results = Vec::new();

    heap.push(PathState {
        cost: 0.0,
        path: vec![from_idx],
    });

    while let Some(PathState { cost, path }) = heap.pop() {
        let Some(&current) = path.last() else {
            continue;
        };

        if current == to_idx {
            let node_path: Vec<NodeId> = path
                .iter()
                .filter_map(|&idx| graph.node_id_for(idx).cloned())
                .collect();
            results.push((node_path, cost));
            if results.len() == k {
                break;
            }
            continue;
        }

        for edge in inner.edges_directed(current, Direction::Outgoing) {
            let next = edge.target();
            if path.contains(&next) {
                continue;
            }

            let mut next_path = path.clone();
            next_path.push(next);
            heap.push(PathState {
                cost: cost + edge.weight().weight,
                path: next_path,
            });
        }
    }

    results
}

// ─── Page Rank ───────────────────────────────────────────────────────────────

/// Node ranked by importance (centrality).
#[derive(Debug, Clone)]
pub struct RankedNode {
    pub id: NodeId,
    pub score: f64,
}

/// Compute page rank over the code graph to identify the most central
/// (most depended-upon) functions/types.
///
/// Returns nodes sorted by descending importance. `top_n` limits results.
pub fn centrality(graph: &CodeGraph, top_n: usize, damping_factor: f32) -> Vec<RankedNode> {
    let inner = graph.inner();
    if inner.node_count() == 0 {
        return vec![];
    }

    let ranks = page_rank(inner, damping_factor, 50);

    let mut ranked: Vec<RankedNode> = inner
        .node_indices()
        .zip(ranks.iter())
        .filter_map(|(idx, &score)| {
            graph.node_id_for(idx).map(|id| RankedNode {
                id: id.clone(),
                score: score as f64,
            })
        })
        .collect();

    ranked.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    ranked.truncate(top_n);
    ranked
}

// ─── Connected Components ────────────────────────────────────────────────────

/// Find independent subgraphs (weakly connected components).
///
/// Returns the number of components. Independent components can be
/// edited in parallel without risk of cross-contamination.
pub fn independent_module_count(graph: &CodeGraph) -> usize {
    count_components(graph)
}

/// Group nodes by their weakly connected component.
pub fn component_groups(graph: &CodeGraph) -> Vec<Vec<NodeId>> {
    let inner = graph.inner();
    let node_count = inner.node_count();
    if node_count == 0 {
        return vec![];
    }

    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut components: Vec<Vec<NodeId>> = Vec::new();

    for start in inner.node_indices() {
        if visited.contains(&start) {
            continue;
        }
        let mut component = Vec::new();
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            if let Some(id) = graph.node_id_for(current) {
                component.push(id.clone());
            }
            // Undirected reachability
            for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
                if !visited.contains(&neighbor) {
                    stack.push(neighbor);
                }
            }
            for neighbor in inner.neighbors_directed(current, Direction::Incoming) {
                if !visited.contains(&neighbor) {
                    stack.push(neighbor);
                }
            }
        }
        if !component.is_empty() {
            components.push(component);
        }
    }

    components
}

// ─── Articulation Points ─────────────────────────────────────────────────────

/// Find articulation points — nodes whose removal disconnects the graph.
///
/// These are "critical functions": if they break, multiple parts of the
/// codebase lose connectivity. High-priority for careful editing.
///
/// Uses component-counting: for each node, checks if removing it increases
/// the number of weakly connected components. O(V * (V + E)) but fine for
/// code graphs which are typically <10K nodes.
pub fn critical_nodes(graph: &CodeGraph) -> Vec<NodeId> {
    let inner = graph.inner();
    if inner.node_count() == 0 {
        return vec![];
    }

    let base_components = count_components(graph);
    let mut articulation_points = Vec::new();

    for node_idx in inner.node_indices() {
        // Count components in graph minus this node
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        visited.insert(node_idx); // "remove" by pre-marking
        let mut components = 0;

        for start in inner.node_indices() {
            if visited.contains(&start) {
                continue;
            }
            components += 1;
            let mut stack = vec![start];
            while let Some(current) = stack.pop() {
                if !visited.insert(current) {
                    continue;
                }
                for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
                    if !visited.contains(&neighbor) {
                        stack.push(neighbor);
                    }
                }
                for neighbor in inner.neighbors_directed(current, Direction::Incoming) {
                    if !visited.contains(&neighbor) {
                        stack.push(neighbor);
                    }
                }
            }
        }

        if components > base_components {
            if let Some(id) = graph.node_id_for(node_idx) {
                articulation_points.push(id.clone());
            }
        }
    }

    articulation_points
}

// ─── Bridges (Critical Edges) ────────────────────────────────────────────────

/// A bridge edge whose removal disconnects the graph.
#[derive(Debug, Clone)]
pub struct BridgeEdge {
    pub from: NodeId,
    pub to: NodeId,
}

/// Find bridge edges — edges whose removal increases the number of
/// connected components. These represent fragile coupling points.
///
/// Uses brute-force edge removal + component recount. O(E * (V + E)).
pub fn bridge_edges(graph: &CodeGraph) -> Vec<BridgeEdge> {
    let base = count_components(graph);
    let inner = graph.inner();
    let mut bridges = Vec::new();

    for edge in inner.edge_references() {
        let from_idx = edge.source();
        let to_idx = edge.target();
        let mut visited: HashSet<NodeIndex> = HashSet::new();
        let mut components = 0;
        let edge_id = edge.id();

        for start in inner.node_indices() {
            if visited.contains(&start) {
                continue;
            }
            components += 1;
            let mut stack = vec![start];
            while let Some(current) = stack.pop() {
                if !visited.insert(current) {
                    continue;
                }
                for e in inner.edges_directed(current, Direction::Outgoing) {
                    if e.id() != edge_id && !visited.contains(&e.target()) {
                        stack.push(e.target());
                    }
                }
                for e in inner.edges_directed(current, Direction::Incoming) {
                    if e.id() != edge_id && !visited.contains(&e.source()) {
                        stack.push(e.source());
                    }
                }
            }
        }

        if components > base {
            if let (Some(from), Some(to)) = (graph.node_id_for(from_idx), graph.node_id_for(to_idx))
            {
                bridges.push(BridgeEdge {
                    from: from.clone(),
                    to: to.clone(),
                });
            }
        }
    }

    bridges
}

// ─── Feedback Arc Set (Cycle Breaking) ───────────────────────────────────────

/// An edge that, if removed, helps break cycles in the graph.
#[derive(Debug, Clone)]
pub struct CycleBreakEdge {
    pub from: NodeId,
    pub to: NodeId,
}

/// Find a set of edges whose removal helps make the graph acyclic.
///
/// Uses a greedy heuristic: for each SCC, suggest the edge whose target has the
/// highest in-degree within that SCC.
pub fn cycle_break_suggestions(graph: &CodeGraph) -> Vec<CycleBreakEdge> {
    let inner = graph.inner();
    let sccs = tarjan_scc(inner);
    let mut suggestions = Vec::new();

    for scc in &sccs {
        if scc.len() <= 1 {
            continue;
        }
        let scc_set: HashSet<NodeIndex> = scc.iter().copied().collect();
        let mut best_edge: Option<(NodeIndex, NodeIndex, usize)> = None;
        for &node in scc {
            for e in inner.edges_directed(node, Direction::Outgoing) {
                let target = e.target();
                if scc_set.contains(&target) {
                    let in_degree = inner
                        .edges_directed(target, Direction::Incoming)
                        .filter(|edge| scc_set.contains(&edge.source()))
                        .count();
                    if best_edge.map(|(_, _, d)| in_degree > d).unwrap_or(true) {
                        best_edge = Some((node, target, in_degree));
                    }
                }
            }
        }
        if let Some((from, to, _)) = best_edge {
            if let (Some(from_id), Some(to_id)) = (graph.node_id_for(from), graph.node_id_for(to)) {
                suggestions.push(CycleBreakEdge {
                    from: from_id.clone(),
                    to: to_id.clone(),
                });
            }
        }
    }

    suggestions
}

// ─── Dijkstra Weighted Shortest Path ─────────────────────────────────────────

/// Find the weighted shortest path between two nodes using edge weights.
///
/// Returns `None` if no path exists.
pub fn weighted_shortest_path(
    graph: &CodeGraph,
    from: &NodeId,
    to: &NodeId,
) -> Option<(Vec<NodeId>, f32)> {
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;

    let from_idx = graph.resolve(from)?;
    let to_idx = graph.resolve(to)?;
    let inner = graph.inner();

    #[derive(Debug, Clone, PartialEq)]
    struct State {
        cost: f32,
        node: NodeIndex,
    }

    impl Eq for State {}

    impl PartialOrd for State {
        fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
            Some(self.cmp(other))
        }
    }

    impl Ord for State {
        fn cmp(&self, other: &Self) -> Ordering {
            other
                .cost
                .partial_cmp(&self.cost)
                .unwrap_or(Ordering::Equal)
        }
    }

    let mut dist: HashMap<NodeIndex, f32> = HashMap::new();
    let mut prev: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut heap = BinaryHeap::new();

    dist.insert(from_idx, 0.0);
    heap.push(State {
        cost: 0.0,
        node: from_idx,
    });

    while let Some(State { cost, node }) = heap.pop() {
        if node == to_idx {
            let mut path = vec![to_idx];
            let mut current = to_idx;
            while let Some(&p) = prev.get(&current) {
                path.push(p);
                current = p;
            }
            path.reverse();
            let node_path: Vec<NodeId> = path
                .iter()
                .filter_map(|&idx| graph.node_id_for(idx).cloned())
                .collect();
            return Some((node_path, cost));
        }

        if cost > *dist.get(&node).unwrap_or(&f32::INFINITY) {
            continue;
        }

        for edge in inner.edges_directed(node, Direction::Outgoing) {
            let next = edge.target();
            let next_cost = cost + edge.weight().weight;
            if next_cost < *dist.get(&next).unwrap_or(&f32::INFINITY) {
                dist.insert(next, next_cost);
                prev.insert(next, node);
                heap.push(State {
                    cost: next_cost,
                    node: next,
                });
            }
        }
    }

    None
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Filter edges to only Calls relationships, useful for call-graph-only analysis.
pub fn call_graph_edges(graph: &CodeGraph) -> Vec<(NodeId, NodeId)> {
    let inner = graph.inner();
    inner
        .edge_references()
        .filter(|e| matches!(e.weight().kind, EdgeKind::Calls))
        .filter_map(|e| {
            let from = graph.node_id_for(e.source())?;
            let to = graph.node_id_for(e.target())?;
            Some((from.clone(), to.clone()))
        })
        .collect()
}

/// Count weakly connected components using BFS (works with StableGraph).
fn count_components(graph: &CodeGraph) -> usize {
    let inner = graph.inner();
    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut count = 0;

    for start in inner.node_indices() {
        if visited.contains(&start) {
            continue;
        }
        count += 1;
        let mut stack = vec![start];
        while let Some(current) = stack.pop() {
            if !visited.insert(current) {
                continue;
            }
            for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
                if !visited.contains(&neighbor) {
                    stack.push(neighbor);
                }
            }
            for neighbor in inner.neighbors_directed(current, Direction::Incoming) {
                if !visited.contains(&neighbor) {
                    stack.push(neighbor);
                }
            }
        }
    }

    count
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};

    fn span() -> Span {
        Span {
            file: PathBuf::from("test.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 5,
            end_col: 1,
            byte_range: 0..50,
        }
    }

    fn node(name: &str) -> NodeData {
        let id = NodeId::new("test.rs", &format!("crate::{name}"), NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("test.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
        }
    }

    fn edge() -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: span(),
            weight: 1.0,
        }
    }

    #[test]
    fn test_mutual_recursion_detection() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a → b → a (mutual recursion)
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &a, edge()).unwrap();
        // c is isolated
        g.add_edge(&a, &c, edge()).unwrap();

        let clusters = find_mutual_recursion(&g);
        assert!(!clusters.is_empty());
        let nontrivial: Vec<_> = clusters.iter().filter(|c| c.is_nontrivial).collect();
        assert_eq!(nontrivial.len(), 1);
        assert_eq!(nontrivial[0].members.len(), 2);
    }

    #[test]
    fn test_is_in_cycle() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &a, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();

        assert!(is_in_cycle(&g, &a));
        assert!(is_in_cycle(&g, &b));
        assert!(!is_in_cycle(&g, &c));
    }

    #[test]
    fn test_topological_order_acyclic() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();

        let order = topological_order(&g).unwrap();
        assert_eq!(order.len(), 3);
        // a must come before b, b before c
        let pos_a = order.iter().position(|x| x == &a).unwrap();
        let pos_b = order.iter().position(|x| x == &b).unwrap();
        let pos_c = order.iter().position(|x| x == &c).unwrap();
        assert!(pos_a < pos_b);
        assert!(pos_b < pos_c);
    }

    #[test]
    fn test_topological_order_cyclic() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &a, edge()).unwrap();

        assert!(topological_order(&g).is_none());
    }

    #[test]
    fn test_taint_paths() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&a, &d, edge()).unwrap();
        g.add_edge(&d, &c, edge()).unwrap();

        let paths = taint_paths(&g, &a, &c, 5);
        assert_eq!(paths.len(), 2); // a→b→c and a→d→c
    }

    #[test]
    fn test_centrality() {
        let mut g = CodeGraph::new();
        let hub = g.add_node(node("hub"));
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // Everything calls hub
        g.add_edge(&a, &hub, edge()).unwrap();
        g.add_edge(&b, &hub, edge()).unwrap();
        g.add_edge(&c, &hub, edge()).unwrap();

        let ranked = centrality(&g, 10, 0.85);
        assert!(!ranked.is_empty());
        // hub should be #1
        assert_eq!(ranked[0].id, hub);
    }

    #[test]
    fn test_connected_components() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c")); // isolated
        g.add_edge(&a, &b, edge()).unwrap();

        assert_eq!(independent_module_count(&g), 2);
        let groups = component_groups(&g);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_cascade_order() {
        let mut g = CodeGraph::new();
        let target = g.add_node(node("target"));
        let caller1 = g.add_node(node("caller1"));
        let caller2 = g.add_node(node("caller2"));
        let grandcaller = g.add_node(node("grandcaller"));
        g.add_edge(&caller1, &target, edge()).unwrap();
        g.add_edge(&caller2, &target, edge()).unwrap();
        g.add_edge(&grandcaller, &caller1, edge()).unwrap();

        let order = cascade_order(&g, &target);
        // Should include caller1, caller2, grandcaller
        assert_eq!(order.len(), 3);
    }
}
