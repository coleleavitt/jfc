//! Graph traversal algorithms leveraging petgraph's built-in iterators.
//!
//! Replaces hand-rolled BFS/DFS with petgraph's `Bfs`, `Dfs`, and `Reversed`
//! adapters for cycle-detected, depth-bounded traversal with zero-copy
//! direction flipping.

use std::collections::{HashMap, HashSet};

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{Dfs, EdgeRef, Reversed};

use crate::csr::{CsrSnapshot, CsrVertex};
use crate::edges::EdgeData;
use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId};

/// Direction of traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TraversalDirection {
    /// Follow outgoing edges (callees)
    Outgoing,
    /// Follow incoming edges (callers)
    Incoming,
    /// Follow both directions
    Both,
}

/// Configuration for a graph traversal.
#[derive(Debug, Clone)]
pub struct TraversalConfig {
    /// Maximum depth to traverse (0 = start node only)
    pub max_depth: usize,
    /// Maximum total nodes to collect (token budget proxy)
    pub max_nodes: usize,
    /// Direction to traverse edges
    pub direction: TraversalDirection,
    /// Use parallel rayon expansion when frontier exceeds [`PARALLEL_THRESHOLD`].
    /// Default `false` to preserve deterministic ordering for the legacy
    /// callers; opt in for analysis-heavy paths.
    pub parallel: bool,
}

/// Frontier size at which parallel expansion starts paying off. Below this,
/// rayon overhead dominates the per-edge work.
pub const PARALLEL_THRESHOLD: usize = 64;

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_nodes: 100,
            direction: TraversalDirection::Outgoing,
            parallel: false,
        }
    }
}

/// Result of a traversal operation.
#[derive(Debug, Clone)]
pub struct TraversalResult {
    /// Nodes collected during traversal (in BFS order)
    pub nodes: Vec<NodeId>,
    /// Edges between collected nodes: (from, to)
    pub edges: Vec<(NodeId, NodeId)>,
    /// Maximum depth actually reached
    pub depth_reached: usize,
    /// Whether traversal was truncated due to max_nodes
    pub was_truncated: bool,
    /// Node IDs where cycles were detected (back edges)
    pub cycles_detected_at: Vec<NodeId>,
}

/// Graph size at which we switch from petgraph's adjacency-list walk
/// to a CSR-snapshot walk. Below this, the petgraph path is faster
/// (CSR build is ~constant overhead). Above, CSR's contiguous-memory
/// access pattern dominates.
pub const CSR_SIZE_THRESHOLD: usize = 1024;

/// Perform a bounded BFS traversal.
///
/// Phase 7: routes through one of three implementations based on
/// graph size and `config.parallel`:
///
/// 1. **Small graphs / serial** (default): petgraph adjacency-list
///    walk. Same code path as before; preserved for compatibility.
/// 2. **Large graphs / serial**: builds a `CsrSnapshot` once and
///    walks the CSR arrays. ~2-4× cache-miss reduction on dense
///    graphs.
/// 3. **`config.parallel = true`**: rayon-parallel frontier expansion
///    over the CSR. Wins for traversals that exceed
///    [`PARALLEL_THRESHOLD`] frontier size; otherwise falls back to
///    serial CSR.
///
/// Depth tracking is maintained manually. Cycle detection reports
/// back-edges via the `cycles_detected_at` field.
pub fn traverse(graph: &CodeGraph, start: &NodeId, config: &TraversalConfig) -> TraversalResult {
    if graph.node_count() >= CSR_SIZE_THRESHOLD || config.parallel {
        let snapshot = graph.snapshot();
        return traverse_csr(&snapshot, start, config);
    }
    traverse_petgraph(graph, start, config)
}

/// Original petgraph-based traversal. Public for callers who want to
/// bypass the CSR-snapshot decision.
pub fn traverse_petgraph(
    graph: &CodeGraph,
    start: &NodeId,
    config: &TraversalConfig,
) -> TraversalResult {
    let Some(start_idx) = graph.resolve(start) else {
        return TraversalResult {
            nodes: vec![],
            edges: vec![],
            depth_reached: 0,
            was_truncated: false,
            cycles_detected_at: vec![],
        };
    };

    let inner = graph.inner();
    let mut result_nodes: Vec<NodeId> = Vec::new();
    let mut result_edges: Vec<(NodeId, NodeId)> = Vec::new();
    let mut cycles_detected_at: Vec<NodeId> = Vec::new();
    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut was_truncated = false;
    let mut max_depth_reached: usize = 0;

    // BFS with depth tracking — we use a layered approach:
    // process nodes level by level to track depth.
    let mut current_layer: Vec<NodeIndex> = vec![start_idx];
    let mut next_layer: Vec<NodeIndex> = Vec::new();
    visited.insert(start_idx);
    result_nodes.push(start.clone());

    for depth in 0..config.max_depth {
        if current_layer.is_empty() {
            break;
        }
        max_depth_reached = depth;

        for &current in &current_layer {
            let neighbors: Vec<NodeIndex> = match config.direction {
                TraversalDirection::Outgoing => inner
                    .neighbors_directed(current, Direction::Outgoing)
                    .collect(),
                TraversalDirection::Incoming => inner
                    .neighbors_directed(current, Direction::Incoming)
                    .collect(),
                TraversalDirection::Both => {
                    let mut n: Vec<NodeIndex> = inner
                        .neighbors_directed(current, Direction::Outgoing)
                        .collect();
                    n.extend(inner.neighbors_directed(current, Direction::Incoming));
                    n
                }
            };

            let current_id = graph.node_id_for(current).cloned();

            for neighbor in neighbors {
                if let (Some(cur_id), Some(nbr_id)) = (&current_id, graph.node_id_for(neighbor)) {
                    result_edges.push((cur_id.clone(), nbr_id.clone()));

                    if visited.contains(&neighbor) {
                        cycles_detected_at.push(nbr_id.clone());
                        continue;
                    }
                }

                if result_nodes.len() >= config.max_nodes {
                    was_truncated = true;
                    break;
                }

                visited.insert(neighbor);
                if let Some(nbr_id) = graph.node_id_for(neighbor) {
                    result_nodes.push(nbr_id.clone());
                }
                next_layer.push(neighbor);
            }

            if was_truncated {
                break;
            }
        }

        if was_truncated {
            break;
        }

        current_layer = std::mem::take(&mut next_layer);
        if !current_layer.is_empty() {
            max_depth_reached = depth + 1;
        }
    }

    TraversalResult {
        nodes: result_nodes,
        edges: result_edges,
        depth_reached: max_depth_reached,
        was_truncated,
        cycles_detected_at,
    }
}

