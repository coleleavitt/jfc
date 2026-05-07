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
        let mut visited_dirs = std::collections::HashSet::new();
        // Canonicalize the root so we can detect symlink cycles.
        if let Ok(canonical) = dir.canonicalize() {
            visited_dirs.insert(canonical);
        }
        Self::walk_dir(dir, extensions, &mut files, &mut visited_dirs, 0);
        files.sort();
        files
    }

    /// Max directory recursion depth to prevent runaway traversal on
    /// deeply nested source trees (e.g. vendored Rust compiler builds
    /// with symlink cycles like `stage2/lib/rustlib/src/rust → /`).
    const MAX_WALK_DEPTH: usize = 32;

    fn should_skip_dir(path: &Path) -> bool {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        matches!(
            name,
            "target"
                | "node_modules"
                | ".git"
                | "build"
                | "dist"
                | "vendor"
                | ".cargo"
                | "__pycache__"
                | ".tox"
                | "venv"
                | ".venv"
                | "research"
        )
    }

    fn walk_dir(
        dir: &Path,
        extensions: &[&str],
        files: &mut Vec<PathBuf>,
        visited_dirs: &mut std::collections::HashSet<PathBuf>,
        depth: usize,
    ) {
        if depth >= Self::MAX_WALK_DEPTH {
            tracing::debug!(
                path = %dir.display(),
                depth,
                "walk_dir: max depth reached, skipping"
            );
            return;
        }

        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // Skip directories that are never useful for code graph indexing.
                // Mirrors rustc tidy's filter_dirs (src/tools/tidy/src/walk.rs:11-39).
                if Self::should_skip_dir(&path) {
                    continue;
                }
                // Resolve symlinks to detect cycles (like Linux's ELOOP on
                // total_link_count >= MAXSYMLINKS in fs/namei.c:1977).
                // If canonicalize fails (dangling symlink, permission error)
                // skip the directory entirely.
                let canonical = match path.canonicalize() {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if !visited_dirs.insert(canonical) {
                    tracing::debug!(
                        path = %path.display(),
                        "walk_dir: symlink cycle detected, skipping"
                    );
                    continue;
                }
                Self::walk_dir(&path, extensions, files, visited_dirs, depth + 1);
            } else if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if extensions.contains(&ext) {
                    files.push(path);
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
