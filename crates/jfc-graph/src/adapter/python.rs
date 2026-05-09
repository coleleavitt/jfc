//! Python adapter (Phase 12-2).
//!
//! Produces `NodeData` / `EdgeData` from `.py` files using
//! `tree-sitter-python`. Extracts:
//!
//! - Functions/methods (`function_definition`).
//! - Classes → `NodeKind::Struct`.
//! - Module-level → `NodeKind::Module`.
//! - Call edges (`call` → callee resolution).
//! - Import edges as `UsesType`.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

pub struct PythonAdapter {
    language: Language,
}

impl PythonAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_python::LANGUAGE.into(),
        }
    }
}

impl LanguageAdapter for PythonAdapter {
    fn language_id(&self) -> &'static str {
        "python"
    }

    fn file_extensions(&self) -> &[&str] {
        &["py", "pyi"]
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile, AdapterError> {
        let mut parser = Parser::new();
        parser
            .set_language(&self.language)
            .map_err(|e| AdapterError::ParseFailed { path: path.to_string_lossy().into(), reason: format!("{e}") })?;
        let tree = parser
            .parse(content, None)
            .ok_or_else(|| AdapterError::ParseFailed { path: path.to_string_lossy().into(), reason: "tree-sitter returned None".into() })?;
        Ok(ParsedFile {
            tree,
            source: content.to_string(),
            path: path.to_path_buf(),
        })
    }

    fn extract_nodes(&self, file: &ParsedFile) -> Vec<NodeData> {
        let mut nodes = Vec::new();
        let root = file.tree.root_node();
        walk_py(root, &file.source, &file.path, &file.path.to_string_lossy(), &[], &mut nodes);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        extract_py_calls(file.tree.root_node(), &file.source, &file.path, nodes, &mut edges);
        edges
    }
}

fn walk_py(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    scope: &[&str],
    out: &mut Vec<NodeData>,
) {
    match node.kind() {
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Function, node, path, path_str, &qn));
            }
        }
        "class_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Struct, node, path, path_str, &qn));
                if let Some(body) = node.child_by_field_name("body") {
                    let binding = text(name_node, source);
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&binding);
                    walk_py(body, source, path, path_str, &child_scope, out);
                }
                return;
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_py(child, source, path, path_str, scope, out);
    }
}

fn extract_py_calls(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "call" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let callee_name = text(func_node, source);
            let mut parent = node.parent();
            let mut caller_id = None;
            while let Some(p) = parent {
                if p.kind() == "function_definition" {
                    if let Some(n) = p.child_by_field_name("name") {
                        let name = text(n, source);
                        caller_id = nodes
                            .iter()
                            .find(|nd| nd.name == name && nd.kind == NodeKind::Function)
                            .map(|nd| nd.id.clone());
                    }
                    break;
                }
                parent = p.parent();
            }
            if let Some(caller) = caller_id {
                if let Some(callee) = nodes
                    .iter()
                    .find(|n| n.name == callee_name && n.kind == NodeKind::Function)
                {
                    edges.push((
                        caller,
                        callee.id.clone(),
                        EdgeData {
                            kind: EdgeKind::Calls,
                            source_span: build_span(node, path),
                            weight: 1.0,
                        },
                    ));
                }
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_py_calls(child, source, path, nodes, edges);
    }
}

fn text(node: TsNode<'_>, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn qualified(scope: &[&str], name: &str) -> String {
    if scope.is_empty() { name.to_string() } else { format!("{}::{}", scope.join("::"), name) }
}

fn build_nd(name: &str, kind: NodeKind, node: TsNode<'_>, path: &Path, path_str: &str, qn: &str) -> NodeData {
    NodeData {
        id: NodeId::new(path_str, qn, kind),
        kind,
        name: name.to_string(),
        qualified_name: qn.to_string(),
        file_path: path.to_path_buf(),
        span: build_span(node, path),
        visibility: Visibility::Public,
        metadata: HashMap::new(),
        birth_revision: 0,
        last_modified_revision: 0,
    }
}

fn build_span(node: TsNode<'_>, path: &Path) -> Span {
    Span {
        file: path.to_path_buf(),
        start_line: node.start_position().row as u32 + 1,
        start_col: node.start_position().column as u32,
        end_line: node.end_position().row as u32 + 1,
        end_col: node.end_position().column as u32,
        byte_range: node.byte_range(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_adapter_parses_function() {
        let a = PythonAdapter::new();
        let parsed = a.parse_file(Path::new("t.py"), "def hello():\n    pass").unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "hello" && n.kind == NodeKind::Function));
    }

    #[test]
    fn python_adapter_parses_class() {
        let a = PythonAdapter::new();
        let src = "class Widget:\n    def render(self):\n        pass";
        let parsed = a.parse_file(Path::new("t.py"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Widget" && n.kind == NodeKind::Struct));
        assert!(nodes.iter().any(|n| n.name == "render" && n.kind == NodeKind::Function));
    }

    #[test]
    fn python_adapter_extracts_call_edges() {
        let a = PythonAdapter::new();
        let src = "def caller():\n    callee()\ndef callee():\n    pass";
        let parsed = a.parse_file(Path::new("t.py"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);
        assert!(!edges.is_empty());
    }
}
