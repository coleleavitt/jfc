//! Go adapter (Phase 12-3).
//!
//! Produces `NodeData` / `EdgeData` from `.go` files using
//! `tree-sitter-go`. Extracts:
//!
//! - Functions (`function_declaration`, `method_declaration`).
//! - Structs (`type_declaration` with struct type) → `NodeKind::Struct`.
//!   Struct fields are emitted as `NodeKind::Field` children.
//! - Interfaces → `NodeKind::Trait`.
//! - Packages → `NodeKind::Module`.
//! - Type aliases (`type Foo = Bar`) → `NodeKind::TypeAlias`. Named-type
//!   declarations (`type Distance int`) also emit a `NodeKind::TypeAlias`
//!   since Go has no separate "alias for an existing scalar" node kind.
//! - Top-level constants (`const X = …` and `const ( … )` groups) →
//!   `NodeKind::Constant`. (Go has no enums; `const`-of-named-int groups are
//!   the idiomatic substitute and surface as constants here.)
//! - Call edges (`call_expression`).

use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind};

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
            .map_err(|e| AdapterError::ParseFailed {
                path: path.to_string_lossy().into(),
                reason: format!("{e}"),
            })?;
        let tree = parser
            .parse(content, None)
            .ok_or_else(|| AdapterError::ParseFailed {
                path: path.to_string_lossy().into(),
                reason: "tree-sitter returned None".into(),
            })?;
        Ok(ParsedFile {
            tree,
            source: content.to_string(),
            path: path.to_path_buf(),
        })
    }

    fn extract_nodes(&self, file: &ParsedFile) -> Vec<NodeData> {
        let mut nodes = Vec::new();
        let root = file.tree.root_node();
        walk_go(
            root,
            &file.source,
            &file.path,
            &file.path.to_string_lossy(),
            &mut nodes,
        );
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        extract_go_calls(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &mut edges,
        );
        edges
    }
}

