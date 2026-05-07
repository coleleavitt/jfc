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
    let node_count = inner.node_count();
    if node_count == 0 {
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
    let inner = graph.inner();
    let base = count_components(graph);
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

// ─── Transitive Reduction ────────────────────────────────────────────────────

/// An edge that is transitively redundant — removing it does not change
/// reachability in the graph. Only meaningful for acyclic graphs.
#[derive(Debug, Clone)]
pub struct RedundantEdge {
    pub from: NodeId,
    pub to: NodeId,
}

/// Compute the transitive reduction of the call graph: identify edges that are
/// redundant because an alternative path exists.
///
/// Only operates on the DAG portion of the graph (edges within SCCs are skipped).
/// Returns the list of edges that can be removed without affecting reachability.
///
/// Use case: cleaning up visual call-graph displays to show only essential edges.
pub fn transitive_reduction(graph: &CodeGraph) -> Vec<RedundantEdge> {
    let inner = graph.inner();
    let mut redundant = Vec::new();

    // For each edge (u, v), check if there is an alternative path u → ... → v
    // of length ≥ 2 (i.e., going through at least one intermediate node).
    for edge in inner.edge_references() {
        let from = edge.source();
        let to = edge.target();

        // BFS/DFS from `from` to `to` avoiding the direct edge
        let has_alt_path = {
            let mut visited: HashSet<NodeIndex> = HashSet::new();
            visited.insert(from);
            let mut stack: Vec<NodeIndex> = Vec::new();

            // Seed with neighbors of `from` OTHER than `to` via this edge
            for e in inner.edges_directed(from, Direction::Outgoing) {
                if e.target() != to || e.id() != edge.id() {
                    if e.target() == to {
                        // Found alternative direct edge — still counts as alt path
                        // but we want length ≥ 2, so continue
                    }
                    if visited.insert(e.target()) {
                        stack.push(e.target());
                    }
                }
            }

            let mut found = false;
            while let Some(current) = stack.pop() {
                if current == to {
                    found = true;
                    break;
                }
                for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
                    if visited.insert(neighbor) {
                        stack.push(neighbor);
                    }
                }
            }
            found
        };

        if has_alt_path {
            if let (Some(from_id), Some(to_id)) = (graph.node_id_for(from), graph.node_id_for(to)) {
                redundant.push(RedundantEdge {
                    from: from_id.clone(),
                    to: to_id.clone(),
                });
            }
        }
    }

    redundant
}

/// Return the essential edges — the graph with transitive-redundant edges removed.
/// This is the minimal edge set that preserves reachability.
pub fn essential_edges(graph: &CodeGraph) -> Vec<(NodeId, NodeId)> {
    let redundant: HashSet<(NodeId, NodeId)> = transitive_reduction(graph)
        .into_iter()
        .map(|r| (r.from, r.to))
        .collect();

    let inner = graph.inner();
    inner
        .edge_references()
        .filter_map(|e| {
            let from = graph.node_id_for(e.source())?.clone();
            let to = graph.node_id_for(e.target())?.clone();
            if redundant.contains(&(from.clone(), to.clone())) {
                None
            } else {
                Some((from, to))
            }
        })
        .collect()
}

// ─── Graph Coloring (Parallelism Analysis) ───────────────────────────────────

/// Result of graph coloring — nodes with the same color can be edited in parallel.
#[derive(Debug, Clone)]
pub struct ColorAssignment {
    pub node: NodeId,
    pub color: usize,
}

/// Color group — all nodes in this group can be edited simultaneously without conflicts.
#[derive(Debug, Clone)]
pub struct ParallelGroup {
    pub color: usize,
    pub members: Vec<NodeId>,
}