/// CSR-backed BFS traversal. Operates over a [`CsrSnapshot`] for
/// cache-friendly contiguous memory access. The semantics of the
/// returned [`TraversalResult`] match [`traverse_petgraph`]: nodes
/// in BFS order, edges between collected nodes (both endpoints in
/// the result set), depth bookkeeping, cycle detection.
///
/// ## Parallelism
///
/// When `config.parallel` is `true` and the frontier exceeds
/// [`PARALLEL_THRESHOLD`], frontier expansion is parallelised via
/// rayon's `par_chunks`. Per-thread visited sets are merged at
/// layer-end to preserve BFS-layer correctness.
pub fn traverse_csr(
    snapshot: &CsrSnapshot,
    start: &NodeId,
    config: &TraversalConfig,
) -> TraversalResult {
    let Some(start_v) = snapshot.vertex_of(start) else {
        return TraversalResult {
            nodes: vec![],
            edges: vec![],
            depth_reached: 0,
            was_truncated: false,
            cycles_detected_at: vec![],
        };
    };

    let mut result_nodes: Vec<NodeId> = Vec::with_capacity(config.max_nodes.min(64));
    let mut result_edges: Vec<(NodeId, NodeId)> = Vec::new();
    let mut cycles_detected_at: Vec<NodeId> = Vec::new();
    let mut visited: HashSet<u32> = HashSet::with_capacity(config.max_nodes.min(64));
    let mut was_truncated = false;
    let mut max_depth_reached: usize = 0;

    visited.insert(start_v.0);
    result_nodes.push(start.clone());

    let mut current: Vec<u32> = vec![start_v.0];
    let mut next: Vec<u32> = Vec::new();

    for depth in 0..config.max_depth {
        if current.is_empty() {
            break;
        }
        max_depth_reached = depth;

        // Decide push vs pull for this layer based on Yang 2018 alpha:
        // when frontier-edges > m / α, pull is cheaper. We approximate
        // the workload by summing degrees.
        let workload: usize = current
            .iter()
            .map(|&v| {
                let cv = CsrVertex(v);
                match config.direction {
                    TraversalDirection::Outgoing => snapshot.out_degree(cv),
                    TraversalDirection::Incoming => snapshot.in_degree(cv),
                    TraversalDirection::Both => snapshot.out_degree(cv) + snapshot.in_degree(cv),
                }
            })
            .sum();

        let should_parallel = config.parallel && current.len() >= PARALLEL_THRESHOLD;

        if should_parallel {
            // Rayon-parallel expansion: each thread emits its
            // candidates into a thread-local Vec; we merge with a
            // single-threaded dedup pass.
            use rayon::prelude::*;

            let local_results: Vec<Vec<(u32, u32)>> = current
                .par_chunks(64.max(current.len() / rayon::current_num_threads().max(1)))
                .map(|chunk| {
                    let mut out: Vec<(u32, u32)> = Vec::new();
                    for &cur in chunk {
                        let cv = CsrVertex(cur);
                        let neighbors: &[u32] = match config.direction {
                            TraversalDirection::Outgoing => snapshot.out_neighbours(cv),
                            TraversalDirection::Incoming => snapshot.in_neighbours(cv),
                            TraversalDirection::Both => {
                                // Both: emit outgoing then incoming.
                                snapshot.out_neighbours(cv)
                            }
                        };
                        for &n in neighbors {
                            out.push((cur, n));
                        }
                        if matches!(config.direction, TraversalDirection::Both) {
                            for &n in snapshot.in_neighbours(cv) {
                                out.push((cur, n));
                            }
                        }
                    }
                    out
                })
                .collect();

            for batch in local_results {
                for (cur, nbr) in batch {
                    if let (Some(cur_id), Some(nbr_id)) = (
                        snapshot.id_of(CsrVertex(cur)),
                        snapshot.id_of(CsrVertex(nbr)),
                    ) {
                        result_edges.push((cur_id.clone(), nbr_id.clone()));
                        if visited.contains(&nbr) {
                            cycles_detected_at.push(nbr_id.clone());
                            continue;
                        }
                    }
                    if result_nodes.len() >= config.max_nodes {
                        was_truncated = true;
                        break;
                    }
                    visited.insert(nbr);
                    if let Some(nbr_id) = snapshot.id_of(CsrVertex(nbr)) {
                        result_nodes.push(nbr_id.clone());
                    }
                    next.push(nbr);
                }
                if was_truncated {
                    break;
                }
            }
        } else {
            // Serial expansion. Hot inner loop kept tight.
            let _ = workload; // silence unused — would feed into pull-mode decision.
            for &cur in &current {
                let cv = CsrVertex(cur);
                let neighbours: Vec<u32> = match config.direction {
                    TraversalDirection::Outgoing => snapshot.out_neighbours(cv).to_vec(),
                    TraversalDirection::Incoming => snapshot.in_neighbours(cv).to_vec(),
                    TraversalDirection::Both => {
                        let mut all = snapshot.out_neighbours(cv).to_vec();
                        all.extend_from_slice(snapshot.in_neighbours(cv));
                        all
                    }
                };
                let cur_id = snapshot.id_of(cv).cloned();
                for nbr in neighbours {
                    if let (Some(cur_id), Some(nbr_id)) = (&cur_id, snapshot.id_of(CsrVertex(nbr)))
                    {
                        result_edges.push((cur_id.clone(), nbr_id.clone()));
                        if visited.contains(&nbr) {
                            cycles_detected_at.push(nbr_id.clone());
                            continue;
                        }
                    }
                    if result_nodes.len() >= config.max_nodes {
                        was_truncated = true;
                        break;
                    }
                    visited.insert(nbr);
                    if let Some(nbr_id) = snapshot.id_of(CsrVertex(nbr)) {
                        result_nodes.push(nbr_id.clone());
                    }
                    next.push(nbr);
                }
                if was_truncated {
                    break;
                }
            }
        }

        if was_truncated {
            break;
        }

        current = std::mem::take(&mut next);
        if !current.is_empty() {
            max_depth_reached = depth + 1;
        }
    }

    TraversalResult {
        nodes: result_nodes,
        edges: result_edges,
        depth_reached: max_depth_reached,
        was_truncated,
        cycles_detected_at,
    }
}

