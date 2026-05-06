//! Core graph data structure and operations.

use std::collections::HashMap;
use std::path::Path;

use petgraph::graph::{DiGraph, NodeIndex};
use petgraph::visit::EdgeRef;
use petgraph::Direction;
use thiserror::Error;

use crate::adapter::LanguageAdapter;
use crate::edges::EdgeData;
use crate::nodes::{NodeData, NodeId, NodeKind};
use crate::persistence::GraphEvent;
use crate::traversal::GraphConnectivity;

/// Errors from graph operations.
#[derive(Debug, Error)]
pub enum GraphError {
    #[error("node not found: {0:?}")]
    NodeNotFound(NodeId),

    #[error("edge already exists between {from:?} and {to:?}")]
    EdgeExists { from: NodeId, to: NodeId },
}

/// The core code graph — wraps petgraph with typed nodes and O(1) ID lookup.
pub struct CodeGraph {
    graph: DiGraph<NodeData, EdgeData>,
    index_map: HashMap<NodeId, NodeIndex>,
}

impl CodeGraph {
    pub fn new() -> Self {
        Self {
            graph: DiGraph::new(),
            index_map: HashMap::new(),
        }
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
        self.index_map
            .get(id)
            .map(|&idx| &self.graph[idx])
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
    pub fn remove_node(&mut self, id: &NodeId) -> Option<NodeData> {
        let idx = self.index_map.remove(id)?;

        let removed = self.graph.remove_node(idx)?;

        // petgraph swaps the last node into the removed index.
        // If the removed index is now occupied by a different node, update index_map.
        if idx.index() < self.graph.node_count() {
            // A node was swapped into `idx`
            let swapped_data = &self.graph[idx];
            self.index_map.insert(swapped_data.id.clone(), idx);
        }

        Some(removed)
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

/// Implement GraphConnectivity so traversal algorithms work on CodeGraph.
impl GraphConnectivity for CodeGraph {
    fn outgoing_neighbors(&self, node: &NodeId) -> Vec<NodeId> {
        let Some(&idx) = self.index_map.get(node) else {
            return Vec::new();
        };

        self.graph
            .neighbors_directed(idx, Direction::Outgoing)
            .map(|neighbor_idx| self.graph[neighbor_idx].id.clone())
            .collect()
    }

    fn incoming_neighbors(&self, node: &NodeId) -> Vec<NodeId> {
        let Some(&idx) = self.index_map.get(node) else {
            return Vec::new();
        };

        self.graph
            .neighbors_directed(idx, Direction::Incoming)
            .map(|neighbor_idx| self.graph[neighbor_idx].id.clone())
            .collect()
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
    fn test_graph_add_node() {
        let mut graph = CodeGraph::new();

        let nodes: Vec<NodeData> = (0..5)
            .map(|i| make_node(&format!("node_{i}"), NodeKind::Function))
            .collect();

        for node in nodes {
            graph.add_node(node);
        }

        assert_eq!(graph.node_count(), 5);
    }

    #[test]
    fn test_graph_add_edge() {
        let mut graph = CodeGraph::new();

        let a = make_node("alpha", NodeKind::Function);
        let b = make_node("beta", NodeKind::Function);
        let c = make_node("gamma", NodeKind::Function);

        let a_id = graph.add_node(a);
        let b_id = graph.add_node(b);
        let c_id = graph.add_node(c);

        graph
            .add_edge(&a_id, &b_id, make_edge(EdgeKind::Calls))
            .unwrap();
        graph
            .add_edge(&b_id, &c_id, make_edge(EdgeKind::Calls))
            .unwrap();

        assert_eq!(graph.edge_count(), 2);

        let edges_from_a = graph.get_edges_from(&a_id);
        assert_eq!(edges_from_a.len(), 1);
        assert_eq!(edges_from_a[0].0, &b_id);
    }

    #[test]
    fn test_graph_add_edge_node_not_found() {
        let mut graph = CodeGraph::new();
        let a = make_node("alpha", NodeKind::Function);
        let a_id = graph.add_node(a);
        let fake_id = NodeId(99999);

        let result = graph.add_edge(&a_id, &fake_id, make_edge(EdgeKind::Calls));
        assert!(result.is_err());
    }

    #[test]
    fn test_graph_lookup_by_name() {
        let mut graph = CodeGraph::new();

        graph.add_node(make_node("foo_bar", NodeKind::Function));
        graph.add_node(make_node("foo_baz", NodeKind::Function));
        graph.add_node(make_node("quux", NodeKind::Struct));

        let results = graph.find_by_name("foo");
        assert_eq!(results.len(), 2);
        assert!(results.iter().all(|n| n.name.contains("foo")));

        // Case-insensitive
        let results_upper = graph.find_by_name("FOO");
        assert_eq!(results_upper.len(), 2);
    }

    #[test]
    fn test_graph_remove_node() {
        let mut graph = CodeGraph::new();

        let a = make_node("a_node", NodeKind::Function);
        let b = make_node("b_node", NodeKind::Function);
        let c = make_node("c_node", NodeKind::Function);

        let a_id = graph.add_node(a);
        let b_id = graph.add_node(b);
        let c_id = graph.add_node(c);

        graph
            .add_edge(&a_id, &b_id, make_edge(EdgeKind::Calls))
            .unwrap();
        graph
            .add_edge(&b_id, &c_id, make_edge(EdgeKind::Calls))
            .unwrap();

        // Remove B
        let removed = graph.remove_node(&b_id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "b_node");

        // B is gone
        assert!(!graph.contains_node(&b_id));
        assert_eq!(graph.node_count(), 2);

        // A and C remain
        assert!(graph.contains_node(&a_id));
        assert!(graph.contains_node(&c_id));

        // Edges involving B are gone
        assert_eq!(graph.edge_count(), 0);
    }

    #[test]
    fn test_graph_nodes_by_kind() {
        let mut graph = CodeGraph::new();

        graph.add_node(make_node("func1", NodeKind::Function));
        graph.add_node(make_node("func2", NodeKind::Function));
        graph.add_node(make_node("MyStruct", NodeKind::Struct));
        graph.add_node(make_node("MyEnum", NodeKind::Enum));
        graph.add_node(make_node("MyTrait", NodeKind::Trait));

        let functions = graph.nodes_by_kind(NodeKind::Function);
        assert_eq!(functions.len(), 2);
        assert!(functions.iter().all(|n| n.kind == NodeKind::Function));

        let structs = graph.nodes_by_kind(NodeKind::Struct);
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "MyStruct");
    }

    #[test]
    fn test_graph_connectivity() {
        let mut graph = CodeGraph::new();

        let a = make_node("a", NodeKind::Function);
        let b = make_node("b", NodeKind::Function);
        let c = make_node("c", NodeKind::Function);

        let a_id = graph.add_node(a);
        let b_id = graph.add_node(b);
        let c_id = graph.add_node(c);

        graph
            .add_edge(&a_id, &b_id, make_edge(EdgeKind::Calls))
            .unwrap();
        graph
            .add_edge(&a_id, &c_id, make_edge(EdgeKind::Calls))
            .unwrap();

        // Outgoing from A
        let outgoing = graph.outgoing_neighbors(&a_id);
        assert_eq!(outgoing.len(), 2);
        assert!(outgoing.contains(&b_id));
        assert!(outgoing.contains(&c_id));

        // Incoming to B
        let incoming = graph.incoming_neighbors(&b_id);
        assert_eq!(incoming.len(), 1);
        assert!(incoming.contains(&a_id));

        // A has no incoming
        let a_incoming = graph.incoming_neighbors(&a_id);
        assert!(a_incoming.is_empty());
    }

    #[test]
    fn test_graph_duplicate_node() {
        let mut graph = CodeGraph::new();

        let node1 = make_node("original", NodeKind::Function);
        let id = node1.id.clone();
        graph.add_node(node1);

        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.get_node(&id).unwrap().name, "original");

        // Add node with same ID but different data
        let mut node2 = make_node("original", NodeKind::Function);
        node2.name = "updated".to_string();
        // Ensure same ID
        node2.id = id.clone();

        graph.add_node(node2);

        // Still only 1 node, but data is updated
        assert_eq!(graph.node_count(), 1);
        assert_eq!(graph.get_node(&id).unwrap().name, "updated");
    }

    #[test]
    fn test_update_file() {
        let adapter = RustAdapter::new();
        let fixtures = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let sample_path = fixtures.join("sample.rs");

        let mut graph = GraphBuilder::build_from_files(&[sample_path.clone()], &adapter);
        let initial_count = graph.node_count();
        assert!(initial_count > 0);

        let modified_content = r#"
pub fn alpha() {
    beta();
}

fn beta() -> i32 {
    99
}
"#;

        let events = graph.update_file(&sample_path, modified_content, &adapter);

        assert!(!events.is_empty());
        assert!(
            events.iter().any(|e| matches!(e, GraphEvent::FileReindexed(_))),
        );

        let names: Vec<&str> = graph
            .find_by_name("alpha")
            .iter()
            .map(|n| n.name.as_str())
            .collect();
        assert!(names.contains(&"alpha"));

        assert!(graph.find_by_name("foo").is_empty());
        assert!(graph.find_by_name("bar").is_empty());
    }
}
