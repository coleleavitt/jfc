//! Language adapter trait and registry for feeding parsed code into the graph engine.
//!
//! Each supported language implements [`LanguageAdapter`] to parse files and extract
//! nodes/edges. The [`AdapterRegistry`] maps file extensions to the appropriate adapter.

pub mod rust;
pub mod typescript;
pub mod python;
pub mod go;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::edges::EdgeData;
use crate::nodes::{NodeData, NodeId};

/// Parsed file representation — holds tree-sitter Tree + source.
pub struct ParsedFile {
    pub path: PathBuf,
    pub source: String,
    pub tree: tree_sitter::Tree,
}

/// Language adapter trait — implement for each supported language.
///
/// Adapters are responsible for parsing source files and extracting structural
/// information (nodes and edges) that feeds into the code graph.
pub trait LanguageAdapter: Send + Sync {
    /// Language identifier (e.g., "rust", "typescript").
    fn language_id(&self) -> &str;

    /// File extensions this adapter handles (e.g., `&["rs"]`).
    fn file_extensions(&self) -> &[&str];

    /// Parse a file into a tree-sitter Tree.
    ///
    /// Returns [`AdapterError::SyntaxError`] if tree-sitter produced any
    /// `ERROR` (or `MISSING`) nodes. Callers who want a best-effort partial
    /// tree even on syntax errors should use [`Self::parse_file_lenient`].
    fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile, AdapterError>;

    /// Parse a file, returning a [`ParseOutcome`] that carries both the tree
    /// and (if applicable) a [`AdapterError::SyntaxError`].
    ///
    /// This is the preferred entry point for the [`crate::builder::GraphBuilder`]:
    /// it lets the builder index a partial graph from broken files while
    /// surfacing the syntax error to the caller, instead of silently dropping
    /// the whole file.
    ///
    /// The default implementation calls `parse_file` and flattens the error
    /// case — adapters that produce a tree before validating can override
    /// this for better partial-recovery semantics.
    fn parse_file_lenient(
        &self,
        path: &Path,
        content: &str,
    ) -> Result<ParseOutcome, AdapterError> {
        match self.parse_file(path, content) {
            Ok(parsed) => Ok(ParseOutcome::ok(parsed)),
            // Only `SyntaxError` is recoverable here — the tree was produced,
            // we just want the caller to know it has ERROR nodes. Hard
            // failures (e.g. tree-sitter returned None) bubble up unchanged.
            Err(AdapterError::SyntaxError { .. }) => {
                // Default impl can't recover the partial tree without
                // re-parsing — call sites that care should override.
                self.parse_file(path, content).map(ParseOutcome::ok)
            }
            Err(other) => Err(other),
        }
    }

    /// Extract nodes (functions, structs, etc.) from a parsed file.
    fn extract_nodes(&self, parsed: &ParsedFile) -> Vec<NodeData>;

    /// Extract edges (calls, uses_type, etc.) from a parsed file given known nodes.
    fn extract_edges(
        &self,
        parsed: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)>;
}

/// Adapter errors.
#[derive(Debug, thiserror::Error)]
pub enum AdapterError {
    #[error("parse failed for {path}: {reason}")]
    ParseFailed { path: String, reason: String },

    #[error("unsupported file extension: {ext}")]
    UnsupportedExtension { ext: String },

    /// Tree-sitter produced a tree containing one or more `ERROR` nodes —
    /// the source has syntax errors and the parsed graph is partial.
    ///
    /// The adapter still surfaces a usable `ParsedFile` to the caller via
    /// `ParseOutcome::Partial`, but standalone callers (anything that
    /// expects `Result<ParsedFile, _>`) get this error so silent corruption
    /// becomes impossible.
    #[error("syntax error in {path} at bytes {start}..{end}: {summary}")]
    SyntaxError {
        path: PathBuf,
        start: usize,
        end: usize,
        summary: String,
    },
}

/// Result of parsing — either a clean tree, or a tree plus a syntax error.
///
/// The builder uses this to drive a continue-with-partial-graph policy:
/// syntax errors are collected per-file rather than aborting the whole
/// indexing pass.
pub struct ParseOutcome {
    pub parsed: ParsedFile,
    pub error: Option<AdapterError>,
}

impl ParseOutcome {
    pub fn ok(parsed: ParsedFile) -> Self {
        Self {
            parsed,
            error: None,
        }
    }

    pub fn partial(parsed: ParsedFile, error: AdapterError) -> Self {
        Self {
            parsed,
            error: Some(error),
        }
    }
}