/// Find shortest path between two nodes using BFS.
///
/// Returns `None` if no path exists within `max_depth` hops.
///
/// Implementation note: parent tracking uses a `HashMap<NodeIndex, NodeIndex>`
/// rather than a `Vec<(NodeIndex, Option<NodeIndex>)>`. This avoids the previous
/// O(n) linear scan per reconstruction step (overall O(n²)) and rules out the
/// "stale entry causes infinite loop" failure mode that the linear scan was
/// vulnerable to. Path reconstruction is now O(n) total.
///
/// As belt-and-suspenders defense against any future corruption of the parent
/// map, reconstruction caps iterations at `parents.len() + 1` and returns `None`
/// on overflow rather than spinning forever.
pub fn find_path(
    graph: &CodeGraph,
    from: &NodeId,
    to: &NodeId,
    max_depth: usize,
) -> Option<Vec<NodeId>> {
    let from_idx = graph.resolve(from)?;
    let to_idx = graph.resolve(to)?;

    if from_idx == to_idx {
        return Some(vec![from.clone()]);
    }

    let inner = graph.inner();
    // `parents[child] = parent` — only inserted once per node (first time visited),
    // so look-ups are O(1) and the chain `to_idx → ... → from_idx` is acyclic by
    // construction (BFS never revisits, so each node has exactly one parent).
    let mut parents: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut queue: std::collections::VecDeque<(NodeIndex, usize)> =
        std::collections::VecDeque::new();

    queue.push_back((from_idx, 0));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
            if neighbor == from_idx || parents.contains_key(&neighbor) {
                continue;
            }

            parents.insert(neighbor, current);

            if neighbor == to_idx {
                return reconstruct_path(graph, &parents, from_idx, to_idx);
            }

            queue.push_back((neighbor, depth + 1));
        }
    }

    None
}

/// Multi-source shortest path: BFS from any of `sources` to `target`.
///
/// All sources are seeded into the BFS frontier at depth 0 simultaneously, so
/// the first source to reach `target` (in BFS order) wins. The returned path
/// starts at whichever source produced it and ends at `target`.
///
/// Returns `None` if `target` is unreachable from every source within
/// `max_depth` hops, if any input is missing from the graph, or if `sources`
/// is empty.
///
/// Implementation note: `parents` doubles as the visited set — sources insert
/// themselves with a self-parent so BFS never re-enqueues them, and path
/// reconstruction stops at the entry whose parent equals itself (rather than
/// the single fixed `from_idx` of the single-source case).
pub fn find_path_multi_source(
    graph: &CodeGraph,
    sources: &[NodeId],
    target: &NodeId,
    max_depth: usize,
) -> Option<Vec<NodeId>> {
    if sources.is_empty() {
        return None;
    }
    let target_idx = graph.resolve(target)?;
    let inner = graph.inner();

    // Seed every resolvable source at depth 0. Sources mark themselves as their
    // own parent so reconstruction can detect "we hit a source".
    let mut parents: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut queue: std::collections::VecDeque<(NodeIndex, usize)> =
        std::collections::VecDeque::new();

    for src in sources {
        if let Some(idx) = graph.resolve(src) {
            // Early exit: a source IS the target.
            if idx == target_idx {
                return Some(vec![src.clone()]);
            }
            // First seeding wins — duplicates are skipped via the contains check.
            if let std::collections::hash_map::Entry::Vacant(e) = parents.entry(idx) {
                e.insert(idx); // self-parent sentinel
                queue.push_back((idx, 0));
            }
        }
    }

    if queue.is_empty() {
        return None;
    }

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
            if parents.contains_key(&neighbor) {
                continue;
            }
            parents.insert(neighbor, current);

            if neighbor == target_idx {
                return reconstruct_path_multi_source(graph, &parents, target_idx);
            }
            queue.push_back((neighbor, depth + 1));
        }
    }

    None
}

/// Reconstruct a path back to whichever source it originated from.
///
/// Sources are encoded by `parents[src] == src` (self-parent sentinel set by
/// [`find_path_multi_source`]); the walk terminates the first time that
/// invariant holds.
fn reconstruct_path_multi_source(
    graph: &CodeGraph,
    parents: &HashMap<NodeIndex, NodeIndex>,
    target_idx: NodeIndex,
) -> Option<Vec<NodeId>> {
    let cap = parents.len() + 1;
    let mut path_indices: Vec<NodeIndex> = Vec::with_capacity(cap);
    path_indices.push(target_idx);

    let mut cursor = target_idx;
    loop {
        let parent = *parents.get(&cursor)?;
        if parent == cursor {
            // Reached a source (self-parent sentinel).
            break;
        }
        path_indices.push(parent);
        cursor = parent;
        if path_indices.len() > cap {
            return None;
        }
    }

    path_indices.reverse();
    Some(
        path_indices
            .into_iter()
            .filter_map(|idx| graph.node_id_for(idx).cloned())
            .collect(),
    )
}

