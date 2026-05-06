//! Graph traversal algorithms with cycle detection.
//!
//! Provides BFS-based traversal that tracks visited nodes to prevent infinite
//! expansion from mutual recursion or circular dependencies.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::nodes::NodeId;

/// Direction of traversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
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
    pub direction: Direction,
}

impl Default for TraversalConfig {
    fn default() -> Self {
        Self {
            max_depth: 3,
            max_nodes: 100,
            direction: Direction::Outgoing,
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
    /// Node IDs where cycles were detected
    pub cycles_detected_at: Vec<NodeId>,
}

/// Trait for providing graph connectivity (decouples traversal from graph implementation).
pub trait GraphConnectivity {
    /// Get all nodes reachable via outgoing edges from `node`.
    fn outgoing_neighbors(&self, node: &NodeId) -> Vec<NodeId>;
    /// Get all nodes with incoming edges to `node`.
    fn incoming_neighbors(&self, node: &NodeId) -> Vec<NodeId>;
}

/// Perform a BFS traversal with cycle detection and depth/node limits.
pub fn traverse(
    start: &NodeId,
    graph: &dyn GraphConnectivity,
    config: &TraversalConfig,
) -> TraversalResult {
    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut queue: VecDeque<(NodeId, usize)> = VecDeque::new();
    let mut result_nodes: Vec<NodeId> = Vec::new();
    let mut result_edges: Vec<(NodeId, NodeId)> = Vec::new();
    let mut cycles_detected_at: Vec<NodeId> = Vec::new();
    let mut max_depth_reached: usize = 0;
    let mut was_truncated = false;

    visited.insert(start.clone());
    queue.push_back((start.clone(), 0));
    result_nodes.push(start.clone());

    while let Some((current, depth)) = queue.pop_front() {
        max_depth_reached = max_depth_reached.max(depth);

        if depth >= config.max_depth {
            continue;
        }

        let neighbors = match config.direction {
            Direction::Outgoing => graph.outgoing_neighbors(&current),
            Direction::Incoming => graph.incoming_neighbors(&current),
            Direction::Both => {
                let mut n = graph.outgoing_neighbors(&current);
                n.extend(graph.incoming_neighbors(&current));
                n
            }
        };

        for neighbor in neighbors {
            result_edges.push((current.clone(), neighbor.clone()));

            if visited.contains(&neighbor) {
                cycles_detected_at.push(neighbor.clone());
                continue;
            }

            if result_nodes.len() >= config.max_nodes {
                was_truncated = true;
                break;
            }

            visited.insert(neighbor.clone());
            result_nodes.push(neighbor.clone());
            queue.push_back((neighbor, depth + 1));
        }

        if was_truncated {
            break;
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

/// Find shortest path between two nodes (BFS).
///
/// Returns `None` if no path exists within `max_depth` hops.
pub fn find_path(
    from: &NodeId,
    to: &NodeId,
    graph: &dyn GraphConnectivity,
    max_depth: usize,
) -> Option<Vec<NodeId>> {
    if from == to {
        return Some(vec![from.clone()]);
    }

    let mut visited: HashSet<NodeId> = HashSet::new();
    let mut queue: VecDeque<(NodeId, usize)> = VecDeque::new();
    let mut parents: HashMap<NodeId, NodeId> = HashMap::new();

    visited.insert(from.clone());
    queue.push_back((from.clone(), 0));

    while let Some((current, depth)) = queue.pop_front() {
        if depth >= max_depth {
            continue;
        }

        for neighbor in graph.outgoing_neighbors(&current) {
            if visited.contains(&neighbor) {
                continue;
            }

            parents.insert(neighbor.clone(), current.clone());

            if &neighbor == to {
                // Reconstruct path
                let mut path = vec![neighbor];
                let mut cursor = &path[0];
                while let Some(parent) = parents.get(cursor) {
                    path.push(parent.clone());
                    cursor = parent;
                }
                path.reverse();
                return Some(path);
            }

            visited.insert(neighbor.clone());
            queue.push_back((neighbor, depth + 1));
        }
    }

    None
}

/// Extract subgraph: all nodes reachable from `start` within `depth` in both directions.
pub fn subgraph(
    start: &NodeId,
    graph: &dyn GraphConnectivity,
    depth: usize,
    max_nodes: usize,
) -> TraversalResult {
    traverse(
        start,
        graph,
        &TraversalConfig {
            max_depth: depth,
            max_nodes,
            direction: Direction::Both,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockGraph {
        edges: HashMap<NodeId, Vec<NodeId>>,
        reverse_edges: HashMap<NodeId, Vec<NodeId>>,
    }

    impl MockGraph {
        fn new() -> Self {
            Self {
                edges: HashMap::new(),
                reverse_edges: HashMap::new(),
            }
        }

        fn add_edge(&mut self, from: NodeId, to: NodeId) {
            self.edges.entry(from.clone()).or_default().push(to.clone());
            self.reverse_edges.entry(to).or_default().push(from);
        }
    }

    impl GraphConnectivity for MockGraph {
        fn outgoing_neighbors(&self, node: &NodeId) -> Vec<NodeId> {
            self.edges.get(node).cloned().unwrap_or_default()
        }

        fn incoming_neighbors(&self, node: &NodeId) -> Vec<NodeId> {
            self.reverse_edges.get(node).cloned().unwrap_or_default()
        }
    }

    fn node(id: u64) -> NodeId {
        NodeId(id)
    }

    #[test]
    fn test_cycle_detection() {
        let mut graph = MockGraph::new();
        let ping = node(1);
        let pong = node(2);

        // ping → pong → ping (cycle)
        graph.add_edge(ping.clone(), pong.clone());
        graph.add_edge(pong.clone(), ping.clone());

        let result = traverse(
            &ping,
            &graph,
            &TraversalConfig {
                max_depth: 10,
                max_nodes: 100,
                direction: Direction::Outgoing,
            },
        );

        // Must terminate
        assert_eq!(result.nodes.len(), 2);
        assert!(result.nodes.contains(&ping));
        assert!(result.nodes.contains(&pong));
        assert!(!result.cycles_detected_at.is_empty());
    }

    #[test]
    fn test_depth_limit() {
        // Linear chain: 0→1→2→3→4→5→6→7→8→9
        let mut graph = MockGraph::new();
        for i in 0..9 {
            graph.add_edge(node(i), node(i + 1));
        }

        let result = traverse(
            &node(0),
            &graph,
            &TraversalConfig {
                max_depth: 2,
                max_nodes: 100,
                direction: Direction::Outgoing,
            },
        );

        // depth=0: node(0), depth=1: node(1), depth=2: node(2)
        assert_eq!(result.nodes.len(), 3);
        assert_eq!(result.nodes[0], node(0));
        assert_eq!(result.nodes[1], node(1));
        assert_eq!(result.nodes[2], node(2));
        assert_eq!(result.depth_reached, 2);
    }

    #[test]
    fn test_max_nodes_truncation() {
        // Linear chain: 0→1→2→...→9
        let mut graph = MockGraph::new();
        for i in 0..9 {
            graph.add_edge(node(i), node(i + 1));
        }

        let result = traverse(
            &node(0),
            &graph,
            &TraversalConfig {
                max_depth: 20,
                max_nodes: 5,
                direction: Direction::Outgoing,
            },
        );

        assert!(result.was_truncated);
        assert_eq!(result.nodes.len(), 5);
    }

    #[test]
    fn test_direction_outgoing() {
        // a(0) → b(1) → c(2)
        let mut graph = MockGraph::new();
        graph.add_edge(node(0), node(1));
        graph.add_edge(node(1), node(2));

        let result = traverse(
            &node(1),
            &graph,
            &TraversalConfig {
                max_depth: 10,
                max_nodes: 100,
                direction: Direction::Outgoing,
            },
        );

        // From b outgoing: b, c
        assert_eq!(result.nodes.len(), 2);
        assert!(result.nodes.contains(&node(1)));
        assert!(result.nodes.contains(&node(2)));
        assert!(!result.nodes.contains(&node(0)));
    }

    #[test]
    fn test_direction_incoming() {
        // a(0) → b(1) → c(2)
        let mut graph = MockGraph::new();
        graph.add_edge(node(0), node(1));
        graph.add_edge(node(1), node(2));

        let result = traverse(
            &node(1),
            &graph,
            &TraversalConfig {
                max_depth: 10,
                max_nodes: 100,
                direction: Direction::Incoming,
            },
        );

        // From b incoming: b, a
        assert_eq!(result.nodes.len(), 2);
        assert!(result.nodes.contains(&node(1)));
        assert!(result.nodes.contains(&node(0)));
        assert!(!result.nodes.contains(&node(2)));
    }

    #[test]
    fn test_find_path() {
        // a(0) → b(1) → c(2)
        let mut graph = MockGraph::new();
        graph.add_edge(node(0), node(1));
        graph.add_edge(node(1), node(2));

        let path = find_path(&node(0), &node(2), &graph, 10);
        assert_eq!(path, Some(vec![node(0), node(1), node(2)]));
    }

    #[test]
    fn test_find_path_no_path() {
        // Disconnected: a(0) → b(1), c(2) isolated
        let mut graph = MockGraph::new();
        graph.add_edge(node(0), node(1));

        let path = find_path(&node(0), &node(2), &graph, 10);
        assert_eq!(path, None);
    }
}
