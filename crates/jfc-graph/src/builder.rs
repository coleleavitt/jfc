use std::path::{Path, PathBuf};

use tracing::warn;

use crate::adapter::{AdapterError, LanguageAdapter};
use crate::graph::CodeGraph;

/// Result of a build pass — the graph plus diagnostics that the caller should
/// surface (rather than silently swallow).
///
/// `parse_errors` is populated when tree-sitter produced ERROR/MISSING nodes
/// but a partial tree was usable; the corresponding file's nodes/edges are
/// still indexed. `files_skipped` lists files that produced no usable tree
/// (I/O failure or hard parse failure).
#[derive(Default)]
pub struct BuildResult {
    pub graph: CodeGraph,
    pub parse_errors: Vec<AdapterError>,
    pub files_skipped: Vec<PathBuf>,
}

/// Orchestrates building a [`CodeGraph`] from source files.
pub struct GraphBuilder;

impl GraphBuilder {
    /// Build graph from all matching files in a directory (recursive).
    ///
    /// Convenience wrapper that discards diagnostics — prefer
    /// [`Self::build_from_directory_with_result`] for production code.
    pub fn build_from_directory(path: &Path, adapter: &dyn LanguageAdapter) -> CodeGraph {
        Self::build_from_directory_with_result(path, adapter).graph
    }

    /// Build graph from a specific list of files.
    ///
    /// Convenience wrapper that discards diagnostics — prefer
    /// [`Self::build_from_files_with_result`] for production code.
    pub fn build_from_files(files: &[PathBuf], adapter: &dyn LanguageAdapter) -> CodeGraph {
        Self::build_from_files_with_result(files, adapter).graph
    }

    /// Build graph + diagnostics from all matching files in a directory.
    pub fn build_from_directory_with_result(
        path: &Path,
        adapter: &dyn LanguageAdapter,
    ) -> BuildResult {
        let files = Self::discover_files(path, adapter.file_extensions());
        Self::build_from_files_with_result(&files, adapter)
    }

    /// Build graph + diagnostics from a specific list of files.
    ///
    /// Files with tree-sitter syntax errors are indexed with their *partial*
    /// tree (so the user still gets best-effort symbols) and the
    /// [`AdapterError::SyntaxError`] is collected into `parse_errors`. Files
    /// that fail to read or hard-fail parsing land in `files_skipped`.
    pub fn build_from_files_with_result(
        files: &[PathBuf],
        adapter: &dyn LanguageAdapter,
    ) -> BuildResult {
        let mut result = BuildResult::default();

        for file_path in files {
            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to read {}: {}", file_path.display(), e);
                    result.files_skipped.push(file_path.clone());
                    continue;
                }
            };

            let outcome = match adapter.parse_file_lenient(file_path, &content) {
                Ok(o) => o,
                Err(e) => {
                    warn!("Failed to parse {}: {}", file_path.display(), e);
                    result.files_skipped.push(file_path.clone());
                    continue;
                }
            };

            if let Some(err) = outcome.error {
                // Partial tree — index what we have but record the error so
                // the caller can surface it (CLI warning, LSP diagnostic, etc.).
                result.parse_errors.push(err);
            }

            let parsed = outcome.parsed;
            let nodes = adapter.extract_nodes(&parsed);
            for node in &nodes {
                result.graph.add_node(node.clone());
            }

            let edges = adapter.extract_edges(&parsed, &nodes);
            for (from, to, edge_data) in edges {
                if result.graph.contains_node(&from) && result.graph.contains_node(&to) {
                    if let Err(e) = result.graph.add_edge(&from, &to, edge_data) {
                        // Edge-invariant violations are a builder/adapter bug,
                        // not user-facing, so we log loudly.
                        warn!(
                            target: "jfc::graph::builder",
                            error = %e,
                            "edge rejected by graph invariant — adapter produced a malformed edge"
                        );
                    }
                }
            }
        }

        result
    }

    fn discover_files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
        use ignore::WalkBuilder;

        let mut files = Vec::new();
        let walker = WalkBuilder::new(dir)
            .hidden(true)
            .git_ignore(true)
            .git_global(true)
            .git_exclude(true)
            .follow_links(false)
            .max_depth(Some(32))
            .build();

        for entry in walker.flatten() {
            let path = entry.path();
            if path.is_file() {
                if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if extensions.contains(&ext) {
                        files.push(path.to_path_buf());
                    }
                }
            }
        }

        files.sort();
        files
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::adapter::rust::RustAdapter;

    fn fixtures_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
    }

    #[test]
    fn test_build_from_fixtures_dir() {
        let adapter = RustAdapter::new();
        let graph = GraphBuilder::build_from_directory(&fixtures_dir(), &adapter);

        assert!(
            graph.node_count() > 0,
            "expected nodes from fixture files, got 0"
        );

        let all_ids = graph.all_node_ids();
        assert!(
            all_ids.len() >= 5,
            "expected at least 5 nodes from fixtures, got {}",
            all_ids.len()
        );

        assert!(
            graph.edge_count() > 0,
            "expected edges from fixture files, got 0"
        );
    }

    #[test]
    fn test_build_from_single_file() {
        let adapter = RustAdapter::new();
        let sample = fixtures_dir().join("sample.rs");
        let graph = GraphBuilder::build_from_files(&[sample], &adapter);

        assert!(
            graph.node_count() >= 8,
            "expected at least 8 nodes from sample.rs, got {}",
            graph.node_count()
        );

        assert!(
            graph.edge_count() > 0,
            "expected edges from sample.rs, got 0"
        );
    }

    #[test]
    fn test_build_handles_missing_file() {
        let adapter = RustAdapter::new();
        let missing = PathBuf::from("/nonexistent/path/to/file.rs");
        let valid = fixtures_dir().join("sample.rs");

        let graph = GraphBuilder::build_from_files(&[missing, valid], &adapter);

        assert!(
            graph.node_count() > 0,
            "expected nodes from valid file after skipping missing"
        );
    }

    #[test]
    fn test_build_deterministic() {
        let adapter = RustAdapter::new();
        let files = vec![
            fixtures_dir().join("sample.rs"),
            fixtures_dir().join("mutual_recursion.rs"),
            fixtures_dir().join("deep_call_chain.rs"),
        ];

        let graph1 = GraphBuilder::build_from_files(&files, &adapter);
        let graph2 = GraphBuilder::build_from_files(&files, &adapter);

        assert_eq!(
            graph1.node_count(),
            graph2.node_count(),
            "node counts differ between builds"
        );
        assert_eq!(
            graph1.edge_count(),
            graph2.edge_count(),
            "edge counts differ between builds"
        );
    }
}
