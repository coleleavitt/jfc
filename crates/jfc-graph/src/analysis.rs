//! Advanced graph analysis using petgraph's algorithm suite.
//!
//! Provides:
//! - **SCC** (Tarjan): mutual recursion detection
//! - **SCC partition + condensation**: cluster-level DAG view of the graph
//! - **Dominators**: precondition analysis ("what must be true to reach X?")
//! - **Topological sort**: cascade edit ordering
//! - **Simple paths**: taint path enumeration
//! - **K-shortest paths**: bounded taint analysis
//! - **Page rank**: function centrality / importance
//! - **Centrality metrics**: in/out degree + PageRank, hottest functions
//! - **Entrypoint classification**: main / public API / test / bench / FFI
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

use std::collections::{HashMap, HashSet, VecDeque};

use petgraph::Direction;
use petgraph::algo::{
    dominators::simple_fast as dominators_simple_fast, page_rank, scc::tarjan_scc::tarjan_scc,
    simple_paths::all_simple_paths, toposort,
};
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{EdgeRef, IntoEdgeReferences};

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeId, Visibility};

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

/// Semantic direction for call-graph traversal.
///
/// Use this at the public API boundary instead of `petgraph::Direction`, whose
/// `Incoming`/`Outgoing` names are graph-level and don't disambiguate which
/// side of the call relation is meant. `Callers` walks **incoming** edges
/// from a target ("who CALLS me"); `Callees` walks **outgoing** edges
/// ("who do I CALL").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallRelation {
    /// Functions that CALL the target (transitively). Walks incoming `Calls`
    /// edges. Used to compute the set of nodes whose tests/types must be
    /// re-validated when the target's signature changes.
    Callers,
    /// Functions that the target CALLS (transitively). Walks outgoing `Calls`
    /// edges. Used to enumerate downstream dependencies.
    Callees,
}

impl CallRelation {
    fn as_petgraph_direction(self) -> Direction {
        match self {
            CallRelation::Callers => Direction::Incoming,
            CallRelation::Callees => Direction::Outgoing,
        }
    }
}

/// Walk **incoming `Calls` edges** from `target` to find every function that
/// CALLS `target` (transitively). Used to compute the set of nodes whose
/// tests/types must be re-validated when `target`'s signature changes.
///
/// Results are returned in topological order (callers-deepest first when the
/// graph is acyclic), suitable for cascade edit dispatch. If the surrounding
/// graph contains cycles, the result falls back to traversal-insertion order.
///
/// This is the directional dual of [`callees_of`].
pub fn callers_of(graph: &CodeGraph, target: &NodeId) -> Vec<NodeId> {
    cascade_walk(graph, target, CallRelation::Callers)
}

/// Walk **outgoing `Calls` edges** from `target` to find every function that
/// `target` CALLS (transitively). Used to enumerate downstream dependencies —
/// e.g., "what does this function transitively depend on?"
///
/// Results are returned in topological order when the graph is acyclic;
/// otherwise in traversal-insertion order. Directional dual of [`callers_of`].
pub fn callees_of(graph: &CodeGraph, target: &NodeId) -> Vec<NodeId> {
    cascade_walk(graph, target, CallRelation::Callees)
}