/// Compute a graph coloring that determines which functions can be edited simultaneously.
///
/// Two nodes that share an edge (caller/callee relationship) get different colors.
/// Nodes with the same color are independent and can be edited in parallel.
///
/// Uses a greedy DSatur-style heuristic. Returns groups sorted by size (largest first).
pub fn parallel_edit_groups(graph: &CodeGraph) -> Vec<ParallelGroup> {
    let inner = graph.inner();
    if inner.node_count() == 0 {
        return vec![];
    }

    // Build adjacency treating the directed graph as undirected
    let mut adj: HashMap<NodeIndex, HashSet<NodeIndex>> = HashMap::new();
    for idx in inner.node_indices() {
        adj.entry(idx).or_default();
        for neighbor in inner.neighbors_directed(idx, Direction::Outgoing) {
            adj.entry(idx).or_default().insert(neighbor);
            adj.entry(neighbor).or_default().insert(idx);
        }
        for neighbor in inner.neighbors_directed(idx, Direction::Incoming) {
            adj.entry(idx).or_default().insert(neighbor);
            adj.entry(neighbor).or_default().insert(idx);
        }
    }

    // Greedy coloring: assign each node the smallest color not used by neighbors
    let mut colors: HashMap<NodeIndex, usize> = HashMap::new();

    // Process nodes in order of decreasing degree (saturation heuristic)
    let mut nodes: Vec<NodeIndex> = inner.node_indices().collect();
    nodes.sort_by(|a, b| {
        let deg_a = adj.get(a).map(|s| s.len()).unwrap_or(0);
        let deg_b = adj.get(b).map(|s| s.len()).unwrap_or(0);
        deg_b.cmp(&deg_a)
    });

    for node in &nodes {
        let neighbor_colors: HashSet<usize> = adj
            .get(node)
            .map(|neighbors| {
                neighbors
                    .iter()
                    .filter_map(|n| colors.get(n).copied())
                    .collect()
            })
            .unwrap_or_default();

        // Find smallest unused color
        let mut color = 0;
        while neighbor_colors.contains(&color) {
            color += 1;
        }
        colors.insert(*node, color);
    }

    // Group by color
    let mut groups: HashMap<usize, Vec<NodeId>> = HashMap::new();
    for (idx, color) in &colors {
        if let Some(id) = graph.node_id_for(*idx) {
            groups.entry(*color).or_default().push(id.clone());
        }
    }

    let mut result: Vec<ParallelGroup> = groups
        .into_iter()
        .map(|(color, members)| ParallelGroup { color, members })
        .collect();
    result.sort_by(|a, b| b.members.len().cmp(&a.members.len()));
    result
}

/// Returns the chromatic number (minimum colors needed) — a measure of how
/// interdependent the codebase is.
pub fn chromatic_number(graph: &CodeGraph) -> usize {
    let groups = parallel_edit_groups(graph);
    groups.len()
}

// ─── Maximal Cliques (Module Clustering) ─────────────────────────────────────

/// A clique — a set of nodes that are all mutually connected.
#[derive(Debug, Clone)]
pub struct Clique {
    pub members: Vec<NodeId>,
}

/// Find all maximal cliques in the code graph (treated as undirected).
///
/// A clique is a set of functions that ALL call each other. These represent
/// tightly-coupled clusters that should probably live in the same module.
///
/// Uses the Bron-Kerbosch algorithm with pivoting. Only returns cliques
/// with 2+ members.
pub fn maximal_cliques(graph: &CodeGraph) -> Vec<Clique> {
    let inner = graph.inner();
    if inner.node_count() == 0 {
        return vec![];
    }

    // Build undirected adjacency
    let mut adj: HashMap<NodeIndex, HashSet<NodeIndex>> = HashMap::new();
    for idx in inner.node_indices() {
        adj.entry(idx).or_default();
    }
    for edge in inner.edge_references() {
        adj.entry(edge.source()).or_default().insert(edge.target());
        adj.entry(edge.target()).or_default().insert(edge.source());
    }

    let all_nodes: HashSet<NodeIndex> = inner.node_indices().collect();
    let mut cliques: Vec<Vec<NodeIndex>> = Vec::new();

    bron_kerbosch(
        &adj,
        HashSet::new(),
        all_nodes,
        HashSet::new(),
        &mut cliques,
    );

    cliques
        .into_iter()
        .filter(|c| c.len() >= 2)
        .map(|c| Clique {
            members: c
                .into_iter()
                .filter_map(|idx| graph.node_id_for(idx).cloned())
                .collect(),
        })
        .collect()
}

