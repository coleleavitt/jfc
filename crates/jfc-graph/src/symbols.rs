//! Symbol table: maps human-readable handles to node locations for semantic editing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind, Span};

/// Human-readable symbol handle (e.g., "fn:sample::foo", "struct:Config").
pub type SymbolHandle = String;

/// Entry in the symbol table mapping a handle to its location.
#[derive(Debug, Clone)]
pub struct SymbolEntry {
    pub node_id: NodeId,
    pub handle: SymbolHandle,
    pub file_path: PathBuf,
    pub span: Span,
    pub qualified_name: String,
    pub kind: NodeKind,
}

/// Symbol table: bidirectional mapping between handles and code locations.
pub struct SymbolTable {
    by_handle: HashMap<SymbolHandle, SymbolEntry>,
    by_node_id: HashMap<NodeId, SymbolHandle>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            by_handle: HashMap::new(),
            by_node_id: HashMap::new(),
        }
    }

    /// Build symbol table from a CodeGraph — generates handles for all nodes.
    pub fn build_from_graph(graph: &CodeGraph) -> Self {
        let mut table = Self::new();

        for node_id in graph.all_node_ids() {
            let Some(node) = graph.get_node(node_id) else {
                continue;
            };
            table.insert_node(node_id, node);
        }

        table
    }

    /// Resolve exact handle to entry.
    pub fn resolve(&self, handle: &str) -> Option<&SymbolEntry> {
        self.by_handle.get(handle)
    }

    /// Fuzzy match: find entries where handle contains the partial string (case-insensitive).
    pub fn resolve_fuzzy(&self, partial: &str) -> Vec<&SymbolEntry> {
        let lower = partial.to_lowercase();
        self.by_handle
            .values()
            .filter(|entry| entry.handle.to_lowercase().contains(&lower))
            .collect()
    }

    /// Remove all entries for a given file (for incremental updates).
    pub fn invalidate_file(&mut self, path: &Path) {
        let handles_to_remove: Vec<SymbolHandle> = self
            .by_handle
            .values()
            .filter(|entry| entry.file_path == path)
            .map(|entry| entry.handle.clone())
            .collect();

        for handle in handles_to_remove {
            if let Some(entry) = self.by_handle.remove(&handle) {
                self.by_node_id.remove(&entry.node_id);
            }
        }
    }

    /// Rebuild entries for a single file from the graph.
    pub fn update_from_graph(&mut self, graph: &CodeGraph, changed_file: &Path) {
        self.invalidate_file(changed_file);

        for node_id in graph.all_node_ids() {
            let Some(node) = graph.get_node(node_id) else {
                continue;
            };
            if node.file_path == changed_file {
                self.insert_node(node_id, node);
            }
        }
    }

    /// Get all handles (for listing/completion).
    pub fn all_handles(&self) -> Vec<&str> {
        self.by_handle.keys().map(String::as_str).collect()
    }

    /// Get handle for a node ID.
    pub fn handle_for_node(&self, node_id: &NodeId) -> Option<&str> {
        self.by_node_id.get(node_id).map(String::as_str)
    }

    /// Total entry count.
    pub fn len(&self) -> usize {
        self.by_handle.len()
    }

    /// Returns true if the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.by_handle.is_empty()
    }

    /// Insert a single node into the symbol table.
    fn insert_node(&mut self, node_id: &NodeId, node: &crate::nodes::NodeData) {
        let handle = format!("{}:{}", kind_prefix(node.kind), node.qualified_name);

        let entry = SymbolEntry {
            node_id: node_id.clone(),
            handle: handle.clone(),
            file_path: node.file_path.clone(),
            span: node.span.clone(),
            qualified_name: node.qualified_name.clone(),
            kind: node.kind,
        };

        self.by_handle.insert(handle.clone(), entry);
        self.by_node_id.insert(node_id.clone(), handle);
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Map NodeKind to its handle prefix.
fn kind_prefix(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Function => "fn",
        NodeKind::Struct => "struct",
        NodeKind::Enum => "enum",
        NodeKind::Module => "mod",
        NodeKind::Trait => "trait",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

    fn make_span(file: &str) -> Span {
        Span {
            file: PathBuf::from(file),
            start_line: 1,
            start_col: 0,
            end_line: 10,
            end_col: 1,
            byte_range: 0..100,
        }
    }

    fn make_node(file: &str, name: &str, qualified: &str, kind: NodeKind) -> NodeData {
        let id = NodeId::new(file, qualified, kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: qualified.to_string(),
            file_path: PathBuf::from(file),
            span: make_span(file),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
        }
    }

    fn build_test_graph() -> CodeGraph {
        let mut graph = CodeGraph::new();
        graph.add_node(make_node("src/sample.rs", "foo", "sample::foo", NodeKind::Function));
        graph.add_node(make_node("src/sample.rs", "bar", "sample::bar", NodeKind::Function));
        graph.add_node(make_node("src/lib.rs", "Config", "Config", NodeKind::Struct));
        graph.add_node(make_node("src/lib.rs", "Status", "Status", NodeKind::Enum));
        graph.add_node(make_node("src/helpers.rs", "helpers", "helpers", NodeKind::Module));
        graph
    }

    #[test]
    fn test_symbol_table_build() {
        let graph = build_test_graph();
        let table = SymbolTable::build_from_graph(&graph);
        assert_eq!(table.len(), 5);
        assert!(!table.is_empty());
    }

    #[test]
    fn test_symbol_resolve_exact() {
        let graph = build_test_graph();
        let table = SymbolTable::build_from_graph(&graph);

        let entry = table.resolve("fn:sample::foo").expect("should resolve");
        assert_eq!(entry.kind, NodeKind::Function);
        assert_eq!(entry.qualified_name, "sample::foo");
        assert_eq!(entry.file_path, PathBuf::from("src/sample.rs"));

        let entry = table.resolve("struct:Config").expect("should resolve");
        assert_eq!(entry.kind, NodeKind::Struct);
        assert_eq!(entry.qualified_name, "Config");

        assert!(table.resolve("fn:nonexistent").is_none());
    }

    #[test]
    fn test_symbol_resolve_fuzzy() {
        let graph = build_test_graph();
        let table = SymbolTable::build_from_graph(&graph);

        let results = table.resolve_fuzzy("foo");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].handle, "fn:sample::foo");

        // Case-insensitive
        let results = table.resolve_fuzzy("CONFIG");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].handle, "struct:Config");

        // Partial match on prefix
        let results = table.resolve_fuzzy("fn:");
        assert_eq!(results.len(), 2);

        // No match
        let results = table.resolve_fuzzy("zzz_no_match");
        assert!(results.is_empty());
    }

    #[test]
    fn test_symbol_invalidate_file() {
        let graph = build_test_graph();
        let mut table = SymbolTable::build_from_graph(&graph);
        assert_eq!(table.len(), 5);

        // Invalidate src/sample.rs — removes foo and bar
        table.invalidate_file(Path::new("src/sample.rs"));
        assert_eq!(table.len(), 3);

        assert!(table.resolve("fn:sample::foo").is_none());
        assert!(table.resolve("fn:sample::bar").is_none());
        assert!(table.resolve("struct:Config").is_some());
        assert!(table.resolve("enum:Status").is_some());
        assert!(table.resolve("mod:helpers").is_some());
    }

    #[test]
    fn test_symbol_handle_for_node() {
        let graph = build_test_graph();
        let table = SymbolTable::build_from_graph(&graph);

        let foo_id = NodeId::new("src/sample.rs", "sample::foo", NodeKind::Function);
        let handle = table.handle_for_node(&foo_id).expect("should find handle");
        assert_eq!(handle, "fn:sample::foo");

        let config_id = NodeId::new("src/lib.rs", "Config", NodeKind::Struct);
        let handle = table.handle_for_node(&config_id).expect("should find handle");
        assert_eq!(handle, "struct:Config");

        let fake_id = NodeId(99999);
        assert!(table.handle_for_node(&fake_id).is_none());
    }

    #[test]
    fn test_symbol_deterministic() {
        let graph = build_test_graph();
        let table1 = SymbolTable::build_from_graph(&graph);
        let table2 = SymbolTable::build_from_graph(&graph);

        // Same graph produces same handles
        let mut handles1 = table1.all_handles();
        let mut handles2 = table2.all_handles();
        handles1.sort();
        handles2.sort();
        assert_eq!(handles1, handles2);

        // Each handle resolves to same entry data
        for handle in &handles1 {
            let e1 = table1.resolve(handle).unwrap();
            let e2 = table2.resolve(handle).unwrap();
            assert_eq!(e1.node_id, e2.node_id);
            assert_eq!(e1.qualified_name, e2.qualified_name);
            assert_eq!(e1.kind, e2.kind);
            assert_eq!(e1.file_path, e2.file_path);
        }
    }
}