fn walk_go(node: TsNode<'_>, source: &str, path: &Path, path_str: &str, out: &mut Vec<NodeData>) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &name);
                nd.complexity = compute_complexity(node, source.as_bytes(), "go");
                out.push(nd);
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
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &qn);
                nd.complexity = compute_complexity(node, source.as_bytes(), "go");
                out.push(nd);
            }
        }
        "type_declaration" => {
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                match child.kind() {
                    "type_spec" => {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let name = text(name_node, source);
                            if let Some(type_node) = child.child_by_field_name("type") {
                                let kind = match type_node.kind() {
                                    "struct_type" => NodeKind::Struct,
                                    "interface_type" => NodeKind::Trait,
                                    // `type Distance int` etc. — no
                                    // dedicated NodeKind, surface as a
                                    // TypeAlias so callers can find it.
                                    _ => NodeKind::TypeAlias,
                                };
                                out.push(build_nd(&name, kind, child, path, path_str, &name));
                                // Emit Field nodes for struct members so
                                // we don't lose them between the type and
                                // the field_declaration_list child.
                                if kind == NodeKind::Struct {
                                    extract_go_struct_fields(
                                        type_node, source, path, path_str, &name, out,
                                    );
                                }
                            }
                        }
                    }
                    // `type Foo = Bar` is a dedicated `type_alias` node.
                    "type_alias" => {
                        if let Some(name_node) = child.child_by_field_name("name") {
                            let name = text(name_node, source);
                            out.push(build_nd(
                                &name,
                                NodeKind::TypeAlias,
                                child,
                                path,
                                path_str,
                                &name,
                            ));
                        }
                    }
                    _ => {}
                }
            }
        }
        "const_declaration" => {
            // const_declaration contains one or more `const_spec` children.
            // Each spec may declare multiple identifiers (`const A, B = 1, 2`).
            let mut cursor = node.walk();
            for spec in node.named_children(&mut cursor) {
                if spec.kind() != "const_spec" {
                    continue;
                }
                let mut spec_cursor = spec.walk();
                for child in spec.named_children(&mut spec_cursor) {
                    if child.kind() == "identifier" {
                        let name = text(child, source);
                        out.push(build_nd(
                            &name,
                            NodeKind::Constant,
                            spec,
                            path,
                            path_str,
                            &name,
                        ));
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
                    out.push(build_nd(
                        &name,
                        NodeKind::Module,
                        node,
                        path,
                        path_str,
                        &name,
                    ));
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

/// Emit `NodeKind::Field` for each `field_declaration` in a `struct_type`.
fn extract_go_struct_fields(
    struct_type_node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    struct_name: &str,
    out: &mut Vec<NodeData>,
) {
    // struct_type → field_declaration_list → field_declaration*
    let Some(field_list) = struct_type_node
        .named_child(0)
        .filter(|n| n.kind() == "field_declaration_list")
    else {
        return;
    };
    let mut cursor = field_list.walk();
    for field in field_list.named_children(&mut cursor) {
        if field.kind() != "field_declaration" {
            continue;
        }
        // `field_declaration` children: field_identifier+ type_identifier
        let mut fc = field.walk();
        for child in field.named_children(&mut fc) {
            if child.kind() == "field_identifier" {
                let name = text(child, source);
                let qn = format!("{struct_name}::{name}");
                let mut nd = build_nd(&name, NodeKind::Field, child, path, path_str, &qn);
                // Use the field_declaration span so go-to-def shows the
                // whole `X int` line rather than just the identifier.
                nd.span = build_span(field, path);
                out.push(nd);
            }
        }
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

use super::{build_nd, build_span, node_text as text};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn go_adapter_parses_function() {
        let a = GoAdapter::new();
        let src = "package main\nfunc hello() {}";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "hello" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn go_adapter_parses_struct() {
        let a = GoAdapter::new();
        let src = "package main\ntype Point struct { X int; Y int }";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Point" && n.kind == NodeKind::Struct)
        );
    }

    #[test]
    fn go_adapter_parses_interface() {
        let a = GoAdapter::new();
        let src = "package main\ntype Reader interface { Read(p []byte) (int, error) }";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Reader" && n.kind == NodeKind::Trait)
        );
    }

    #[test]
    fn go_adapter_parses_method() {
        let a = GoAdapter::new();
        let src = "package main\ntype S struct{}\nfunc (s *S) Do() {}";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Do" && n.kind == NodeKind::Function)
        );
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
    fn go_adapter_extracts_struct_fields() {
        let a = GoAdapter::new();
        let src = "package main\ntype Point struct {\n  X int\n  Y int\n  Name string\n}";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "X" && n.kind == NodeKind::Field),
            "expected Field 'X', got: {:?}",
            nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Field)
                .collect::<Vec<_>>()
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Y" && n.kind == NodeKind::Field)
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Name" && n.kind == NodeKind::Field)
        );
        // qualified naming should namespace under the struct.
        assert!(
            nodes
                .iter()
                .any(|n| n.qualified_name == "Point::X" && n.kind == NodeKind::Field),
            "Field qualified name should be Point::X"
        );
    }

    #[test]
    fn go_adapter_extracts_const_declarations() {
        let a = GoAdapter::new();
        let src = "package main\nconst MaxSize = 100\nconst Pi float64 = 3.14\nconst (\n  A = 1\n  B = 2\n)";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        for name in &["MaxSize", "Pi", "A", "B"] {
            assert!(
                nodes
                    .iter()
                    .any(|n| n.name == *name && n.kind == NodeKind::Constant),
                "expected Constant '{name}'"
            );
        }
    }

    #[test]
    fn go_adapter_extracts_type_alias() {
        let a = GoAdapter::new();
        // `type ID = string` is an alias; `type Distance int` is a named type.
        // Both surface as TypeAlias for query convenience.
        let src = "package main\ntype ID = string\ntype Distance int";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "ID" && n.kind == NodeKind::TypeAlias),
            "expected TypeAlias 'ID'"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Distance" && n.kind == NodeKind::TypeAlias),
            "expected TypeAlias 'Distance' (named type)"
        );
    }

    #[test]
    fn go_adapter_parses_package() {
        let a = GoAdapter::new();
        let src = "package mylib\nfunc Foo() {}";
        let parsed = a.parse_file(Path::new("t.go"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "mylib" && n.kind == NodeKind::Module)
        );
    }
}