/// Walk a tree-sitter tree to find the first ERROR (or MISSING) node and
/// summarise it for a typed [`AdapterError::SyntaxError`].
///
/// Returns `None` if the tree is clean.
pub fn first_syntax_error(
    tree: &tree_sitter::Tree,
    path: &Path,
    source: &str,
) -> Option<AdapterError> {
    if !tree.root_node().has_error() {
        return None;
    }

    // DFS for the first node with `is_error()` or `is_missing()`.
    let mut stack: Vec<tree_sitter::Node<'_>> = vec![tree.root_node()];
    while let Some(node) = stack.pop() {
        if node.is_error() || node.is_missing() {
            let range = node.byte_range();
            let snippet = source
                .get(range.start..range.end.min(range.start + 80))
                .unwrap_or("")
                .replace('\n', " ");
            let summary = if node.is_missing() {
                format!("missing `{}`", node.kind())
            } else if snippet.is_empty() {
                format!(
                    "ERROR node ({} at line {})",
                    node.kind(),
                    node.start_position().row + 1
                )
            } else {
                format!("ERROR near `{snippet}`")
            };
            return Some(AdapterError::SyntaxError {
                path: path.to_path_buf(),
                start: range.start,
                end: range.end,
                summary,
            });
        }
        // Push children for DFS — but only descend if the subtree contains errors,
        // to keep this O(error-locality) rather than O(tree size).
        if node.has_error() {
            let mut cursor = node.walk();
            let children: Vec<_> = node.children(&mut cursor).collect();
            // Reverse so the leftmost child is popped first (DFS in source order).
            for child in children.into_iter().rev() {
                stack.push(child);
            }
        }
    }

    None
}

/// Registry that maps file extensions to language adapters.
///
/// Stores adapters by `language_id` and maintains a separate extension → language_id
/// lookup table, allowing one adapter to serve multiple file extensions.
pub struct AdapterRegistry {
    adapters: HashMap<String, Arc<dyn LanguageAdapter>>,
    extension_map: HashMap<String, String>,
}

impl AdapterRegistry {
    pub fn new() -> Self {
        Self {
            adapters: HashMap::new(),
            extension_map: HashMap::new(),
        }
    }

    /// Register an adapter for its declared extensions.
    pub fn register(&mut self, adapter: impl LanguageAdapter + 'static) {
        let lang_id = adapter.language_id().to_string();
        let extensions: Vec<String> = adapter
            .file_extensions()
            .iter()
            .map(|e| e.to_string())
            .collect();

        let arc = Arc::new(adapter);
        self.adapters.insert(lang_id.clone(), arc);

        for ext in extensions {
            self.extension_map.insert(ext, lang_id.clone());
        }
    }

    /// Look up an adapter by file extension (without the leading dot).
    pub fn get_by_extension(&self, ext: &str) -> Option<&dyn LanguageAdapter> {
        let lang_id = self.extension_map.get(ext)?;
        self.adapters.get(lang_id).map(|a| a.as_ref())
    }

    /// Look up an adapter by language identifier.
    pub fn get_by_language(&self, lang_id: &str) -> Option<&dyn LanguageAdapter> {
        self.adapters.get(lang_id).map(|a| a.as_ref())
    }
}

impl Default for AdapterRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MockAdapter;

    impl LanguageAdapter for MockAdapter {
        fn language_id(&self) -> &str {
            "mock"
        }

        fn file_extensions(&self) -> &[&str] {
            &["mock", "mk"]
        }

        fn parse_file(&self, path: &Path, _content: &str) -> Result<ParsedFile, AdapterError> {
            Err(AdapterError::ParseFailed {
                path: path.display().to_string(),
                reason: "mock adapter does not parse".into(),
            })
        }

        fn extract_nodes(&self, _parsed: &ParsedFile) -> Vec<NodeData> {
            vec![]
        }

        fn extract_edges(
            &self,
            _parsed: &ParsedFile,
            _nodes: &[NodeData],
        ) -> Vec<(NodeId, NodeId, EdgeData)> {
            vec![]
        }
    }

    #[test]
    fn test_adapter_registry_register() {
        let mut registry = AdapterRegistry::new();
        registry.register(MockAdapter);

        let adapter = registry.get_by_language("mock");
        assert!(adapter.is_some());
        assert_eq!(adapter.unwrap().language_id(), "mock");
    }

    #[test]
    fn test_adapter_registry_extension_lookup() {
        let mut registry = AdapterRegistry::new();
        registry.register(MockAdapter);

        let by_mock = registry.get_by_extension("mock");
        assert!(by_mock.is_some());
        assert_eq!(by_mock.unwrap().language_id(), "mock");

        let by_mk = registry.get_by_extension("mk");
        assert!(by_mk.is_some());
        assert_eq!(by_mk.unwrap().language_id(), "mock");
    }

    #[test]
    fn test_adapter_registry_unknown_extension() {
        let mut registry = AdapterRegistry::new();
        registry.register(MockAdapter);

        assert!(registry.get_by_extension("xyz").is_none());
        assert!(registry.get_by_language("unknown").is_none());
    }

    #[test]
    fn test_adapter_error_display() {
        let err = AdapterError::ParseFailed {
            path: "src/main.rs".into(),
            reason: "syntax error".into(),
        };
        assert_eq!(
            err.to_string(),
            "parse failed for src/main.rs: syntax error"
        );

        let err = AdapterError::UnsupportedExtension { ext: "xyz".into() };
        assert_eq!(err.to_string(), "unsupported file extension: xyz");
    }

    #[test]
    fn test_adapter_registry_default() {
        let registry = AdapterRegistry::default();
        assert!(registry.get_by_extension("rs").is_none());
    }
}