/// Bron-Kerbosch algorithm with pivoting.
fn bron_kerbosch(
    adj: &HashMap<NodeIndex, HashSet<NodeIndex>>,
    r: HashSet<NodeIndex>,
    mut p: HashSet<NodeIndex>,
    mut x: HashSet<NodeIndex>,
    cliques: &mut Vec<Vec<NodeIndex>>,
) {
    if p.is_empty() && x.is_empty() {
        if r.len() >= 2 {
            cliques.push(r.into_iter().collect());
        }
        return;
    }

    // Pick pivot with max degree in P ∪ X
    let pivot = p
        .union(&x)
        .max_by_key(|&&v| {
            adj.get(&v)
                .map(|n| n.intersection(&p).count())
                .unwrap_or(0)
        })
        .copied();

    let pivot_neighbors: HashSet<NodeIndex> = pivot
        .and_then(|pv| adj.get(&pv))
        .cloned()
        .unwrap_or_default();

    let candidates: Vec<NodeIndex> = p.difference(&pivot_neighbors).copied().collect();

    for v in candidates {
        let v_neighbors = adj.get(&v).cloned().unwrap_or_default();

        let mut new_r = r.clone();
        new_r.insert(v);

        let new_p: HashSet<NodeIndex> = p.intersection(&v_neighbors).copied().collect();
        let new_x: HashSet<NodeIndex> = x.intersection(&v_neighbors).copied().collect();

        bron_kerbosch(adj, new_r, new_p, new_x, cliques);

        p.remove(&v);
        x.insert(v);
    }
}

// ─── Floyd-Warshall All-Pairs Shortest Paths ─────────────────────────────────

/// Distance between two nodes.
#[derive(Debug, Clone)]
pub struct NodeDistance {
    pub from: NodeId,
    pub to: NodeId,
    pub distance: f32,
}

/// Compute all-pairs shortest paths using Floyd-Warshall.
///
/// Returns a distance matrix (as a HashMap for sparse access).
/// Only suitable for graphs with <2K nodes due to O(V³) complexity.
///
/// Use case: pre-compute distances for fast "how far is X from Y?" queries.
pub fn all_pairs_distances(graph: &CodeGraph) -> HashMap<(NodeId, NodeId), f32> {
    let inner = graph.inner();
    let indices: Vec<NodeIndex> = inner.node_indices().collect();
    let n = indices.len();

    if n == 0 || n > 2000 {
        return HashMap::new();
    }

    // Map NodeIndex → sequential index for the matrix
    let idx_to_seq: HashMap<NodeIndex, usize> =
        indices.iter().enumerate().map(|(i, &idx)| (idx, i)).collect();

    // Initialize distance matrix
    let mut dist = vec![vec![f32::INFINITY; n]; n];
    for i in 0..n {
        dist[i][i] = 0.0;
    }

    // Fill from edges
    for edge in inner.edge_references() {
        if let (Some(&from_seq), Some(&to_seq)) = (
            idx_to_seq.get(&edge.source()),
            idx_to_seq.get(&edge.target()),
        ) {
            let w = edge.weight().weight;
            if w < dist[from_seq][to_seq] {
                dist[from_seq][to_seq] = w;
            }
        }
    }

    // Floyd-Warshall relaxation
    for k in 0..n {
        for i in 0..n {
            for j in 0..n {
                let through_k = dist[i][k] + dist[k][j];
                if through_k < dist[i][j] {
                    dist[i][j] = through_k;
                }
            }
        }
    }

    // Convert back to NodeId pairs
    let mut result = HashMap::new();
    for i in 0..n {
        for j in 0..n {
            if i != j && dist[i][j] < f32::INFINITY {
                if let (Some(from_id), Some(to_id)) = (
                    graph.node_id_for(indices[i]),
                    graph.node_id_for(indices[j]),
                ) {
                    result.insert((from_id.clone(), to_id.clone()), dist[i][j]);
                }
            }
        }
    }

    result
}

/// Find the N closest nodes to a given node (by weighted path distance).
pub fn nearest_neighbors(graph: &CodeGraph, node: &NodeId, n: usize) -> Vec<(NodeId, f32)> {
    let Some(node_idx) = graph.resolve(node) else {
        return vec![];
    };
    let inner = graph.inner();

    // Use Dijkstra from this single node (more efficient than full Floyd-Warshall)
    use std::cmp::Ordering;
    use std::collections::BinaryHeap;

    #[derive(Clone, PartialEq)]
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
    let mut heap = BinaryHeap::new();
    dist.insert(node_idx, 0.0);
    heap.push(State { cost: 0.0, node: node_idx });

    while let Some(State { cost, node: current }) = heap.pop() {
        if cost > *dist.get(&current).unwrap_or(&f32::INFINITY) {
            continue;
        }
        for edge in inner.edges_directed(current, Direction::Outgoing) {
            let next = edge.target();
            let next_cost = cost + edge.weight().weight;
            if next_cost < *dist.get(&next).unwrap_or(&f32::INFINITY) {
                dist.insert(next, next_cost);
                heap.push(State { cost: next_cost, node: next });
            }
        }
    }

    let mut neighbors: Vec<(NodeId, f32)> = dist
        .iter()
        .filter(|(idx, _)| **idx != node_idx)
        .filter_map(|(idx, &cost)| graph.node_id_for(*idx).map(|id| (id.clone(), cost)))
        .collect();
    neighbors.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(Ordering::Equal));
    neighbors.truncate(n);
    neighbors
}

