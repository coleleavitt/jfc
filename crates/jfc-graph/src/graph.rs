//! Core graph data structure and operations.
//!
//! Uses `StableGraph` instead of `DiGraph` so that `NodeIndex` values remain
//! stable across removals — no more swap-back fixup.

use std::collections::HashMap;
use std::path::Path;

use petgraph::Direction;
use petgraph::stable_graph::{NodeIndex, StableDiGraph};
use petgraph::visit::EdgeRef;
use thiserror::Error;

use crate::adapter::LanguageAdapter;
use crate::edges::EdgeData;
use crate::nodes::{NodeData, NodeId, NodeKind};
use crate::persistence::GraphEvent;

/// Errors from graph operations.
#[derive(Debug, Error)]
pub enum GraphError {
    #[error("node not found: {0:?}")]
    NodeNotFound(NodeId),

    #[error("edge already exists between {from:?} and {to:?}")]
    EdgeExists { from: NodeId, to: NodeId },
}

/// The core code graph — wraps petgraph's `StableDiGraph` with typed nodes and O(1) ID lookup.
///
/// `StableDiGraph` keeps indices stable across removals, eliminating the swap-back
/// fixup that was necessary with plain `DiGraph`.
pub struct CodeGraph {
    pub(crate) graph: StableDiGraph<NodeData, EdgeData>,
    pub(crate) index_map: HashMap<NodeId, NodeIndex>,
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            graph: StableDiGraph::new(),
            index_map: HashMap::new(),
        }
    }

    /// Direct read access to the inner petgraph. Enables all petgraph
    /// algorithms (SCC, dominators, toposort, page_rank, etc.) to operate
    /// without copying.
    pub fn inner(&self) -> &StableDiGraph<NodeData, EdgeData> {
        &self.graph
    }

    /// Resolve a NodeId to a petgraph NodeIndex.
    pub fn resolve(&self, id: &NodeId) -> Option<NodeIndex> {
        self.index_map.get(id).copied()
    }

    /// Reverse lookup: NodeIndex → NodeId.
    pub fn node_id_for(&self, idx: NodeIndex) -> Option<&NodeId> {
        self.graph.node_weight(idx).map(|n| &n.id)
    }

    /// Add a node. Returns the NodeId. If node with same ID exists, updates it.
    pub fn add_node(&mut self, data: NodeData) -> NodeId {
        let id = data.id.clone();
        if let Some(&idx) = self.index_map.get(&id) {
            self.graph[idx] = data;
        } else {
            let idx = self.graph.add_node(data);
            self.index_map.insert(id.clone(), idx);
        }
        id
    }

    /// Add an edge between two nodes. Returns error if either node doesn't exist.
    pub fn add_edge(
        &mut self,
        from: &NodeId,
        to: &NodeId,
        data: EdgeData,
    ) -> Result<(), GraphError> {
        let &from_idx = self
            .index_map
            .get(from)
            .ok_or_else(|| GraphError::NodeNotFound(from.clone()))?;
        let &to_idx = self
            .index_map
            .get(to)
            .ok_or_else(|| GraphError::NodeNotFound(to.clone()))?;

        self.graph.add_edge(from_idx, to_idx, data);
        Ok(())
    }

    /// Get node data by ID.
    pub fn get_node(&self, id: &NodeId) -> Option<&NodeData> {
        self.index_map.get(id).map(|&idx| &self.graph[idx])
    }

    /// Get all outgoing edges from a node: (target_id, edge_data)
    pub fn get_edges_from(&self, id: &NodeId) -> Vec<(&NodeId, &EdgeData)> {
        let Some(&idx) = self.index_map.get(id) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Outgoing)
            .map(|edge| {
                let target_data = &self.graph[edge.target()];
                (&target_data.id, edge.weight())
            })
            .collect()
    }

    /// Get all incoming edges to a node: (source_id, edge_data)
    pub fn get_edges_to(&self, id: &NodeId) -> Vec<(&NodeId, &EdgeData)> {
        let Some(&idx) = self.index_map.get(id) else {
            return Vec::new();
        };

        self.graph
            .edges_directed(idx, Direction::Incoming)
            .map(|edge| {
                let source_data = &self.graph[edge.source()];
                (&source_data.id, edge.weight())
            })
            .collect()
    }

    /// Remove a node and all its connected edges.
    ///
    /// With `StableDiGraph`, indices remain stable after removal — no swap-back fixup needed.
    pub fn remove_node(&mut self, id: &NodeId) -> Option<NodeData> {
        let idx = self.index_map.remove(id)?;
        self.graph.remove_node(idx)
    }

    /// Find nodes by kind.
    pub fn nodes_by_kind(&self, kind: NodeKind) -> Vec<&NodeData> {
        self.graph
            .node_weights()
            .filter(|data| data.kind == kind)
            .collect()
    }

    /// Find nodes by name (substring match, case-insensitive).
    pub fn find_by_name(&self, name: &str) -> Vec<&NodeData> {
        let lower = name.to_lowercase();
        self.graph
            .node_weights()
            .filter(|data| data.name.to_lowercase().contains(&lower))
            .collect()
    }

    /// Total node count.
    pub fn node_count(&self) -> usize {
        self.graph.node_count()
    }

    /// Total edge count.
    pub fn edge_count(&self) -> usize {
        self.graph.edge_count()
    }

    /// Get all node IDs.
    pub fn all_node_ids(&self) -> Vec<&NodeId> {
        self.index_map.keys().collect()
    }

    /// Check if a node exists.
    pub fn contains_node(&self, id: &NodeId) -> bool {
        self.index_map.contains_key(id)
    }

    /// Iterate over all node indices (for algorithm adapters).
    pub fn node_indices(&self) -> impl Iterator<Item = NodeIndex> + '_ {
        self.graph.node_indices()
    }

    /// Incrementally update the graph for a single changed file.
    /// Returns the persistence events generated.
    pub fn update_file(
        &mut self,
        path: &Path,
        new_content: &str,
        adapter: &dyn LanguageAdapter,
    ) -> Vec<GraphEvent> {
        let mut events = Vec::new();

        let to_remove: Vec<NodeId> = self
            .all_node_ids()
            .into_iter()
            .filter(|id| {
                self.get_node(id)
                    .map(|n| n.file_path == path)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();

        for id in &to_remove {
            if self.remove_node(id).is_some() {
                events.push(GraphEvent::NodeRemoved(id.clone()));
            }
        }

        if let Ok(parsed) = adapter.parse_file(path, new_content) {
            let nodes = adapter.extract_nodes(&parsed);
            for node in &nodes {
                self.add_node(node.clone());
                events.push(GraphEvent::NodeAdded(node.clone()));
            }
            let edges = adapter.extract_edges(&parsed, &nodes);
            for (from, to, data) in edges {
                if self.contains_node(&from) && self.contains_node(&to) {
                    let _ = self.add_edge(&from, &to, data.clone());
                    events.push(GraphEvent::EdgeAdded { from, to, data });
                }
            }
        }

        events.push(GraphEvent::FileReindexed(path.to_path_buf()));
        events
    }
}

impl Default for CodeGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::adapter::rust::RustAdapter;
    use crate::builder::GraphBuilder;
    use crate::edges::EdgeKind;
    use crate::nodes::{Span, Visibility};

    fn sample_span() -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: 1,
            start_col: 0,
            end_line: 10,
            end_col: 1,
            byte_range: 0..100,
        }
    }

    fn make_node(name: &str, kind: NodeKind) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: sample_span(),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
        }
    }

    fn make_edge(kind: EdgeKind) -> EdgeData {
        EdgeData {
            kind,
            source_span: sample_span(),
            weight: 1.0,
        }
    }

    #[test]
    fn test_add_and_get_node() {
        let mut graph = CodeGraph::new();
        let node = make_node("foo", NodeKind::Function);
        let id = graph.add_node(node.clone());

        assert!(graph.contains_node(&id));
        let retrieved = graph.get_node(&id).unwrap();
        assert_eq!(retrieved.name, "foo");
    }

    #[test]
    fn test_inner_access() {
        let mut graph = CodeGraph::new();
        let node = make_node("bar", NodeKind::Function);
        graph.add_node(node);
        assert_eq!(graph.inner().node_count(), 1);
    }

    #[test]
    fn test_resolve_and_node_id_for() {
        let mut graph = CodeGraph::new();
        let node = make_node("baz", NodeKind::Struct);
        let id = graph.add_node(node);

        let idx = graph.resolve(&id).unwrap();
        let round_trip = graph.node_id_for(idx).unwrap();
        assert_eq!(&id, round_trip);
    }

    #[test]
    fn test_add_edge_and_retrieve() {
        let mut graph = CodeGraph::new();
        let a = make_node("a", NodeKind::Function);
        let b = make_node("b", NodeKind::Function);
        let a_id = graph.add_node(a);
        let b_id = graph.add_node(b);

        graph
            .add_edge(&a_id, &b_id, make_edge(EdgeKind::Calls))
            .unwrap();

        let edges_from_a = graph.get_edges_from(&a_id);
        assert_eq!(edges_from_a.len(), 1);
        assert_eq!(edges_from_a[0].0, &b_id);

        let edges_to_b = graph.get_edges_to(&b_id);
        assert_eq!(edges_to_b.len(), 1);
        assert_eq!(edges_to_b[0].0, &a_id);
    }

    #[test]
    fn test_remove_node() {
        let mut graph = CodeGraph::new();
        let node = make_node("remove_me", NodeKind::Function);
        let id = graph.add_node(node);

        assert!(graph.contains_node(&id));
        graph.remove_node(&id);
        assert!(!graph.contains_node(&id));
    }

    #[test]
    fn test_stable_indices_after_removal() {
        let mut graph = CodeGraph::new();
        let a_id = graph.add_node(make_node("a", NodeKind::Function));
        let b_id = graph.add_node(make_node("b", NodeKind::Function));
        let c_id = graph.add_node(make_node("c", NodeKind::Function));

        // Remove middle node
        graph.remove_node(&b_id);

        // Other indices still resolve correctly
        assert!(graph.resolve(&a_id).is_some());
        assert!(graph.resolve(&c_id).is_some());
        assert_eq!(graph.get_node(&a_id).unwrap().name, "a");
        assert_eq!(graph.get_node(&c_id).unwrap().name, "c");
    }
}