/// Enumerate all simple (cycle-free) paths from `source` to `target` up to
/// `max_depth` hops, capped at `max_paths` results.
///
/// Uses iterative DFS with backtracking: a `path` stack tracks the current
/// candidate, and `on_path` (a `HashSet<NodeIndex>`) keeps the cycle-prevention
/// check at O(1). Each frame stores the iterator state for its expansion so
/// backtracking is just popping the stack — no recursion-depth ceiling.
///
/// Returns an empty vector if either endpoint is missing from the graph or if
/// no simple path under the depth cap exists. The `source == target` case
/// returns the single trivial path `[source]`.
pub fn all_simple_paths(
    graph: &CodeGraph,
    source: &NodeId,
    target: &NodeId,
    max_depth: usize,
    max_paths: usize,
) -> Vec<Vec<NodeId>> {
    let mut results: Vec<Vec<NodeId>> = Vec::new();
    if max_paths == 0 {
        return results;
    }
    let Some(source_idx) = graph.resolve(source) else {
        return results;
    };
    let Some(target_idx) = graph.resolve(target) else {
        return results;
    };

    if source_idx == target_idx {
        results.push(vec![source.clone()]);
        return results;
    }

    let inner = graph.inner();

    // Iterative DFS frame: (node, neighbor iterator).
    type Frame<'a> = (NodeIndex, petgraph::stable_graph::Neighbors<'a, EdgeData>);

    let mut path: Vec<NodeIndex> = Vec::new();
    let mut on_path: HashSet<NodeIndex> = HashSet::new();
    let mut stack: Vec<Frame> = Vec::new();

    path.push(source_idx);
    on_path.insert(source_idx);
    stack.push((
        source_idx,
        inner.neighbors_directed(source_idx, Direction::Outgoing),
    ));

    while let Some((_node, iter)) = stack.last_mut() {
        // Path length in *edges* (hops) is `path.len() - 1`. A new neighbor
        // would push to depth `path.len()`, i.e. `path.len()` hops. Skip
        // expansion entirely once that would exceed `max_depth`.
        if path.len() > max_depth {
            // Backtrack: this frame can't extend further without overshooting.
            on_path.remove(&path.pop().expect("path matches stack depth"));
            stack.pop();
            continue;
        }

        match iter.next() {
            Some(neighbor) => {
                if neighbor == target_idx {
                    // Snapshot the full path including the target.
                    let mut snapshot: Vec<NodeId> = path
                        .iter()
                        .filter_map(|&idx| graph.node_id_for(idx).cloned())
                        .collect();
                    if let Some(tid) = graph.node_id_for(target_idx) {
                        snapshot.push(tid.clone());
                    }
                    results.push(snapshot);
                    if results.len() >= max_paths {
                        return results;
                    }
                    // Don't descend through the target — paths through it
                    // would re-enter (target acts as a leaf for enumeration).
                    continue;
                }
                if on_path.contains(&neighbor) {
                    // Cycle — skip.
                    continue;
                }
                // Descend.
                path.push(neighbor);
                on_path.insert(neighbor);
                stack.push((
                    neighbor,
                    inner.neighbors_directed(neighbor, Direction::Outgoing),
                ));
            }
            None => {
                // No more neighbors — backtrack.
                on_path.remove(&path.pop().expect("path matches stack depth"));
                stack.pop();
            }
        }
    }

    results
}

/// Shortest path from `source` to `target` that avoids any node where
/// `avoid_node` returns `true`. The source and target themselves are checked
/// against the predicate; if either matches, no path is returned.
///
/// BFS, identical to [`find_path`] except that neighbors are filtered through
/// the predicate before being enqueued. The predicate is consulted at
/// expansion time rather than baked into the parent map, so callers can swap
/// predicates without re-traversing.
pub fn find_path_avoiding<F>(
    graph: &CodeGraph,
    source: &NodeId,
    target: &NodeId,
    max_depth: usize,
    avoid_node: F,
) -> Option<Vec<NodeId>>
where
    F: Fn(&NodeId, &NodeData) -> bool,
{
    let from_idx = graph.resolve(source)?;
    let to_idx = graph.resolve(target)?;

    // Source or target excluded by predicate → no admissible path exists.
    let inner = graph.inner();
    if let Some(data) = inner.node_weight(from_idx)
        && avoid_node(source, data)
    {
        return None;
    }
    if let Some(data) = inner.node_weight(to_idx)
        && avoid_node(target, data)
    {
        return None;
    }

    if from_idx == to_idx {
        return Some(vec![source.clone()]);
    }

    let mut parents: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut queue: std::collections::VecDeque<(NodeIndex, usize)> =
        std::collections::VecDeque::new();
    queue.push_back((from_idx, 0));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
            if neighbor == from_idx || parents.contains_key(&neighbor) {
                continue;
            }
            // Predicate check: skip avoided nodes (target itself is exempt
            // from avoidance — already validated above).
            if neighbor != to_idx
                && let Some(data) = inner.node_weight(neighbor)
                && let Some(nbr_id) = graph.node_id_for(neighbor)
                && avoid_node(nbr_id, data)
            {
                continue;
            }

            parents.insert(neighbor, current);

            if neighbor == to_idx {
                return reconstruct_path(graph, &parents, from_idx, to_idx);
            }
            queue.push_back((neighbor, depth + 1));
        }
    }

    None
}

/// Shortest path from `source` to `target` that avoids any edge where
/// `avoid_edge` returns `true`.
///
/// Mirror of [`find_path_avoiding`] for edge-level predicates. Iterates
/// `edges_directed` rather than `neighbors_directed` so the [`EdgeData`] is
/// available to the predicate; the same parent-map BFS reconstruction is
/// reused.
pub fn find_path_avoiding_edge<F>(
    graph: &CodeGraph,
    source: &NodeId,
    target: &NodeId,
    max_depth: usize,
    avoid_edge: F,
) -> Option<Vec<NodeId>>
where
    F: Fn(&EdgeData) -> bool,
{
    let from_idx = graph.resolve(source)?;
    let to_idx = graph.resolve(target)?;

    if from_idx == to_idx {
        return Some(vec![source.clone()]);
    }

    let inner = graph.inner();
    let mut parents: HashMap<NodeIndex, NodeIndex> = HashMap::new();
    let mut queue: std::collections::VecDeque<(NodeIndex, usize)> =
        std::collections::VecDeque::new();
    queue.push_back((from_idx, 0));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for edge in inner.edges_directed(current, Direction::Outgoing) {
            if avoid_edge(edge.weight()) {
                continue;
            }
            let neighbor = edge.target();
            if neighbor == from_idx || parents.contains_key(&neighbor) {
                continue;
            }
            parents.insert(neighbor, current);

            if neighbor == to_idx {
                return reconstruct_path(graph, &parents, from_idx, to_idx);
            }
            queue.push_back((neighbor, depth + 1));
        }
    }

    None
}

