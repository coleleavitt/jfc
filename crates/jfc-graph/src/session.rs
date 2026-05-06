//! High-level session facade — the single entry point for jfc-ui.

use std::path::Path;

use crate::adapter::rust::RustAdapter;
use crate::builder::GraphBuilder;
use crate::capabilities::{Capability, CapabilityTree};
use crate::dsl::{self, QueryConfig, QueryError, QueryResult};
use crate::formatting::{self, FormattedOutput};
use crate::graph::CodeGraph;
use crate::persistence::EventLog;
use crate::symbols::SymbolTable;

/// Owns the graph, symbols, event log, and capabilities.
/// Provides query execution and incremental file updates.
pub struct GraphSession {
    pub graph: CodeGraph,
    pub symbols: SymbolTable,
    pub events: EventLog,
    pub capabilities: CapabilityTree,
    adapter: RustAdapter,
}

impl GraphSession {
    /// Build a session by indexing all supported files under `workspace_root`.
    pub fn from_directory(workspace_root: &Path) -> Self {
        let adapter = RustAdapter::new();
        let graph = GraphBuilder::build_from_directory(workspace_root, &adapter);
        let symbols = SymbolTable::build_from_graph(&graph);
        Self {
            graph,
            symbols,
            events: EventLog::new(),
            capabilities: CapabilityTree::new(),
            adapter,
        }
    }

    /// Execute a DSL query and return token-budgeted formatted output.
    pub fn query(&self, query_str: &str, max_tokens: usize) -> Result<FormattedOutput, QueryError> {
        let config = QueryConfig {
            max_tokens,
            max_nodes: 50,
        };
        let result = dsl::run_query(query_str, &self.graph, &config)?;
        Ok(formatting::format_query_result(
            &result,
            &self.graph,
            Some(&self.symbols),
            max_tokens,
        ))
    }

    /// Execute a DSL query and return the raw result for programmatic use.
    pub fn query_raw(&self, query_str: &str) -> Result<QueryResult, QueryError> {
        let config = QueryConfig::default();
        dsl::run_query(query_str, &self.graph, &config)
    }

    /// Incrementally update the graph after a file modification.
    pub fn file_changed(&mut self, path: &Path, new_content: &str) {
        let events = self.graph.update_file(path, new_content, &self.adapter);
        for event in events {
            self.events.append(event, None);
        }
        self.symbols.update_from_graph(&self.graph, path);
    }

    pub fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }

    pub fn is_capable(&self, cap: Capability) -> bool {
        self.capabilities.is_enabled(cap)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn fixtures_dir() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
    }

    #[test]
    fn test_session_from_fixtures() {
        let session = GraphSession::from_directory(fixtures_dir());
        assert!(
            session.graph.node_count() > 0,
            "session graph should have nodes from fixtures"
        );
        assert!(
            !session.symbols.is_empty(),
            "session symbols should be populated"
        );
    }

    #[test]
    fn test_session_query() {
        let session = GraphSession::from_directory(fixtures_dir());
        let output = session
            .query(r#"fn("foo") | callees"#, 1000)
            .expect("query should succeed");
        assert!(output.nodes_shown > 0, "query should return nodes");
        assert!(!output.text.is_empty(), "formatted output should have text");
    }

    #[test]
    fn test_session_file_changed() {
        let mut session = GraphSession::from_directory(fixtures_dir());
        let sample_path = fixtures_dir().join("sample.rs");

        let initial_count = session.graph.node_count();

        let modified = r#"
pub fn alpha() {
    beta();
}

fn beta() -> i32 {
    99
}
"#;
        session.file_changed(&sample_path, modified);

        // Events were recorded
        assert!(!session.events.is_empty());

        // Graph was updated — alpha and beta should exist
        assert!(!session.graph.find_by_name("alpha").is_empty());
        assert!(!session.graph.find_by_name("beta").is_empty());

        // Original nodes from sample.rs (foo, bar, etc.) should be gone
        let foo_nodes = session.graph.find_by_name("foo");
        let foo_in_sample: Vec<_> = foo_nodes
            .iter()
            .filter(|n| n.file_path == sample_path)
            .collect();
        assert!(
            foo_in_sample.is_empty(),
            "foo from sample.rs should be removed after update"
        );

        // Node count changed (sample.rs had many nodes, now only 2)
        assert_ne!(session.graph.node_count(), initial_count);
    }
}
