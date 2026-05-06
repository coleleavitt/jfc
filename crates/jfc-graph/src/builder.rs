use std::path::{Path, PathBuf};

use tracing::warn;

use crate::adapter::LanguageAdapter;
use crate::graph::CodeGraph;

/// Orchestrates building a [`CodeGraph`] from source files.
pub struct GraphBuilder;

impl GraphBuilder {
    /// Build graph from all matching files in a directory (recursive).
    pub fn build_from_directory(path: &Path, adapter: &dyn LanguageAdapter) -> CodeGraph {
        let files = Self::discover_files(path, adapter.file_extensions());
        Self::build_from_files(&files, adapter)
    }

    /// Build graph from a specific list of files.
    pub fn build_from_files(files: &[PathBuf], adapter: &dyn LanguageAdapter) -> CodeGraph {
        let mut graph = CodeGraph::new();

        for file_path in files {
            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to read {}: {}", file_path.display(), e);
                    continue;
                }
            };

            let parsed = match adapter.parse_file(file_path, &content) {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to parse {}: {}", file_path.display(), e);
                    continue;
                }
            };

            let nodes = adapter.extract_nodes(&parsed);
            for node in &nodes {
                graph.add_node(node.clone());
            }

            let edges = adapter.extract_edges(&parsed, &nodes);
            for (from, to, edge_data) in edges {
                if graph.contains_node(&from) && graph.contains_node(&to) {
                    let _ = graph.add_edge(&from, &to, edge_data);
                }
            }
        }

        graph
    }

    fn discover_files(dir: &Path, extensions: &[&str]) -> Vec<PathBuf> {
        let mut files = Vec::new();
        Self::walk_dir(dir, extensions, &mut files);
        files.sort();
        files
    }

    fn walk_dir(dir: &Path, extensions: &[&str], files: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    Self::walk_dir(&path, extensions, files);
                } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                    if extensions.contains(&ext) {
                        files.push(path);
                    }
                }
            }
        }
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