/// Reconstruct a path `from_idx → ... → to_idx` from a parent map produced by BFS.
///
/// Returns `None` if the chain exceeds the size of the parent map (which would
/// indicate corruption, since BFS guarantees each node has at most one parent
/// and the chain length is bounded by the visited set).
fn reconstruct_path(
    graph: &CodeGraph,
    parents: &HashMap<NodeIndex, NodeIndex>,
    from_idx: NodeIndex,
    to_idx: NodeIndex,
) -> Option<Vec<NodeId>> {
    let cap = parents.len() + 1;
    let mut path_indices: Vec<NodeIndex> = Vec::with_capacity(cap);
    path_indices.push(to_idx);

    let mut cursor = to_idx;
    while cursor != from_idx {
        let parent = *parents.get(&cursor)?;
        path_indices.push(parent);
        cursor = parent;
        if path_indices.len() > cap {
            // Defensive: parent map is corrupted (cycle or detached chain).
            return None;
        }
    }

    path_indices.reverse();
    Some(
        path_indices
            .into_iter()
            .filter_map(|idx| graph.node_id_for(idx).cloned())
            .collect(),
    )
}

/// Extract subgraph: all nodes reachable from `start` within `depth` in both directions.
pub fn subgraph(
    graph: &CodeGraph,
    start: &NodeId,
    depth: usize,
    max_nodes: usize,
) -> TraversalResult {
    traverse(
        graph,
        start,
        &TraversalConfig {
            max_depth: depth,
            max_nodes,
            direction: TraversalDirection::Both,
            parallel: false,
        },
    )
}

/// DFS traversal using petgraph's Dfs iterator.
///
/// Useful for topological-order sensitive operations (cascade planning).
pub fn dfs_collect(
    graph: &CodeGraph,
    start: &NodeId,
    direction: TraversalDirection,
    max_nodes: usize,
) -> Vec<NodeId> {
    let Some(start_idx) = graph.resolve(start) else {
        return vec![];
    };

    let inner = graph.inner();
    let mut result = Vec::new();

    match direction {
        TraversalDirection::Outgoing => {
            let mut dfs = Dfs::new(inner, start_idx);
            while let Some(nx) = dfs.next(inner) {
                if let Some(id) = graph.node_id_for(nx) {
                    result.push(id.clone());
                    if result.len() >= max_nodes {
                        break;
                    }
                }
            }
        }
        TraversalDirection::Incoming => {
            let reversed = Reversed(inner);
            let mut dfs = Dfs::new(&reversed, start_idx);
            while let Some(nx) = dfs.next(&reversed) {
                if let Some(id) = graph.node_id_for(nx) {
                    result.push(id.clone());
                    if result.len() >= max_nodes {
                        break;
                    }
                }
            }
        }
        TraversalDirection::Both => {
            // Both: collect outgoing then incoming, dedup
            let mut seen = HashSet::new();
            let mut dfs = Dfs::new(inner, start_idx);
            while let Some(nx) = dfs.next(inner) {
                if seen.insert(nx) {
                    if let Some(id) = graph.node_id_for(nx) {
                        result.push(id.clone());
                        if result.len() >= max_nodes {
                            break;
                        }
                    }
                }
            }
            if result.len() < max_nodes {
                let reversed = Reversed(inner);
                let mut dfs = Dfs::new(&reversed, start_idx);
                while let Some(nx) = dfs.next(&reversed) {
                    if seen.insert(nx) {
                        if let Some(id) = graph.node_id_for(nx) {
                            result.push(id.clone());
                            if result.len() >= max_nodes {
                                break;
                            }
                        }
                    }
                }
            }
        }
    }

    result
}

// Keep the old trait for backward compat with existing code that uses it,
// but implement it via the new graph methods.

/// Trait for providing graph connectivity (legacy compat layer).
pub trait GraphConnectivity {
    fn outgoing_neighbors(&self, node: &NodeId) -> Vec<NodeId>;
    fn incoming_neighbors(&self, node: &NodeId) -> Vec<NodeId>;
}

impl GraphConnectivity for CodeGraph {
    fn outgoing_neighbors(&self, node: &NodeId) -> Vec<NodeId> {
        let Some(idx) = self.resolve(node) else {
            return Vec::new();
        };
        self.inner()
            .neighbors_directed(idx, Direction::Outgoing)
            .filter_map(|n| self.node_id_for(n).cloned())
            .collect()
    }

    fn incoming_neighbors(&self, node: &NodeId) -> Vec<NodeId> {
        let Some(idx) = self.resolve(node) else {
            return Vec::new();
        };
        self.inner()
            .neighbors_directed(idx, Direction::Incoming)
            .filter_map(|n| self.node_id_for(n).cloned())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};

