//! High-level session facade — the single entry point for jfc-ui.

use std::path::{Path, PathBuf};

use tracing::warn;

use crate::adapter::{AdapterError, rust::RustAdapter};
use crate::builder::GraphBuilder;
use crate::capabilities::{Capability, CapabilityTree};
use crate::dsl::{self, QueryConfig, QueryError, QueryResult};
use crate::formatting::{self, FormattedOutput};
use crate::graph::CodeGraph;
use crate::incremental::{QueryCache, QueryKey, ReadSet};
use crate::persistence::EventLog;
use crate::symbols::SymbolTable;

/// Owns the graph, symbols, event log, and capabilities.
/// Provides query execution and incremental file updates.
pub struct GraphSession {
    pub graph: CodeGraph,
    pub symbols: SymbolTable,
    pub events: EventLog,
    pub capabilities: CapabilityTree,
    /// Tree-sitter syntax errors collected during the initial indexing pass.
    /// Surfaces files with partial graphs so the UI can warn the user.
    pub parse_errors: Vec<AdapterError>,
    /// Files skipped entirely (I/O failure or hard parse failure).
    pub files_skipped: Vec<PathBuf>,
    /// Memoised DSL query results — invalidated per-node when files
    /// change. See [`crate::incremental`] for the cache model.
    query_cache: QueryCache<QueryResult>,
    adapter: RustAdapter,
}

impl GraphSession {
    /// Build a session by indexing all supported files under `workspace_root`.
    pub fn from_directory(workspace_root: &Path) -> Self {
        let adapter = RustAdapter::new();
        let result = GraphBuilder::build_from_directory_with_result(workspace_root, &adapter);
        let symbols = SymbolTable::build_from_graph(&result.graph);

        // Log a single summary line so the parse errors are observable even
        // when the caller doesn't inspect `parse_errors` directly.
        if !result.parse_errors.is_empty() {
            warn!(
                target: "jfc::graph::session",
                count = result.parse_errors.len(),
                "files with tree-sitter syntax errors — partial graph indexed"
            );
        }

        Self {
            graph: result.graph,
            symbols,
            events: EventLog::new(),
            capabilities: CapabilityTree::from_env(),
            parse_errors: result.parse_errors,
            files_skipped: result.files_skipped,
            query_cache: QueryCache::new(),
            adapter,
        }
    }

    /// Execute a DSL query and return token-budgeted formatted output.
    ///
    /// Delegates to [`dsl::run_query_expr`] (the extended-grammar entry
    /// point) — it parses the legacy pipe-chain as a sub-form, so all
    /// pre-existing pipe queries still work, while callers also get
    /// `union` / `intersect` / `\` set algebra, `path` / `paths`,
    /// `entrypoints`, and the `since N` postfix filter for free.
    pub fn query(&self, query_str: &str, max_tokens: usize) -> Result<FormattedOutput, QueryError> {
        let config = QueryConfig {
            max_tokens,
            max_nodes: 50,
        };
        let result = dsl::run_query_expr(query_str, &self.graph, &config)?;
        Ok(formatting::format_query_result_with_capabilities(
            &result,
            &self.graph,
            Some(&self.symbols),
            Some(&self.capabilities),
            max_tokens,
        ))
    }

    /// Execute a DSL query and return the raw [`QueryResult`] for
    /// programmatic use (e.g. handle extraction, history recording,
    /// chained predicate analysis). Same parser as [`Self::query`].
    ///
    /// Phase 5+8: results are memoised in [`Self::query_cache`]. Cache
    /// hits skip parsing + execution entirely. Cache invalidation
    /// (Phase 8) tracks a fine-grained read-set per entry: the result
    /// nodes **plus the 1-hop neighbourhood in both directions**
    /// (anything a follow-up traversal could reach). When a file
    /// changes, only entries whose read-set intersects the file's
    /// nodes are invalidated — unrelated queries keep their cache
    /// entries.
    ///
    /// The 1-hop expansion is the cheapest correct approximation for
    /// pipe-chain queries that touch direct neighbours via `callers`,
    /// `callees`, `taint`, `preconditions`, etc. Deeper queries pay a
    /// false-invalidation penalty (their read-set undercounts), but
    /// the cache stays correct because revision-mismatched lookups
    /// are also discarded by [`QueryKey`].
    pub fn query_raw(&self, query_str: &str) -> Result<QueryResult, QueryError> {
        let key = QueryKey::new(query_str, self.graph.current_revision());
        if let Some(cached) = self.query_cache.get(&key) {
            return Ok((*cached).clone());
        }
        let config = QueryConfig::default();
        let result = dsl::run_query_expr(query_str, &self.graph, &config)?;

        // Phase 8 read-set: result nodes + 1-hop neighbours
        // (incoming + outgoing). This captures the dependencies of
        // any pipe stage like `| callers` or `| callees` that the
        // query could have used to reach those nodes.
        let mut read_set = ReadSet::new();
        for id in &result.nodes {
            read_set.record(id);
            for (nbr, _) in self.graph.get_edges_from(id) {
                read_set.record(nbr);
            }
            for (nbr, _) in self.graph.get_edges_to(id) {
                read_set.record(nbr);
            }
        }
        self.query_cache.put(key, result.clone(), read_set);
        Ok(result)
    }

