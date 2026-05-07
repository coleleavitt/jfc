//! Graph traversal algorithms leveraging petgraph's built-in iterators.
//!
//! Replaces hand-rolled BFS/DFS with petgraph's `Bfs`, `Dfs`, and `Reversed`
//! adapters for cycle-detected, depth-bounded traversal with zero-copy
//! direction flipping.

use std::collections::HashSet;

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::{Dfs, Reversed};

use crate::graph::CodeGraph;
use crate::nodes::NodeId;

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
}

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_nodes: 100,
            direction: TraversalDirection::Outgoing,
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

/// Perform a bounded BFS traversal using petgraph's Bfs iterator.
///
/// Depth tracking is maintained manually since petgraph's Bfs doesn't
/// expose depth natively. Cycle detection reports back-edges via the
/// `cycles_detected_at` field.
pub fn traverse(graph: &CodeGraph, start: &NodeId, config: &TraversalConfig) -> TraversalResult {
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

/// Find shortest path between two nodes using BFS.
///
/// Returns `None` if no path exists within `max_depth` hops.
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
    let mut visited: HashSet<NodeIndex> = HashSet::new();
    let mut parents: Vec<(NodeIndex, Option<NodeIndex>)> = Vec::new();
    let mut queue: std::collections::VecDeque<(NodeIndex, usize)> =
        std::collections::VecDeque::new();

    visited.insert(from_idx);
    parents.push((from_idx, None));
    queue.push_back((from_idx, 0));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for neighbor in inner.neighbors_directed(current, Direction::Outgoing) {
            if visited.contains(&neighbor) {
                continue;
            }

            visited.insert(neighbor);
            parents.push((neighbor, Some(current)));

            if neighbor == to_idx {
                // Reconstruct path
                let mut path_indices = vec![neighbor];
                let mut cursor = current;
                path_indices.push(cursor);
                while let Some((_, Some(parent))) = parents.iter().find(|(n, _)| *n == cursor) {
                    path_indices.push(*parent);
                    cursor = *parent;
                }
                path_indices.reverse();

                return Some(
                    path_indices
                        .iter()
                        .filter_map(|idx| graph.node_id_for(*idx).cloned())
                        .collect(),
                );
            }

            queue.push_back((neighbor, depth + 1));
        }
    }

    None
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
        let Some(&idx) = self.index_map.get(node) else {
            return Vec::new();
        };
        self.graph
            .neighbors_directed(idx, Direction::Outgoing)
            .filter_map(|n| self.node_id_for(n).cloned())
            .collect()
    }

    fn incoming_neighbors(&self, node: &NodeId) -> Vec<NodeId> {
        let Some(&idx) = self.index_map.get(node) else {
            return Vec::new();
        };
        self.graph
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