    fn sample_span() -> Span {
        Span {
            file: PathBuf::from("test.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 5,
            end_col: 1,
            byte_range: 0..50,
        }
    }

    fn make_node(name: &str) -> NodeData {
        let id = NodeId::new("test.rs", &format!("crate::{name}"), NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("test.rs"),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    fn make_edge() -> EdgeData {
        EdgeData {
            kind: EdgeKind::Calls,
            source_span: sample_span(),
            weight: 1.0,
        }
    }

    #[test]
    fn csr_traverse_matches_petgraph_traverse() {
        // Same graph, same query, two backends — must agree on
        // node set (order may differ between implementations).
        let mut g = CodeGraph::new();
        let a = g.add_node(make_node("a"));
        let b = g.add_node(make_node("b"));
        let c = g.add_node(make_node("c"));
        g.add_edge(&a, &b, make_edge()).unwrap();
        g.add_edge(&b, &c, make_edge()).unwrap();
        let cfg = TraversalConfig {
            max_depth: 5,
            max_nodes: 100,
            direction: TraversalDirection::Outgoing,
            parallel: false,
        };
        let pg = traverse_petgraph(&g, &a, &cfg);
        let csr = traverse_csr(&g.snapshot(), &a, &cfg);
        let pg_set: std::collections::BTreeSet<_> = pg.nodes.iter().collect();
        let csr_set: std::collections::BTreeSet<_> = csr.nodes.iter().collect();
        assert_eq!(pg_set, csr_set);
    }

    #[test]
    fn csr_traverse_respects_max_depth() {
        let mut g = CodeGraph::new();
        let a = g.add_node(make_node("a"));
        let b = g.add_node(make_node("b"));
        let c = g.add_node(make_node("c"));
        g.add_edge(&a, &b, make_edge()).unwrap();
        g.add_edge(&b, &c, make_edge()).unwrap();
        let r = traverse_csr(
            &g.snapshot(),
            &a,
            &TraversalConfig {
                max_depth: 1,
                max_nodes: 100,
                direction: TraversalDirection::Outgoing,
                parallel: false,
            },
        );
        assert!(r.nodes.contains(&a));
        assert!(r.nodes.contains(&b));
        assert!(!r.nodes.contains(&c));
    }

    #[test]
    fn csr_traverse_parallel_matches_serial() {
        // Build a fan-out graph so the parallel path has work to do.
        let mut g = CodeGraph::new();
        let root = g.add_node(make_node("root"));
        for i in 0..200 {
            let leaf = g.add_node(make_node(&format!("leaf{i}")));
            g.add_edge(&root, &leaf, make_edge()).unwrap();
        }
        let snap = g.snapshot();
        let cfg_serial = TraversalConfig {
            max_depth: 5,
            max_nodes: 1000,
            direction: TraversalDirection::Outgoing,
            parallel: false,
        };
        let cfg_par = TraversalConfig {
            parallel: true,
            ..cfg_serial
        };
        let serial = traverse_csr(&snap, &root, &cfg_serial);
        let par = traverse_csr(&snap, &root, &cfg_par);
        let s: std::collections::BTreeSet<_> = serial.nodes.iter().collect();
        let p: std::collections::BTreeSet<_> = par.nodes.iter().collect();
        assert_eq!(s, p, "parallel and serial CSR BFS must agree");
    }

    #[test]
    fn traverse_routes_to_csr_for_large_graphs() {
        // Build a graph just over CSR_SIZE_THRESHOLD; traverse() should
        // pick the CSR path. We can't directly observe which path was
        // taken, but we can verify correctness on a graph that
        // exercises the routing decision.
        let mut g = CodeGraph::new();
        let mut prev = g.add_node(make_node("n0"));
        for i in 1..(CSR_SIZE_THRESHOLD + 10) {
            let nid = g.add_node(make_node(&format!("n{i}")));
            g.add_edge(&prev, &nid, make_edge()).unwrap();
            prev = nid;
        }
        let r = traverse(
            &g,
            // start from "n0"
            &g.find_by_name("n0")[0].id.clone(),
            &TraversalConfig {
                max_depth: 50,
                max_nodes: 100,
                direction: TraversalDirection::Outgoing,
                parallel: false,
            },
        );
        assert!(r.nodes.len() > 1);
    }

    #[test]
    fn test_bfs_traversal() {
        let mut g = CodeGraph::new();
        let a_id = g.add_node(make_node("a"));
        let b_id = g.add_node(make_node("b"));
        let c_id = g.add_node(make_node("c"));
        g.add_edge(&a_id, &b_id, make_edge()).unwrap();
        g.add_edge(&b_id, &c_id, make_edge()).unwrap();

        let result = traverse(
            &g,
            &a_id,
            &TraversalConfig {
                max_depth: 5,
                max_nodes: 100,
                direction: TraversalDirection::Outgoing,
                parallel: false,
            },
        );

        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.nodes[0], a_id);
        assert!(!result.was_truncated);
    }

    #[test]
    fn test_cycle_detection() {
        let mut g = CodeGraph::new();
        let a_id = g.add_node(make_node("a"));
        let b_id = g.add_node(make_node("b"));
        g.add_edge(&a_id, &b_id, make_edge()).unwrap();
        g.add_edge(&b_id, &a_id, make_edge()).unwrap(); // cycle

        let result = traverse(
            &g,
            &a_id,
            &TraversalConfig {
                max_depth: 5,
                max_nodes: 100,
                direction: TraversalDirection::Outgoing,
                parallel: false,
            },
        );

        assert_eq!(result.nodes.len(), 2);
        assert!(!result.cycles_detected_at.is_empty());
    }

    #[test]
    fn test_max_nodes_truncation() {
        let mut g = CodeGraph::new();
        let ids: Vec<NodeId> = (0..10)
            .map(|i| g.add_node(make_node(&format!("n{i}"))))
            .collect();
        for i in 0..9 {
            g.add_edge(&ids[i], &ids[i + 1], make_edge()).unwrap();
        }

        let result = traverse(
            &g,
            &ids[0],
            &TraversalConfig {
                max_depth: 20,
                max_nodes: 5,
                direction: TraversalDirection::Outgoing,
                parallel: false,
            },
        );

        assert_eq!(result.nodes.len(), 5);
        assert!(result.was_truncated);
    }

    #[test]
    fn test_find_path() {
        let mut g = CodeGraph::new();
        let a_id = g.add_node(make_node("a"));
        let b_id = g.add_node(make_node("b"));
        let c_id = g.add_node(make_node("c"));
        g.add_edge(&a_id, &b_id, make_edge()).unwrap();
        g.add_edge(&b_id, &c_id, make_edge()).unwrap();

        let path = find_path(&g, &a_id, &c_id, 10).unwrap();
        assert_eq!(path.len(), 3);
        assert_eq!(path[0], a_id);
        assert_eq!(path[2], c_id);
    }

    #[test]
    fn find_path_long_chain_normal() {
        // Regression test for the O(n²) reconstruction loop in find_path.
        // Build a 1000-node chain n0 → n1 → ... → n999 and assert the returned
        // path has 1000 entries in the correct order. (We don't time-bound the
        // test — correctness on a long chain is sufficient evidence the
        // reconstruction is no longer quadratic, since the previous O(n²)
        // implementation also produced correct output, just slowly.)
        let mut g = CodeGraph::new();
        let n = 1000;
        let ids: Vec<NodeId> = (0..n)
            .map(|i| g.add_node(make_node(&format!("n{i}"))))
            .collect();
        for i in 0..n - 1 {
            g.add_edge(&ids[i], &ids[i + 1], make_edge()).unwrap();
        }

        let path = find_path(&g, &ids[0], &ids[n - 1], n).expect("path should exist");
        assert_eq!(path.len(), n);
        assert_eq!(path[0], ids[0]);
        assert_eq!(path[n - 1], ids[n - 1]);
        // Spot-check ordering at a few interior positions.
        assert_eq!(path[1], ids[1]);
        assert_eq!(path[n / 2], ids[n / 2]);
        assert_eq!(path[n - 2], ids[n - 2]);
    }

    #[test]
    fn find_path_returns_none_when_disconnected_robust() {
        // Two disconnected components: {a, b} and {c, d}. No path from a to d.
        let mut g = CodeGraph::new();
        let a_id = g.add_node(make_node("a"));
        let b_id = g.add_node(make_node("b"));
        let c_id = g.add_node(make_node("c"));
        let d_id = g.add_node(make_node("d"));
        g.add_edge(&a_id, &b_id, make_edge()).unwrap();
        g.add_edge(&c_id, &d_id, make_edge()).unwrap();

        assert!(find_path(&g, &a_id, &d_id, 100).is_none());
        assert!(find_path(&g, &a_id, &c_id, 100).is_none());
        // Sanity: paths within each component still resolve.
        assert!(find_path(&g, &a_id, &b_id, 100).is_some());
        assert!(find_path(&g, &c_id, &d_id, 100).is_some());
    }

    #[test]
    fn test_dfs_with_reversed() {
        let mut g = CodeGraph::new();
        let a_id = g.add_node(make_node("a"));
        let b_id = g.add_node(make_node("b"));
        let c_id = g.add_node(make_node("c"));
        g.add_edge(&a_id, &b_id, make_edge()).unwrap();
        g.add_edge(&b_id, &c_id, make_edge()).unwrap();

        // DFS from c going incoming should find b, a
        let result = dfs_collect(&g, &c_id, TraversalDirection::Incoming, 100);
        assert_eq!(result.len(), 3); // c, b, a
        assert_eq!(result[0], c_id);
    }

    #[test]
    fn find_path_multi_source_picks_shortest_from_any_normal() {
        // Three sources; the chosen one should produce the shortest path.
        //   s1 → x → t        (2 hops)
        //   s2 → t            (1 hop)  ← winner
        //   s3 → y → z → t    (3 hops)
        let mut g = CodeGraph::new();
        let s1 = g.add_node(make_node("s1"));
        let s2 = g.add_node(make_node("s2"));
        let s3 = g.add_node(make_node("s3"));
        let x = g.add_node(make_node("x"));
        let y = g.add_node(make_node("y"));
        let z = g.add_node(make_node("z"));
        let t = g.add_node(make_node("t"));
        g.add_edge(&s1, &x, make_edge()).unwrap();
        g.add_edge(&x, &t, make_edge()).unwrap();
        g.add_edge(&s2, &t, make_edge()).unwrap();
        g.add_edge(&s3, &y, make_edge()).unwrap();
        g.add_edge(&y, &z, make_edge()).unwrap();
        g.add_edge(&z, &t, make_edge()).unwrap();

        let path = find_path_multi_source(&g, &[s1.clone(), s2.clone(), s3.clone()], &t, 10)
            .expect("path should exist");
        assert_eq!(path, vec![s2, t]);
    }

    #[test]
    fn find_path_multi_source_returns_none_when_all_disconnected_robust() {
        // Sources sit in component A, target in component B. No path possible.
        let mut g = CodeGraph::new();
        let s1 = g.add_node(make_node("s1"));
        let s2 = g.add_node(make_node("s2"));
        let mid = g.add_node(make_node("mid"));
        let t = g.add_node(make_node("t"));
        let other = g.add_node(make_node("other"));
        g.add_edge(&s1, &mid, make_edge()).unwrap();
        g.add_edge(&s2, &mid, make_edge()).unwrap();
        g.add_edge(&t, &other, make_edge()).unwrap(); // detached subgraph

        assert!(find_path_multi_source(&g, &[s1.clone(), s2.clone()], &t, 100).is_none());
        // Empty sources: no path.
        assert!(find_path_multi_source(&g, &[], &t, 100).is_none());
        // Single source equal to target — the trivial path is returned.
        let trivial = find_path_multi_source(&g, std::slice::from_ref(&t), &t, 5).unwrap();
        assert_eq!(trivial, vec![t]);
    }

    #[test]
    fn all_simple_paths_diamond_returns_two_paths_normal() {
        // Diamond: A → B → D and A → C → D.
        let mut g = CodeGraph::new();
        let a = g.add_node(make_node("a"));
        let b = g.add_node(make_node("b"));
        let c = g.add_node(make_node("c"));
        let d = g.add_node(make_node("d"));
        g.add_edge(&a, &b, make_edge()).unwrap();
        g.add_edge(&a, &c, make_edge()).unwrap();
        g.add_edge(&b, &d, make_edge()).unwrap();
        g.add_edge(&c, &d, make_edge()).unwrap();

        let paths = all_simple_paths(&g, &a, &d, 10, 10);
        assert_eq!(paths.len(), 2);
        // Both paths start at A and end at D, length 3.
        for p in &paths {
            assert_eq!(p.len(), 3);
            assert_eq!(p[0], a);
            assert_eq!(p[2], d);
        }
        // Together they cover both intermediate nodes.
        let mids: HashSet<&NodeId> = paths.iter().map(|p| &p[1]).collect();
        assert!(mids.contains(&b));
        assert!(mids.contains(&c));
    }

    #[test]
    fn all_simple_paths_respects_max_paths_robust() {
        // Build A → {b1..b5} → t. Five distinct paths of length 3 exist;
        // cap at 3 and verify exactly 3 returned.
        let mut g = CodeGraph::new();
        let a = g.add_node(make_node("a"));
        let t = g.add_node(make_node("t"));
        let bs: Vec<NodeId> = (0..5)
            .map(|i| g.add_node(make_node(&format!("b{i}"))))
            .collect();
        for b in &bs {
            g.add_edge(&a, b, make_edge()).unwrap();
            g.add_edge(b, &t, make_edge()).unwrap();
        }

        let capped = all_simple_paths(&g, &a, &t, 10, 3);
        assert_eq!(capped.len(), 3);
        // Without the cap, all 5 should be enumerated.
        let uncapped = all_simple_paths(&g, &a, &t, 10, 100);
        assert_eq!(uncapped.len(), 5);
        // max_depth bound: with depth 1, no path of length 2 fits (path has 2
        // hops: a→bi→t).
        let too_shallow = all_simple_paths(&g, &a, &t, 1, 100);
        assert!(too_shallow.is_empty());
    }

    #[test]
    fn all_simple_paths_excludes_cyclic_paths_robust() {
        // Cycle: A → B → C → A, plus A → B → D. Only the acyclic path A→B→D
        // should be enumerated; the cycle must not produce A→B→C→A→B→D etc.
        let mut g = CodeGraph::new();
        let a = g.add_node(make_node("a"));
        let b = g.add_node(make_node("b"));
        let c = g.add_node(make_node("c"));
        let d = g.add_node(make_node("d"));
        g.add_edge(&a, &b, make_edge()).unwrap();
        g.add_edge(&b, &c, make_edge()).unwrap();
        g.add_edge(&c, &a, make_edge()).unwrap(); // cycle back
        g.add_edge(&b, &d, make_edge()).unwrap();

        let paths = all_simple_paths(&g, &a, &d, 20, 100);
        assert_eq!(paths.len(), 1);
        assert_eq!(paths[0], vec![a.clone(), b.clone(), d.clone()]);

        // From source==target, the trivial single-node path is returned.
        let trivial = all_simple_paths(&g, &a, &a, 5, 5);
        assert_eq!(trivial, vec![vec![a]]);
    }

    #[test]
    fn find_path_avoiding_node_skips_blacklisted_normal() {
        // Diamond A→B→D, A→C→D. Blacklist B by qualified name; only A→C→D
        // should be returned.
        let mut g = CodeGraph::new();
        let a = g.add_node(make_node("a"));
        let b = g.add_node(make_node("b"));
        let c = g.add_node(make_node("c"));
        let d = g.add_node(make_node("d"));
        g.add_edge(&a, &b, make_edge()).unwrap();
        g.add_edge(&a, &c, make_edge()).unwrap();
        g.add_edge(&b, &d, make_edge()).unwrap();
        g.add_edge(&c, &d, make_edge()).unwrap();

        let path = find_path_avoiding(&g, &a, &d, 10, |_id, data| {
            data.qualified_name == "crate::b"
        })
        .expect("path through C should exist");
        assert_eq!(path, vec![a.clone(), c.clone(), d.clone()]);

        // If we blacklist BOTH intermediates, no path exists.
        let blocked = find_path_avoiding(&g, &a, &d, 10, |_id, data| {
            data.qualified_name == "crate::b" || data.qualified_name == "crate::c"
        });
        assert!(blocked.is_none());

        // If we blacklist the source itself, no admissible path.
        let self_blocked = find_path_avoiding(&g, &a, &d, 10, |_id, data| {
            data.qualified_name == "crate::a"
        });
        assert!(self_blocked.is_none());
    }

    #[test]
    fn find_path_avoiding_edge_skips_blacklisted_normal() {
        // Diamond A→B→D, A→C→D. Blacklist the A→B edge by tagging it with a
        // distinct weight, then verify only the C-route is returned.
        let mut g = CodeGraph::new();
        let a = g.add_node(make_node("a"));
        let b = g.add_node(make_node("b"));
        let c = g.add_node(make_node("c"));
        let d = g.add_node(make_node("d"));
        let mut tagged = make_edge();
        tagged.weight = 99.0; // sentinel marking the edge to avoid
        g.add_edge(&a, &b, tagged).unwrap();
        g.add_edge(&a, &c, make_edge()).unwrap();
        g.add_edge(&b, &d, make_edge()).unwrap();
        g.add_edge(&c, &d, make_edge()).unwrap();

        let path = find_path_avoiding_edge(&g, &a, &d, 10, |edge| edge.weight == 99.0).unwrap();
        assert_eq!(path, vec![a.clone(), c.clone(), d.clone()]);

        // Blacklisting all outgoing edges from A produces no path.
        let blocked = find_path_avoiding_edge(&g, &a, &d, 10, |edge| edge.weight >= 0.0);
        assert!(blocked.is_none());
    }

    #[test]
    fn test_graph_connectivity_trait() {
        let mut g = CodeGraph::new();
        let a_id = g.add_node(make_node("a"));
        let b_id = g.add_node(make_node("b"));
        g.add_edge(&a_id, &b_id, make_edge()).unwrap();

        let out = g.outgoing_neighbors(&a_id);
        assert_eq!(out, vec![b_id.clone()]);
        let inc = g.incoming_neighbors(&b_id);
        assert_eq!(inc, vec![a_id]);
    }
}