// ─── Dot/Graphviz Export ─────────────────────────────────────────────────────

/// Generate a Graphviz DOT representation of the code graph.
///
/// Nodes are labeled with their name and kind; edges with their kind.
/// The output can be piped to `dot -Tsvg` for visualization.
pub fn to_dot(graph: &CodeGraph) -> String {
    let inner = graph.inner();
    let mut out = String::from("digraph CodeGraph {\n");
    out.push_str("    rankdir=LR;\n");
    out.push_str("    node [shape=box, fontname=\"monospace\", fontsize=10];\n");
    out.push_str("    edge [fontname=\"monospace\", fontsize=8];\n\n");

    // Nodes
    for idx in inner.node_indices() {
        if let Some(data) = inner.node_weight(idx) {
            let shape = match data.kind {
                crate::nodes::NodeKind::Function => "box",
                crate::nodes::NodeKind::Struct => "record",
                crate::nodes::NodeKind::Enum => "diamond",
                crate::nodes::NodeKind::Module => "folder",
                crate::nodes::NodeKind::Trait => "ellipse",
            };
            let label = format!("{}\\n({:?})", data.name, data.kind);
            out.push_str(&format!(
                "    n{} [label=\"{}\", shape={}];\n",
                idx.index(),
                label,
                shape
            ));
        }
    }

    out.push('\n');

    // Edges
    for edge in inner.edge_references() {
        let label = match &edge.weight().kind {
            EdgeKind::Calls => "calls",
            EdgeKind::UnresolvedCall(name) => name.as_str(),
            EdgeKind::UsesType => "uses_type",
            EdgeKind::References => "refs",
            EdgeKind::Contains => "contains",
            EdgeKind::Implements => "implements",
            EdgeKind::ExternalCall(crate_name, _) => crate_name.as_str(),
        };
        let style = match &edge.weight().kind {
            EdgeKind::Calls => "",
            EdgeKind::UnresolvedCall(_) => ", style=dashed",
            EdgeKind::Contains => ", style=dotted",
            _ => ", style=bold",
        };
        out.push_str(&format!(
            "    n{} -> n{} [label=\"{}\"{}];\n",
            edge.source().index(),
            edge.target().index(),
            label,
            style
        ));
    }

    out.push_str("}\n");
    out
}

/// Generate a DOT representation of only a subset of nodes (e.g., query results).
pub fn to_dot_subgraph(graph: &CodeGraph, nodes: &[NodeId]) -> String {
    let node_set: HashSet<&NodeId> = nodes.iter().collect();
    let inner = graph.inner();
    let mut out = String::from("digraph QueryResult {\n");
    out.push_str("    rankdir=LR;\n");
    out.push_str("    node [shape=box, fontname=\"monospace\", fontsize=10];\n");
    out.push_str("    edge [fontname=\"monospace\", fontsize=8];\n\n");

    // Only nodes in the result set
    for node_id in nodes {
        if let Some(idx) = graph.resolve(node_id) {
            if let Some(data) = inner.node_weight(idx) {
                let shape = match data.kind {
                    crate::nodes::NodeKind::Function => "box",
                    crate::nodes::NodeKind::Struct => "record",
                    crate::nodes::NodeKind::Enum => "diamond",
                    crate::nodes::NodeKind::Module => "folder",
                    crate::nodes::NodeKind::Trait => "ellipse",
                };
                out.push_str(&format!(
                    "    n{} [label=\"{}\", shape={}];\n",
                    idx.index(),
                    data.name,
                    shape
                ));
            }
        }
    }

    out.push('\n');

    // Only edges between result nodes
    for edge in inner.edge_references() {
        let from_id = graph.node_id_for(edge.source());
        let to_id = graph.node_id_for(edge.target());
        if let (Some(from), Some(to)) = (from_id, to_id) {
            if node_set.contains(from) && node_set.contains(to) {
                let label = match &edge.weight().kind {
                    EdgeKind::Calls => "calls",
                    EdgeKind::UnresolvedCall(name) => name.as_str(),
                    EdgeKind::UsesType => "uses_type",
                    EdgeKind::References => "refs",
                    EdgeKind::Contains => "contains",
                    EdgeKind::Implements => "implements",
                    EdgeKind::ExternalCall(crate_name, _) => crate_name.as_str(),
                };
                out.push_str(&format!(
                    "    n{} -> n{} [label=\"{}\"];\n",
                    edge.source().index(),
                    edge.target().index(),
                    label
                ));
            }
        }
    }

    out.push_str("}\n");
    out
}