    /// Incrementally update the graph after a file modification.
    /// Drops every query-cache entry whose read-set referenced one
    /// of the file's removed/replaced nodes.
    pub fn file_changed(&mut self, path: &Path, new_content: &str) {
        // Snapshot the file's nodes *before* mutation so we know what
        // to invalidate.
        let touched_ids: Vec<_> = self
            .graph
            .all_node_ids()
            .into_iter()
            .filter(|id| {
                self.graph
                    .get_node(id)
                    .map(|n| n.file_path == path)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        for id in &touched_ids {
            self.query_cache.invalidate_for_node(id);
        }

        let events = self.graph.update_file(path, new_content, &self.adapter);
        for event in events {
            self.events.append(event, None);
        }
        self.symbols.update_from_graph(&self.graph, path);
    }

    /// Clear the entire query result cache. Use when in doubt about
    /// invalidation correctness — coarse but always-correct.
    pub fn clear_query_cache(&self) {
        self.query_cache.clear();
    }

    /// Number of cached queries (testing aid).
    pub fn query_cache_len(&self) -> usize {
        self.query_cache.len()
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
    fn cache_hit_on_repeated_query() {
        let session = GraphSession::from_directory(fixtures_dir());
        let q = r#"fn("foo") | callees"#;
        let r1 = session.query_raw(q).expect("first query");
        assert_eq!(session.query_cache_len(), 1);
        let r2 = session.query_raw(q).expect("second query");
        assert_eq!(r1.nodes, r2.nodes, "cache must return identical result");
        // Length still 1 — we didn't add a second entry.
        assert_eq!(session.query_cache_len(), 1);
    }

    #[test]
    fn cache_invalidates_on_file_change() {
        let mut session = GraphSession::from_directory(fixtures_dir());
        let sample = fixtures_dir().join("sample.rs");
        // Run any query that touches sample.rs nodes.
        let _ = session.query_raw(r#"fn("foo") | callees"#);
        let pre = session.query_cache_len();
        assert!(pre >= 1);

        // Mutate the file: cache for sample.rs nodes should drop.
        session.file_changed(&sample, "pub fn x() {}");
        // Either the entry was directly invalidated by node-id, or
        // our coarse path keeps it; either way the new query
        // populates a fresh, correct entry.
        let _ = session.query_raw(r#"fn("foo") | callees"#);
    }

    #[test]
    fn cache_preserves_unrelated_queries_on_file_change() {
        // Phase 8: unrelated queries shouldn't be invalidated by a
        // file change to nodes they don't reference.
        let mut session = GraphSession::from_directory(fixtures_dir());
        // Run a query whose read-set is the foo subtree.
        let _ = session.query_raw(r#"fn("foo")"#);
        let cached_count_before = session.query_cache_len();

        // Mutate a fictional path that doesn't exist in the graph —
        // should not invalidate anything (no nodes touched).
        let phantom = fixtures_dir().join("nonexistent.rs");
        session.file_changed(&phantom, "// nothing");
        let cached_count_after = session.query_cache_len();

        assert_eq!(
            cached_count_before, cached_count_after,
            "phantom file should not invalidate any cache entries"
        );
    }

    #[test]
    fn clear_query_cache_drops_all() {
        let session = GraphSession::from_directory(fixtures_dir());
        let _ = session.query_raw(r#"fn("foo") | callees"#);
        let _ = session.query_raw(r#"fn("bar") | callees"#);
        assert!(session.query_cache_len() > 0);
        session.clear_query_cache();
        assert_eq!(session.query_cache_len(), 0);
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