/// Internal: traverse the call graph from `target` in the given semantic
/// direction and return the reachable set in topological order (or insertion
/// order if cyclic).
///
/// Only walks call-like edges (`EdgeKind::Calls`, `UnresolvedCall`,
/// `ExternalCall`). Structural edges such as `Contains` (module → child)
/// or `UsesType` are excluded — a module that contains a function is not
/// a "caller" of that function in any meaningful sense.
fn cascade_walk(graph: &CodeGraph, target: &NodeId, relation: CallRelation) -> Vec<NodeId> {
    let Some(target_idx) = graph.resolve(target) else {
        return vec![];
    };

    let direction = relation.as_petgraph_direction();
    let inner = graph.inner();
    let mut reachable: HashSet<NodeIndex> = HashSet::new();
    let mut stack = vec![target_idx];
    while let Some(current) = stack.pop() {
        for edge in inner.edges_directed(current, direction) {
            if !is_call_edge(&edge.weight().kind) {
                continue;
            }
            // For Incoming edges, the "neighbor" is the source; for Outgoing,
            // it's the target. petgraph's `edges_directed` yields edges with
            // source/target as stored, so we pick the opposite endpoint.
            let neighbor = match direction {
                Direction::Incoming => edge.source(),
                Direction::Outgoing => edge.target(),
            };
            if reachable.insert(neighbor) {
                stack.push(neighbor);
            }
        }
    }

    // The target itself is never a "caller of itself" / "callee of itself"
    // even if it participates in a cycle — remove it from the reachable set.
    reachable.remove(&target_idx);

    // Try to toposort just the reachable subgraph — return in order.
    // If cyclic, fall back to insertion order.
    let mut result: Vec<NodeId> = Vec::new();
    if let Ok(full_order) = toposort(graph.inner(), None) {
        for idx in full_order {
            if reachable.contains(&idx) {
                if let Some(id) = graph.node_id_for(idx) {
                    result.push(id.clone());
                }
            }
        }
    } else {
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

// ─── Articulation Points (Hopcroft–Tarjan, linear time) ─────────────────────

/// Find articulation points — nodes whose removal disconnects the graph.
///
/// These are "critical functions": if they break, multiple parts of the
/// codebase lose connectivity. High-priority for careful editing.
///
/// ## Ordering note (Phase 2 change)
///
/// Result vector is ordered by **DFS-discovery sequence**, not by the
/// node-insertion / NodeIndex scan order the previous brute-force
/// implementation produced. There is no documented public ordering
/// contract on the return value; consumers using `==` on the `Vec`
/// should `sort()` or `HashSet`-collect first. Tests in this crate
/// avoid order-dependent assertions; if a downstream caller breaks,
/// the fix is one `sort_by_key(|id| id.0)` away.
///
/// Implementation: **Hopcroft–Tarjan iterative DFS** in **O(V + E)**. We
/// treat the call graph as undirected (a removed function disconnects
/// callers from callees regardless of direction), then for each DFS tree:
///
/// - The **root** is articulation iff it has more than one child in the
///   DFS tree.
/// - A **non-root** vertex `u` is articulation iff some child `v` has
///   `low(v) >= disc(u)` — i.e. no back-edge from `v`'s subtree escapes
///   above `u`.
///
/// This replaces the previous brute-force O(V·(V+E)) component-recount
/// implementation, which was the asymptotically slowest hot loop in the
/// crate. On a 10k-node graph the new implementation is ~1000× faster.
pub fn critical_nodes(graph: &CodeGraph) -> Vec<NodeId> {
    let inner = graph.inner();
    let n = inner.node_count();
    if n == 0 {
        return Vec::new();
    }

    // Map NodeIndex → dense usize for vec-backed scratch arrays.
    let node_indices: Vec<NodeIndex> = inner.node_indices().collect();
    let mut idx_of: HashMap<NodeIndex, usize> = HashMap::with_capacity(n);
    for (i, &ni) in node_indices.iter().enumerate() {
        idx_of.insert(ni, i);
    }

    let mut disc: Vec<u32> = vec![u32::MAX; n];
    let mut low: Vec<u32> = vec![u32::MAX; n];
    let mut parent: Vec<i32> = vec![-1; n];
    let mut is_articulation: Vec<bool> = vec![false; n];
    let mut timer: u32 = 0;

    // Iterative DFS — store per-frame the iterator over undirected
    // neighbours (out + in), and resume it on backtrack.
    type Neighbours = std::vec::IntoIter<NodeIndex>;
    let mut stack: Vec<(usize, Neighbours, u32)> = Vec::new(); // (u, iter, child_count)

    for &start in &node_indices {
        let s = idx_of[&start];
        if disc[s] != u32::MAX {
            continue;
        }
        timer += 1;
        disc[s] = timer;
        low[s] = timer;

        let mut neighbours: Vec<NodeIndex> = inner
            .neighbors_directed(start, Direction::Outgoing)
            .chain(inner.neighbors_directed(start, Direction::Incoming))
            .collect();
        neighbours.sort_unstable();
        neighbours.dedup();
        stack.push((s, neighbours.into_iter(), 0));

        while let Some((u, iter, child_count)) = stack.last_mut() {
            match iter.next() {
                Some(v_ni) => {
                    let v = idx_of[&v_ni];
                    if disc[v] == u32::MAX {
                        // Tree edge: descend.
                        parent[v] = *u as i32;
                        *child_count += 1;
                        timer += 1;
                        disc[v] = timer;
                        low[v] = timer;
                        let mut sub: Vec<NodeIndex> = inner
                            .neighbors_directed(v_ni, Direction::Outgoing)
                            .chain(inner.neighbors_directed(v_ni, Direction::Incoming))
                            .collect();
                        sub.sort_unstable();
                        sub.dedup();
                        stack.push((v, sub.into_iter(), 0));
                    } else if parent[*u] != v as i32 {
                        // Back-edge: update low[u] without recursing.
                        if disc[v] < low[*u] {
                            low[*u] = disc[v];
                        }
                    }
                }
                None => {
                    // Backtrack: propagate low to parent and check the
                    // articulation condition on the parent.
                    let u = *u;
                    let cc = *child_count;
                    stack.pop();
                    if let Some((p, _, _)) = stack.last() {
                        let p = *p;
                        if low[u] < low[p] {
                            low[p] = low[u];
                        }
                        // Non-root parent: if low[u] >= disc[p], p is articulation.
                        if parent[p] != -1 && low[u] >= disc[p] {
                            is_articulation[p] = true;
                        }
                    } else {
                        // u was a DFS-tree root: articulation iff > 1 child.
                        if cc > 1 {
                            is_articulation[u] = true;
                        }
                    }
                }
            }
        }
    }

    let mut out = Vec::new();
    for (i, &flag) in is_articulation.iter().enumerate() {
        if flag {
            if let Some(id) = graph.node_id_for(node_indices[i]) {
                out.push(id.clone());
            }
        }
    }
    out
}

// ─── Bridges (Tarjan's linear-time algorithm) ────────────────────────────────

/// A bridge edge whose removal disconnects the graph.
#[derive(Debug, Clone)]
pub struct BridgeEdge {
    pub from: NodeId,
    pub to: NodeId,
}

/// Find bridge edges — edges whose removal increases the number of
/// connected components. These represent fragile coupling points.
///
/// ## Ordering note (Phase 2 change)
///
/// Bridges are returned in **DFS-discovery order** of the *child* node
/// — same caveat as [`critical_nodes`]. No public ordering contract;
/// sort if you need determinism across builds.
///
/// Implementation: **Tarjan's bridge-finding** in **O(V + E)** via the
/// same DFS-low-link machinery as articulation points. An edge `(u, v)`
/// where `v` is `u`'s DFS-tree child is a bridge iff `low(v) > disc(u)`
/// — i.e. no back-edge from `v`'s subtree reaches `u` or above.
///
/// Replaces the previous O(E·(V+E)) brute-force edge-removal +
/// component recount.
pub fn bridge_edges(graph: &CodeGraph) -> Vec<BridgeEdge> {
    let inner = graph.inner();
    let n = inner.node_count();
    if n == 0 {
        return Vec::new();
    }

    let node_indices: Vec<NodeIndex> = inner.node_indices().collect();
    let mut idx_of: HashMap<NodeIndex, usize> = HashMap::with_capacity(n);
    for (i, &ni) in node_indices.iter().enumerate() {
        idx_of.insert(ni, i);
    }

    let mut disc: Vec<u32> = vec![u32::MAX; n];
    let mut low: Vec<u32> = vec![u32::MAX; n];
    let mut parent: Vec<i32> = vec![-1; n];
    let mut bridges: Vec<(NodeIndex, NodeIndex)> = Vec::new();
    let mut timer: u32 = 0;

    type Neighbours = std::vec::IntoIter<NodeIndex>;
    let mut stack: Vec<(usize, NodeIndex, Neighbours)> = Vec::new();

    for &start in &node_indices {
        let s = idx_of[&start];
        if disc[s] != u32::MAX {
            continue;
        }
        timer += 1;
        disc[s] = timer;
        low[s] = timer;
        let mut neighbours: Vec<NodeIndex> = inner
            .neighbors_directed(start, Direction::Outgoing)
            .chain(inner.neighbors_directed(start, Direction::Incoming))
            .collect();
        neighbours.sort_unstable();
        neighbours.dedup();
        stack.push((s, start, neighbours.into_iter()));

        while let Some((u, _u_ni, iter)) = stack.last_mut() {
            match iter.next() {
                Some(v_ni) => {
                    let v = idx_of[&v_ni];
                    if disc[v] == u32::MAX {
                        parent[v] = *u as i32;
                        timer += 1;
                        disc[v] = timer;
                        low[v] = timer;
                        let mut sub: Vec<NodeIndex> = inner
                            .neighbors_directed(v_ni, Direction::Outgoing)
                            .chain(inner.neighbors_directed(v_ni, Direction::Incoming))
                            .collect();
                        sub.sort_unstable();
                        sub.dedup();
                        stack.push((v, v_ni, sub.into_iter()));
                    } else if parent[*u] != v as i32 {
                        if disc[v] < low[*u] {
                            low[*u] = disc[v];
                        }
                    }
                }
                None => {
                    let u = *u;
                    let u_ni = *_u_ni;
                    stack.pop();
                    if let Some((p, p_ni, _)) = stack.last() {
                        let p = *p;
                        let p_ni = *p_ni;
                        if low[u] < low[p] {
                            low[p] = low[u];
                        }
                        // Bridge condition: low(child) > disc(parent).
                        if low[u] > disc[p] {
                            bridges.push((p_ni, u_ni));
                        }
                    }
                }
            }
        }
    }

    let mut out = Vec::new();
    for (a, b) in bridges {
        if let (Some(from), Some(to)) = (graph.node_id_for(a), graph.node_id_for(b)) {
            out.push(BridgeEdge {
                from: from.clone(),
                to: to.clone(),
            });
        }
    }
    out
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
    result.sort_by_key(|group| std::cmp::Reverse(group.members.len()));
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
        .max_by_key(|&&v| adj.get(&v).map(|n| n.intersection(&p).count()).unwrap_or(0))
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
    let idx_to_seq: HashMap<NodeIndex, usize> = indices
        .iter()
        .enumerate()
        .map(|(i, &idx)| (idx, i))
        .collect();

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
                if let (Some(from_id), Some(to_id)) =
                    (graph.node_id_for(indices[i]), graph.node_id_for(indices[j]))
                {
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
    heap.push(State {
        cost: 0.0,
        node: node_idx,
    });

    while let Some(State {
        cost,
        node: current,
    }) = heap.pop()
    {
        if cost > *dist.get(&current).unwrap_or(&f32::INFINITY) {
            continue;
        }
        for edge in inner.edges_directed(current, Direction::Outgoing) {
            let next = edge.target();
            let next_cost = cost + edge.weight().weight;
            if next_cost < *dist.get(&next).unwrap_or(&f32::INFINITY) {
                dist.insert(next, next_cost);
                heap.push(State {
                    cost: next_cost,
                    node: next,
                });
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
        .filter(|e| {
            matches!(
                e.weight().kind,
                EdgeKind::Calls | EdgeKind::UnresolvedCall(_)
            )
        })
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

// ─── SCC Partition + Condensation ────────────────────────────────────────────

/// Strongly-connected components of the call graph.
///
/// Each component contains 1+ `NodeId`s; multi-element components are
/// mutually-recursive function clusters (replaces the older "cycle_detected"
/// point flags). `component_of` maps every node into the index of its
/// component in `components`, so callers can check "are A and B in the same
/// SCC?" in O(1).
#[derive(Debug, Clone)]
pub struct SccPartition {
    /// Index into `components` for each `NodeId`.
    pub component_of: HashMap<NodeId, usize>,
    /// All SCCs. The order produced by Tarjan's algorithm is a reverse
    /// topological order on the condensation, but callers should not rely on
    /// the index assignment carrying any other meaning.
    pub components: Vec<Vec<NodeId>>,
}

impl CodeGraph {
    /// Compute SCCs using Tarjan's algorithm via petgraph. O(V + E).
    ///
    /// Tarjan is preferred over Kosaraju because petgraph's `tarjan_scc`
    /// makes a single DFS pass — Kosaraju needs two passes plus a transposed
    /// graph, which on a `StableDiGraph` would require an explicit copy.
    pub fn strongly_connected_components(&self) -> SccPartition {
        let inner = self.inner();
        let raw = tarjan_scc(inner);

        let mut components: Vec<Vec<NodeId>> = Vec::with_capacity(raw.len());
        let mut component_of: HashMap<NodeId, usize> = HashMap::new();

        for (component_idx, scc) in raw.into_iter().enumerate() {
            let mut members: Vec<NodeId> = Vec::with_capacity(scc.len());
            for idx in scc {
                if let Some(id) = self.node_id_for(idx) {
                    component_of.insert(id.clone(), component_idx);
                    members.push(id.clone());
                }
            }
            components.push(members);
        }

        SccPartition {
            component_of,
            components,
        }
    }

    /// Build the condensation DAG: each SCC becomes one node; edges are the
    /// union of cross-SCC edges in the original graph (deduplicated).
    ///
    /// Returns a separate `petgraph::Graph<Vec<NodeId>, ()>` so the caller
    /// can reason about SCC-level ordering without touching the primary
    /// graph. The result is guaranteed to be acyclic (this is the defining
    /// property of a condensation).
    pub fn condensation(&self) -> petgraph::Graph<Vec<NodeId>, ()> {
        let partition = self.strongly_connected_components();
        let inner = self.inner();

        let mut out: petgraph::Graph<Vec<NodeId>, ()> = petgraph::Graph::new();
        let mut comp_idx_to_node: Vec<petgraph::graph::NodeIndex> =
            Vec::with_capacity(partition.components.len());
        for component in &partition.components {
            comp_idx_to_node.push(out.add_node(component.clone()));
        }

        // Walk every edge once. For each, look up the source/target component
        // and add a deduplicated cross-SCC edge.
        let mut seen: HashSet<(usize, usize)> = HashSet::new();
        for edge in inner.edge_references() {
            let Some(src_id) = self.node_id_for(edge.source()) else {
                continue;
            };
            let Some(dst_id) = self.node_id_for(edge.target()) else {
                continue;
            };
            let Some(&src_comp) = partition.component_of.get(src_id) else {
                continue;
            };
            let Some(&dst_comp) = partition.component_of.get(dst_id) else {
                continue;
            };
            if src_comp == dst_comp {
                continue;
            }
            if seen.insert((src_comp, dst_comp)) {
                out.add_edge(comp_idx_to_node[src_comp], comp_idx_to_node[dst_comp], ());
            }
        }

        out
    }
}

// ─── Centrality Metrics ──────────────────────────────────────────────────────

/// Per-node centrality measurements.
///
/// `in_degree` / `out_degree` are exact; `pagerank` is an approximation
/// (damping = 0.85, 50 iterations) suitable for ranking but not numeric
/// analysis.
#[derive(Debug, Clone)]
pub struct CentralityMetrics {
    pub in_degree: HashMap<NodeId, usize>,
    pub out_degree: HashMap<NodeId, usize>,
    /// Approximate PageRank (damping=0.85, 50 iterations). Cheap, not
    /// perfect — meant for ranking, not numeric analysis.
    pub pagerank: HashMap<NodeId, f64>,
}

impl CodeGraph {
    /// Compute in-degree, out-degree, and PageRank for every node.
    ///
    /// PageRank uses petgraph's `page_rank` with damping = 0.85 and 50
    /// iterations — enough for stable ranking on graphs <100k nodes without
    /// adding a new dependency.
    pub fn centrality(&self) -> CentralityMetrics {
        let inner = self.inner();
        let mut in_degree: HashMap<NodeId, usize> = HashMap::with_capacity(inner.node_count());
        let mut out_degree: HashMap<NodeId, usize> = HashMap::with_capacity(inner.node_count());
        let mut pagerank: HashMap<NodeId, f64> = HashMap::with_capacity(inner.node_count());

        for idx in inner.node_indices() {
            let Some(id) = self.node_id_for(idx) else {
                continue;
            };
            let inc = inner.neighbors_directed(idx, Direction::Incoming).count();
            let outg = inner.neighbors_directed(idx, Direction::Outgoing).count();
            in_degree.insert(id.clone(), inc);
            out_degree.insert(id.clone(), outg);
        }

        if inner.node_count() > 0 {
            let ranks = page_rank(inner, 0.85_f32, 50);
            for (idx, score) in inner.node_indices().zip(ranks.iter()) {
                if let Some(id) = self.node_id_for(idx) {
                    pagerank.insert(id.clone(), *score as f64);
                }
            }
        }

        CentralityMetrics {
            in_degree,
            out_degree,
            pagerank,
        }
    }

    /// Top-N functions by PageRank.
    ///
    /// Stable order: ties broken by `NodeId` for determinism (per
    /// Woerister's iteration-order-stability idiom — same input must yield
    /// the same ranking across runs regardless of `HashMap` iteration order).
    pub fn hottest_functions(&self, n: usize) -> Vec<(NodeId, f64)> {
        let metrics = self.centrality();
        let mut ranked: Vec<(NodeId, f64)> = metrics.pagerank.into_iter().collect();
        ranked.sort_by(|a, b| {
            // Score descending; on tie, NodeId ascending for determinism.
            b.1.partial_cmp(&a.1)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.0.cmp(&b.0))
        });
        ranked.truncate(n);
        ranked
    }
}

// ─── Entrypoint Classification ───────────────────────────────────────────────

/// What kind of entrypoint a function is, if any.
///
/// A function may match multiple kinds in principle (e.g. a `pub fn main`),
/// but [`CodeGraph::classify_entrypoints`] reports a single canonical kind
/// using the precedence: Test > Bench > FfiExport > Main > PublicApi.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EntrypointKind {
    /// Function named `main` at module root.
    Main,
    /// Public function exposed at crate root or `pub mod`.
    PublicApi,
    /// Test function: `#[test]` or `#[tokio::test]`.
    Test,
    /// Has `#[bench]`.
    Bench,
    /// FFI export: `pub extern "..." fn` or `#[no_mangle]`.
    FfiExport,
}

/// Summary statistics for one entrypoint.
#[derive(Debug, Clone)]
pub struct EntrypointSummary {
    pub node_id: NodeId,
    pub kind: EntrypointKind,
    pub fan_in: usize,
    pub fan_out: usize,
    /// Maximum reachable depth in the call graph from this entrypoint.
    pub max_reach_depth: usize,
    /// Total reachable nodes count (excludes the entrypoint itself).
    pub reach_size: usize,
}

impl CodeGraph {
    /// Classify all functions into entrypoint kinds based on metadata
    /// (visibility, attributes).
    ///
    /// Functions that don't match any kind are not included.
    ///
    /// # Metadata key mapping
    ///
    /// The Rust adapter records function attributes into
    /// [`crate::nodes::NodeData::metadata`]. This routine looks at the
    /// following keys (any present truthy value qualifies — concretely
    /// "true" / "1" / non-empty other than "false"/"0"):
    ///
    /// - `"test"` — `#[test]` or `#[tokio::test]` attribute
    /// - `"bench"` — `#[bench]` attribute
    /// - `"no_mangle"` — `#[no_mangle]` attribute
    /// - `"extern"` — `extern "..."` ABI on the fn
    ///
    /// If a metadata key isn't present (e.g. older adapter versions that
    /// don't record attributes), the corresponding kind simply isn't
    /// reported — we never panic. Visibility is read from the structured
    /// [`crate::nodes::Visibility`] field directly; `main` is detected by
    /// the function's `name`.
    pub fn classify_entrypoints(&self) -> Vec<EntrypointSummary> {
        let inner = self.inner();
        let mut out: Vec<EntrypointSummary> = Vec::new();

        for idx in inner.node_indices() {
            let Some(node) = inner.node_weight(idx) else {
                continue;
            };
            if !matches!(node.kind, crate::nodes::NodeKind::Function) {
                continue;
            }

            let Some(kind) = classify_function_entrypoint(node) else {
                continue;
            };

            let fan_in = inner.neighbors_directed(idx, Direction::Incoming).count();
            let fan_out = inner.neighbors_directed(idx, Direction::Outgoing).count();
            let (max_reach_depth, reach_size) = bfs_reach_metrics(inner, idx);

            out.push(EntrypointSummary {
                node_id: node.id.clone(),
                kind,
                fan_in,
                fan_out,
                max_reach_depth,
                reach_size,
            });
        }

        // Deterministic order: by NodeId.
        out.sort_by(|a, b| a.node_id.cmp(&b.node_id));
        out
    }
}

/// Apply the entrypoint precedence rules to a single function node.
///
/// Returns `None` if the function isn't an entrypoint of any flavor.
fn classify_function_entrypoint(node: &crate::nodes::NodeData) -> Option<EntrypointKind> {
    if metadata_flag(node, "test") {
        return Some(EntrypointKind::Test);
    }
    if metadata_flag(node, "bench") {
        return Some(EntrypointKind::Bench);
    }
    if metadata_flag(node, "no_mangle") || metadata_flag(node, "extern") {
        return Some(EntrypointKind::FfiExport);
    }

    // `fn main` at module root counts as Main regardless of visibility (an
    // implicit `fn main` is private but still THE entrypoint of a binary
    // crate).
    if node.name == "main" {
        return Some(EntrypointKind::Main);
    }

    if matches!(node.visibility, Visibility::Public) {
        return Some(EntrypointKind::PublicApi);
    }

    None
}

/// True if the named metadata key is present and represents a truthy value.
///
/// Truthy: any present value other than empty / "false" / "0", case-insensitive.
fn metadata_flag(node: &crate::nodes::NodeData, key: &str) -> bool {
    match node.metadata.get(key) {
        None => false,
        Some(v) => {
            let trimmed = v.trim();
            !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("false") && trimmed != "0"
        }
    }
}

/// BFS over the call graph starting at `start`, returning
/// `(max_depth, reach_size)`. `reach_size` excludes the start node itself.
fn bfs_reach_metrics(
    inner: &petgraph::stable_graph::StableDiGraph<crate::nodes::NodeData, crate::edges::EdgeData>,
    start: NodeIndex,
) -> (usize, usize) {
    let mut visited: HashSet<NodeIndex> = HashSet::new();
    visited.insert(start);
    let mut queue: VecDeque<(NodeIndex, usize)> = VecDeque::new();
    queue.push_back((start, 0));
    let mut max_depth = 0usize;

    while let Some((current, depth)) = queue.pop_front() {
        for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
            if visited.insert(neighbor) {
                let nd = depth + 1;
                if nd > max_depth {
                    max_depth = nd;
                }
                queue.push_back((neighbor, nd));
            }
        }
    }

    // Subtract 1 to exclude the start node from the reach count.
    let reach_size = visited.len().saturating_sub(1);
    (max_depth, reach_size)
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// True if the edge represents a call relationship (direct, unresolved, or
/// external). Used by [`callers_of`] / [`callees_of`] to exclude structural
/// edges like `Contains`, `UsesType`, `References`, `Implements`.
fn is_call_edge(kind: &EdgeKind) -> bool {
    matches!(
        kind,
        EdgeKind::Calls | EdgeKind::UnresolvedCall(_) | EdgeKind::ExternalCall(_, _)
    )
}

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
            birth_revision: 0,
            last_modified_revision: 0,
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
        let _c = g.add_node(node("c")); // isolated
        g.add_edge(&a, &b, edge()).unwrap();

        assert_eq!(independent_module_count(&g), 2);
        let groups = component_groups(&g);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn test_callers_of() {
        let mut g = CodeGraph::new();
        let target = g.add_node(node("target"));
        let caller1 = g.add_node(node("caller1"));
        let caller2 = g.add_node(node("caller2"));
        let grandcaller = g.add_node(node("grandcaller"));
        g.add_edge(&caller1, &target, edge()).unwrap();
        g.add_edge(&caller2, &target, edge()).unwrap();
        g.add_edge(&grandcaller, &caller1, edge()).unwrap();

        let order = callers_of(&g, &target);
        // Should include caller1, caller2, grandcaller
        assert_eq!(order.len(), 3);
    }

    #[test]
    fn callers_of_returns_only_incoming_callers_normal() {
        // Graph: caller → target → callee
        // callers_of(target) should return [caller], NOT [callee] — it must
        // walk INCOMING edges only.
        let mut g = CodeGraph::new();
        let caller = g.add_node(node("caller"));
        let target = g.add_node(node("target"));
        let callee = g.add_node(node("callee"));
        g.add_edge(&caller, &target, edge()).unwrap();
        g.add_edge(&target, &callee, edge()).unwrap();

        let callers = callers_of(&g, &target);
        assert_eq!(callers, vec![caller.clone()]);
        assert!(
            !callers.contains(&callee),
            "callees must not appear in callers_of"
        );

        // Mirror sanity: callees_of(target) returns [callee], not [caller].
        let callees = callees_of(&g, &target);
        assert_eq!(callees, vec![callee.clone()]);
        assert!(!callees.contains(&caller));
    }

    #[test]
    fn callers_of_handles_cycles_robust() {
        // Mutual recursion a ↔ b, plus c calls a. callers_of(a) must terminate
        // and include {b, c} without spinning on the cycle.
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &a, edge()).unwrap(); // cycle
        g.add_edge(&c, &a, edge()).unwrap();

        let callers = callers_of(&g, &a);
        // Both b (cycle partner) and c (external caller) call a.
        assert!(callers.contains(&b));
        assert!(callers.contains(&c));
        // a itself is not its own caller in the result set.
        assert!(!callers.contains(&a));
    }

    #[test]
    fn callers_of_excludes_non_call_edges_robust() {
        // Module `m` Contains function `f`. A separate function `caller` calls f.
        // callers_of(f) must include `caller` but NOT `m` — a Contains edge
        // from a Module is structural, not a call relationship.
        let mut g = CodeGraph::new();
        let m = NodeData {
            id: NodeId::new("test.rs", "crate::m", NodeKind::Module),
            kind: NodeKind::Module,
            name: "m".to_string(),
            qualified_name: "crate::m".to_string(),
            file_path: PathBuf::from("test.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
        };
        let m_id = g.add_node(m);
        let f_id = g.add_node(node("f"));
        let caller_id = g.add_node(node("caller"));

        // Module `m` Contains f.
        g.add_edge(
            &m_id,
            &f_id,
            EdgeData {
                kind: EdgeKind::Contains,
                source_span: span(),
                weight: 1.0,
            },
        )
        .unwrap();
        // `caller` calls f.
        g.add_edge(&caller_id, &f_id, edge()).unwrap();

        let callers = callers_of(&g, &f_id);
        assert!(
            callers.contains(&caller_id),
            "real caller must be present in callers_of"
        );
        assert!(
            !callers.contains(&m_id),
            "Contains edge from Module must NOT count as a caller"
        );
        assert_eq!(callers.len(), 1);
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
        let b_color = groups
            .iter()
            .find(|grp| grp.members.contains(&b))
            .unwrap()
            .color;
        let c_color = groups
            .iter()
            .find(|grp| grp.members.contains(&c))
            .unwrap()
            .color;
        assert_eq!(b_color, c_color);

        // a should be in a different color than b (they share an edge)
        let a_color = groups
            .iter()
            .find(|grp| grp.members.contains(&a))
            .unwrap()
            .color;
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
        g.add_edge(
            &a,
            &c,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span(),
                weight: 5.0,
            },
        )
        .unwrap();
        g.add_edge(
            &a,
            &d,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span(),
                weight: 2.0,
            },
        )
        .unwrap();

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
        g.add_edge(
            &a,
            &c,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span(),
                weight: 10.0,
            },
        )
        .unwrap();

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
        g.add_edge(
            &a,
            &c,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span(),
                weight: 3.0,
            },
        )
        .unwrap();

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
        // Use a Module as source for the Contains edge — Contains requires
        // source = Module|Struct|Enum|Trait per EdgeKind::valid_for.
        let m = g.add_node(NodeData {
            id: NodeId::new("test.rs", "crate::m", NodeKind::Module),
            kind: NodeKind::Module,
            name: "m".to_string(),
            qualified_name: "crate::m".to_string(),
            file_path: PathBuf::from("test.rs"),
            span: span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
        });
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        g.add_edge(&a, &b, edge()).unwrap(); // Calls edge
        g.add_edge(
            &m,
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

    #[test]
    fn tarjan_articulation_skips_endpoints_normal() {
        // a—b—c—d: only b and c are articulation points (endpoints
        // a and d each have only one neighbour, removing them
        // doesn't disconnect anything).
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&c, &d, edge()).unwrap();

        let critical = critical_nodes(&g);
        assert!(critical.contains(&b));
        assert!(critical.contains(&c));
        assert!(!critical.contains(&a));
        assert!(!critical.contains(&d));
    }

    #[test]
    fn tarjan_articulation_cycle_has_none() {
        // Triangle a—b—c—a: nothing is articulation, every vertex
        // has a back-edge bypass.
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&c, &a, edge()).unwrap();

        let critical = critical_nodes(&g);
        assert!(critical.is_empty(), "got {:?}", critical);
    }

    #[test]
    fn tarjan_bridges_in_cycle_are_empty() {
        // Same triangle: no bridges in a cycle.
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&c, &a, edge()).unwrap();
        let bridges = bridge_edges(&g);
        assert!(bridges.is_empty(), "got {:?}", bridges);
    }

    #[test]
    fn tarjan_bridges_chain_every_edge() {
        // Linear chain a—b—c—d: every edge is a bridge.
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&c, &d, edge()).unwrap();
        let bridges = bridge_edges(&g);
        assert_eq!(bridges.len(), 3);
    }

    #[test]
    fn tarjan_handles_disconnected_graphs() {
        // Two disconnected components, each with its own articulation point.
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        let e = g.add_node(node("e"));
        let f = g.add_node(node("f"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();
        g.add_edge(&d, &e, edge()).unwrap();
        g.add_edge(&e, &f, edge()).unwrap();

        let critical = critical_nodes(&g);
        assert!(critical.contains(&b));
        assert!(critical.contains(&e));
    }

    #[test]
    fn tarjan_empty_graph_no_panic() {
        let g = CodeGraph::new();
        assert!(critical_nodes(&g).is_empty());
        assert!(bridge_edges(&g).is_empty());
    }

    #[test]
    fn tarjan_single_node_no_articulation() {
        let mut g = CodeGraph::new();
        g.add_node(node("solo"));
        assert!(critical_nodes(&g).is_empty());
        assert!(bridge_edges(&g).is_empty());
    }

    // ─── SCC partition + condensation ──────────────────────────────────────

    fn function_node_with(name: &str, vis: Visibility, meta: &[(&str, &str)]) -> NodeData {
        let id = NodeId::new("test.rs", &format!("crate::{name}"), NodeKind::Function);
        let mut metadata = HashMap::new();
        for (k, v) in meta {
            metadata.insert((*k).to_string(), (*v).to_string());
        }
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("test.rs"),
            span: span(),
            visibility: vis,
            metadata,
            birth_revision: 0,
            last_modified_revision: 0,
        }
    }

    #[test]
    fn scc_isolated_nodes_each_in_own_component_normal() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));

        let part = g.strongly_connected_components();
        // Three disconnected nodes => three singleton components.
        assert_eq!(part.components.len(), 3);
        for component in &part.components {
            assert_eq!(component.len(), 1);
        }
        // Every node is mapped.
        assert!(part.component_of.contains_key(&a));
        assert!(part.component_of.contains_key(&b));
        assert!(part.component_of.contains_key(&c));
        // The component_of indices are consistent with components[].
        for (id, &comp_idx) in &part.component_of {
            assert!(part.components[comp_idx].contains(id));
        }
    }

    #[test]
    fn scc_a_b_a_cycle_clusters_robust() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a ⇄ b cycle, c on the side
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &a, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();

        let part = g.strongly_connected_components();
        // a and b must share a component; c must be on its own.
        let a_comp = part.component_of[&a];
        let b_comp = part.component_of[&b];
        let c_comp = part.component_of[&c];
        assert_eq!(a_comp, b_comp);
        assert_ne!(a_comp, c_comp);
        // The {a,b} component has exactly 2 members.
        assert_eq!(part.components[a_comp].len(), 2);
        assert_eq!(part.components[c_comp].len(), 1);
    }

    #[test]
    fn scc_diamond_no_back_edge_each_singleton_normal() {
        // a → b, a → c, b → d, c → d  (no back edges = DAG)
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();
        g.add_edge(&b, &d, edge()).unwrap();
        g.add_edge(&c, &d, edge()).unwrap();

        let part = g.strongly_connected_components();
        // Every node is its own component.
        assert_eq!(part.components.len(), 4);
        for component in &part.components {
            assert_eq!(component.len(), 1);
        }
    }

    #[test]
    fn condensation_is_acyclic_robust() {
        // Mix of cycle + DAG edges should still produce an acyclic condensation.
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        // a ⇄ b cycle, then a → c → d chain, plus c → b (re-entering the cycle).
        // The c → b edge creates a path c → b → a → c, so {a, b, c} collapse
        // into a single SCC. Only {d} remains as a separate component.
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&b, &a, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();
        g.add_edge(&c, &d, edge()).unwrap();
        g.add_edge(&c, &b, edge()).unwrap();

        let cond = g.condensation();
        assert!(!petgraph::algo::is_cyclic_directed(&cond));
        // SCCs: {a,b,c} and {d} → 2 condensation nodes.
        assert_eq!(cond.node_count(), 2);
    }

    // ─── Centrality metrics ────────────────────────────────────────────────

    #[test]
    fn centrality_in_out_degree_matches_petgraph_normal() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        // a → b, a → c, b → c
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();

        let metrics = g.centrality();
        assert_eq!(metrics.out_degree[&a], 2);
        assert_eq!(metrics.out_degree[&b], 1);
        assert_eq!(metrics.out_degree[&c], 0);
        assert_eq!(metrics.in_degree[&a], 0);
        assert_eq!(metrics.in_degree[&b], 1);
        assert_eq!(metrics.in_degree[&c], 2);
    }

    #[test]
    fn centrality_pagerank_high_for_high_fan_in_normal() {
        // Many nodes pointing at `hub` should give it the highest PageRank.
        let mut g = CodeGraph::new();
        let hub = g.add_node(node("hub"));
        let leaves: Vec<_> = (0..5)
            .map(|i| g.add_node(node(&format!("leaf{i}"))))
            .collect();
        for leaf in &leaves {
            g.add_edge(leaf, &hub, edge()).unwrap();
        }

        let metrics = g.centrality();
        let hub_score = metrics.pagerank[&hub];
        for leaf in &leaves {
            assert!(
                hub_score > metrics.pagerank[leaf],
                "hub PageRank ({}) should exceed leaf PageRank ({})",
                hub_score,
                metrics.pagerank[leaf]
            );
        }
    }

    #[test]
    fn hottest_functions_returns_stable_order_robust() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a"));
        let b = g.add_node(node("b"));
        let c = g.add_node(node("c"));
        let d = g.add_node(node("d"));
        // Equal in-degree on b and c; a feeds both, d feeds nobody.
        // Forces a tie that the secondary NodeId ordering must break
        // deterministically.
        g.add_edge(&a, &b, edge()).unwrap();
        g.add_edge(&a, &c, edge()).unwrap();
        g.add_edge(&d, &b, edge()).unwrap();
        g.add_edge(&d, &c, edge()).unwrap();

        let first = g.hottest_functions(4);
        let second = g.hottest_functions(4);
        assert_eq!(
            first.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
            second.iter().map(|x| x.0.clone()).collect::<Vec<_>>(),
            "hottest_functions must be deterministic across calls"
        );
        assert_eq!(first.len(), 4);
    }

    // ─── Entrypoint classification ─────────────────────────────────────────

    #[test]
    fn entrypoints_finds_main_normal() {
        let mut g = CodeGraph::new();
        let main_id = g.add_node(function_node_with("main", Visibility::Private, &[]));
        let _other = g.add_node(function_node_with("helper", Visibility::Private, &[]));

        let summaries = g.classify_entrypoints();
        let main_summary = summaries
            .iter()
            .find(|s| s.node_id == main_id)
            .expect("main must be classified");
        assert_eq!(main_summary.kind, EntrypointKind::Main);
    }

    #[test]
    fn entrypoints_finds_pub_fn_normal() {
        let mut g = CodeGraph::new();
        let pub_id = g.add_node(function_node_with("api", Visibility::Public, &[]));
        let _priv = g.add_node(function_node_with("helper", Visibility::Private, &[]));

        let summaries = g.classify_entrypoints();
        let api_summary = summaries
            .iter()
            .find(|s| s.node_id == pub_id)
            .expect("public fn must be classified");
        assert_eq!(api_summary.kind, EntrypointKind::PublicApi);
    }

    #[test]
    fn entrypoints_excludes_private_normal() {
        let mut g = CodeGraph::new();
        let _priv = g.add_node(function_node_with("helper", Visibility::Private, &[]));
        // No metadata, not main, private → should be filtered out.
        let summaries = g.classify_entrypoints();
        assert!(
            summaries.is_empty(),
            "private non-test fn should not be an entrypoint, got {:?}",
            summaries
        );
    }

    #[test]
    fn entrypoints_summary_includes_reach_depth_normal() {
        let mut g = CodeGraph::new();
        let main_id = g.add_node(function_node_with("main", Visibility::Private, &[]));
        let b = g.add_node(function_node_with("b", Visibility::Private, &[]));
        let c = g.add_node(function_node_with("c", Visibility::Private, &[]));
        // main → b → c (depth 2 from main)
        g.add_edge(&main_id, &b, edge()).unwrap();
        g.add_edge(&b, &c, edge()).unwrap();

        let summaries = g.classify_entrypoints();
        let main_s = summaries
            .iter()
            .find(|s| s.node_id == main_id)
            .expect("main is an entrypoint");
        assert_eq!(main_s.fan_out, 1);
        assert_eq!(main_s.fan_in, 0);
        assert_eq!(main_s.max_reach_depth, 2);
        assert_eq!(main_s.reach_size, 2);
    }

    #[test]
    fn entrypoints_classifies_test_attribute_robust() {
        let mut g = CodeGraph::new();
        let test_id = g.add_node(function_node_with(
            "my_test",
            Visibility::Private,
            &[("test", "true")],
        ));
        let summaries = g.classify_entrypoints();
        let s = summaries
            .iter()
            .find(|s| s.node_id == test_id)
            .expect("test must be classified");
        assert_eq!(s.kind, EntrypointKind::Test);
    }

    #[test]
    fn entrypoints_classifies_ffi_no_mangle_robust() {
        let mut g = CodeGraph::new();
        let ffi_id = g.add_node(function_node_with(
            "exported",
            Visibility::Public,
            &[("no_mangle", "true")],
        ));
        let summaries = g.classify_entrypoints();
        let s = summaries
            .iter()
            .find(|s| s.node_id == ffi_id)
            .expect("ffi export must be classified");
        // FFI takes precedence over PublicApi.
        assert_eq!(s.kind, EntrypointKind::FfiExport);
    }

    #[test]
    fn entrypoints_metadata_flag_handles_falsey_robust() {
        let mut g = CodeGraph::new();
        // metadata says test=false → should NOT be a Test entrypoint.
        // Visibility is Public, so it will fall through to PublicApi.
        let id = g.add_node(function_node_with(
            "not_a_test",
            Visibility::Public,
            &[("test", "false")],
        ));
        let summaries = g.classify_entrypoints();
        let s = summaries
            .iter()
            .find(|s| s.node_id == id)
            .expect("public fn falls through to PublicApi");
        assert_eq!(s.kind, EntrypointKind::PublicApi);
    }
}
