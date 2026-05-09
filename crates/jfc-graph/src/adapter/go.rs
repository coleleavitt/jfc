//! Go adapter (Phase 12-3).
//!
//! Produces `NodeData` / `EdgeData` from `.go` files using
//! `tree-sitter-go`. Extracts:
//!
//! - Functions (`function_declaration`, `method_declaration`).
//! - Structs (`type_declaration` with struct type) → `NodeKind::Struct`.
//! - Interfaces → `NodeKind::Trait`.
//! - Packages → `NodeKind::Module`.
//! - Call edges (`call_expression`).

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

pub struct GoAdapter {
    language: Language,
}

impl GoAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_go::LANGUAGE.into(),
        }
    }
}

impl LanguageAdapter for GoAdapter {
    fn language_id(&self) -> &'static str {
        "go"
    }

    fn file_extensions(&self) -> &[&str] {
        &["go"]
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
        walk_go(root, &file.source, &file.path, &file.path.to_string_lossy(), &mut nodes);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        extract_go_calls(file.tree.root_node(), &file.source, &file.path, nodes, &mut edges);
        edges
    }
}

fn walk_go(node: TsNode<'_>, source: &str, path: &Path, path_str: &str, out: &mut Vec<NodeData>) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                out.push(build_nd(&name, NodeKind::Function, node, path, path_str, &name));
            }
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                // Receiver type is the "scope" for qualified naming.
                let receiver = node
                    .child_by_field_name("receiver")
                    .and_then(|r| {
                        let mut c = r.walk();
                        r.named_children(&mut c)
                            .find(|ch| ch.kind() == "parameter_declaration")
                            .and_then(|pd| pd.child_by_field_name("type"))
                            .map(|t| text(t, source))
                    })
                    .unwrap_or_default();
                let qn = if receiver.is_empty() {
                    name.clone()
                } else {
                    format!("{receiver}::{name}")
                };
                out.push(build_nd(&name, NodeKind::Function, node, path, path_str, &qn));
            }
        }
        "type_declaration" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "type_spec" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = text(name_node, source);
                        if let Some(type_node) = child.child_by_field_name("type") {
                            let kind = match type_node.kind() {
                                "struct_type" => NodeKind::Struct,
                                "interface_type" => NodeKind::Trait,
                                _ => continue,
                            };
                            out.push(build_nd(&name, kind, child, path, path_str, &name));
                        }
                    }
                }
            }
        }
        "package_clause" => {
            // tree-sitter-go: the package name is a direct child
            // identifier, not always accessed via field_name("name").
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "package_identifier" {
                    let name = text(child, source);
                    out.push(build_nd(&name, NodeKind::Module, node, path, path_str, &name));
                    break;
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_go(child, source, path, path_str, out);
    }
}

fn extract_go_calls(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "call_expression" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let callee_name = text(func_node, source);
            // Find enclosing function.
            let mut parent = node.parent();
            let mut caller_id = None;
            while let Some(p) = parent {
                if matches!(p.kind(), "function_declaration" | "method_declaration") {
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
        extract_go_calls(child, source, path, nodes, edges);
    }
}

fn text(node: TsNode<'_>, source: &str) -> String {
    source[node.byte_range()].to_string()
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
    fn go_adapter_parses_function() {
        let a = GoAdapter::new();
        let src = "package main\nfunc hello() {}";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "hello" && n.kind == NodeKind::Function));
    }

    #[test]
    fn go_adapter_parses_struct() {
        let a = GoAdapter::new();
        let src = "package main\ntype Point struct { X int; Y int }";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Point" && n.kind == NodeKind::Struct));
    }

    #[test]
    fn go_adapter_parses_interface() {
        let a = GoAdapter::new();
        let src = "package main\ntype Reader interface { Read(p []byte) (int, error) }";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Reader" && n.kind == NodeKind::Trait));
    }

    #[test]
    fn go_adapter_parses_method() {
        let a = GoAdapter::new();
        let src = "package main\ntype S struct{}\nfunc (s *S) Do() {}";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Do" && n.kind == NodeKind::Function));
    }

    #[test]
    fn go_adapter_extracts_call_edges() {
        let a = GoAdapter::new();
        let src = "package main\nfunc caller() { callee() }\nfunc callee() {}";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);
        assert!(!edges.is_empty());
    }

    #[test]
    fn go_adapter_parses_package() {
        let a = GoAdapter::new();
        let src = "package mylib\nfunc Foo() {}";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "mylib" && n.kind == NodeKind::Module));
    }
}