// ─── Filtered Graph Views ────────────────────────────────────────────────────

/// Return only call-graph edges (filtering out Contains, UsesType, etc.)
/// as a lightweight view for analysis that only cares about call relationships.
pub fn call_only_edges(graph: &CodeGraph) -> Vec<(NodeId, NodeId, f32)> {
    let inner = graph.inner();
    inner
        .edge_references()
        .filter(|e| matches!(e.weight().kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_)))
        .filter_map(|e| {
            let from = graph.node_id_for(e.source())?.clone();
            let to = graph.node_id_for(e.target())?.clone();
            Some((from, to, e.weight().weight))
        })
        .collect()
}

/// Return edges filtered by a specific kind.
pub fn edges_by_kind(graph: &CodeGraph, kind: &EdgeKind) -> Vec<(NodeId, NodeId)> {
    let inner = graph.inner();
    inner
        .edge_references()
        .filter(|e| &e.weight().kind == kind)
        .filter_map(|e| {
            let from = graph.node_id_for(e.source())?.clone();
            let to = graph.node_id_for(e.target())?.clone();
            Some((from, to))
        })
        .collect()
}

/// Compute a subgraph containing only nodes of a specific kind and edges between them.
pub fn subgraph_by_kind(graph: &CodeGraph, kind: crate::nodes::NodeKind) -> Vec<(NodeId, NodeId)> {
    let inner = graph.inner();
    let kind_nodes: HashSet<NodeIndex> = inner
        .node_indices()
        .filter(|&idx| {
            inner
                .node_weight(idx)
                .map(|n| n.kind == kind)
                .unwrap_or(false)
        })
        .collect();

    inner
        .edge_references()
        .filter(|e| kind_nodes.contains(&e.source()) && kind_nodes.contains(&e.target()))
        .filter_map(|e| {
            let from = graph.node_id_for(e.source())?.clone();
            let to = graph.node_id_for(e.target())?.clone();
            Some((from, to))
        })
        .collect()
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

    // ─── Tests for new algorithms ────────────────────────────────────────────

    #[test]
    fn test_transitive_reduction() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a→b→c and a→c (the a→c is redundant)
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();

        let redundant = transitive_reduction(&g);
        assert_eq!(redundant.len(), 1);
        assert_eq!(redundant[0].from, a);
        assert_eq!(redundant[0].to, c);

        let essential = essential_edges(&g);
        assert_eq!(essential.len(), 2);
    }

    #[test]
    fn test_transitive_reduction_no_redundancy() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // Linear chain — no redundancy
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();

        let redundant = transitive_reduction(&g);
        assert!(redundant.is_empty());
    }

    #[test]
    fn test_parallel_edit_groups() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a→b, a→c — b and c are independent of each other
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();

        let groups = parallel_edit_groups(&g);
        assert!(!groups.is_empty());

        // b and c should be in the same color group (they don't share an edge)
        let b_color = groups.iter().find(|grp| grp.members.contains(&b)).unwrap().color;
        let c_color = groups.iter().find(|grp| grp.members.contains(&c)).unwrap().color;
        assert_eq!(b_color, c_color);

        // a should be in a different color than b (they share an edge)
        let a_color = groups.iter().find(|grp| grp.members.contains(&a)).unwrap().color;
        assert_ne!(a_color, b_color);
    }

    #[test]
    fn test_chromatic_number() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // Triangle: all connected to each other
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&c, &a, edge()).unwrap();

        // Triangle needs 3 colors
        assert_eq!(chromatic_number(&g), 3);
    }

    #[test]
    fn test_maximal_cliques() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        // a↔b↔c forms a triangle (as undirected), d is connected only to a
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &a, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&c, &b, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();
        g.add_edge(&c, &a, edge()).unwrap();
        g.add_edge(&a, &d, edge()).unwrap();

        let cliques = maximal_cliques(&g);
        // Should find the {a,b,c} triangle
        let has_triangle = cliques.iter().any(|c| c.members.len() == 3);
        assert!(has_triangle);
    }

    #[test]
    fn test_all_pairs_distances() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();

        let dists = all_pairs_distances(&g);
        assert_eq!(*dists.get(&(a.clone(), b.clone())).unwrap(), 1.0);
        assert_eq!(*dists.get(&(a.clone(), c.clone())).unwrap(), 2.0);
        assert_eq!(*dists.get(&(b.clone(), c.clone())).unwrap(), 1.0);
        // No path from c to a (directed graph)
        assert!(!dists.contains_key(&(c, a)));
    }

    #[test]
    fn test_nearest_neighbors() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&a, &c, EdgeData { kind: EdgeKind::Calls, source_span: span(), weight: 5.0 }).unwrap();
        g.add_edge(&a, &d, EdgeData { kind: EdgeKind::Calls, source_span: span(), weight: 2.0 }).unwrap();

        let neighbors = nearest_neighbors(&g, &a, 2);
        assert_eq!(neighbors.len(), 2);
        // b (weight 1.0) should be closest, then d (weight 2.0)
        assert_eq!(neighbors[0].0, b);
        assert_eq!(neighbors[1].0, d);
    }

    #[test]
    fn test_weighted_shortest_path() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a→b (cost 1) + b→c (cost 1) = 2, vs a→c (cost 10)
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&a, &c, EdgeData { kind: EdgeKind::Calls, source_span: span(), weight: 10.0 }).unwrap();

        let (path, cost) = weighted_shortest_path(&g, &a, &c).unwrap();
        assert_eq!(cost, 2.0);
        assert_eq!(path, vec![a, b, c]);
    }

    #[test]
    fn test_k_shortest_paths() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // Two paths: a→b→c (cost 2) and a→c (cost 3)
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&a, &c, EdgeData { kind: EdgeKind::Calls, source_span: span(), weight: 3.0 }).unwrap();

        let paths = k_shortest_paths(&g, &a, &c, 2);
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].1, 2.0); // shortest
        assert_eq!(paths[1].1, 3.0); // second shortest
    }

    #[test]
    fn test_cycle_break_suggestions() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a→b→c→a cycle
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&c, &a, edge()).unwrap();

        let suggestions = cycle_break_suggestions(&g);
        assert!(!suggestions.is_empty());
    }

    #[test]
    fn test_dot_export() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        g.add_edge(&a, &b, edge()).unwrap();

        let dot = to_dot(&g);
        assert!(dot.contains("digraph CodeGraph"));
        assert!(dot.contains("\"a\\n(Function)\""));
        assert!(dot.contains("\"b\\n(Function)\""));
        assert!(dot.contains("calls"));
    }

    #[test]
    fn test_dot_subgraph() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();

        // Only show a and b
        let dot = to_dot_subgraph(&g, &[a.clone(), b.clone()]);
        assert!(dot.contains("digraph QueryResult"));
        assert!(dot.contains("\"a\""));
        assert!(dot.contains("\"b\""));
        // Edge a→b should be included, b→c should NOT (c not in subgraph)
        assert!(dot.contains("calls"));
    }

    #[test]
    fn test_call_only_edges() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(
            &a,
            &b,
            EdgeData {
                kind: EdgeKind::Contains,
                source_span: span(),
                weight: 1.0,
            },
        )
        .unwrap();

        let call_edges = call_only_edges(&g);
        assert_eq!(call_edges.len(), 1);
        assert_eq!(call_edges[0].0, a);
        assert_eq!(call_edges[0].1, b);
    }

    #[test]
    fn test_bridge_edges() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a↔b is a bridge; b→c is a bridge (removing either disconnects)
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &a, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();

        let bridges = bridge_edges(&g);
        // b→c is a bridge (removing it disconnects c)
        assert!(!bridges.is_empty());
    }

    #[test]
    fn test_critical_nodes() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a→b→c — removing b disconnects a from c
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();

        let critical = critical_nodes(&g);
        assert!(critical.contains(&b));
    }
}
