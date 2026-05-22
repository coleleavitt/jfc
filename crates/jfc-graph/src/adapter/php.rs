//! PHP adapter.
//!
//! Produces `NodeData` / `EdgeData` from `.php` files using `tree-sitter-php`.
//! Extracts:
//!
//! - Functions (`function_definition`) → `NodeKind::Function`.
//! - Classes (`class_declaration`) → `NodeKind::Struct`.
//! - Interfaces (`interface_declaration`) → `NodeKind::Trait`.
//! - Enums (`enum_declaration`) → `NodeKind::Enum`.
//! - Namespaces (`namespace_definition`) → `NodeKind::Module`.
//! - Methods (`method_declaration`) → `NodeKind::Function` (qualified as `Class.method`).
//! - Call edges (`function_call_expression`, `member_call_expression`) → `EdgeKind::Calls`.
//! - Implements/extends → `EdgeKind::Implements` / `EdgeKind::UsesType`.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

pub struct PhpAdapter {
    language: Language,
}

impl PhpAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_php::LANGUAGE_PHP.into(),
        }
    }
}

impl LanguageAdapter for PhpAdapter {
    fn language_id(&self) -> &'static str {
        "php"
    }

    fn file_extensions(&self) -> &[&str] {
        &["php"]
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
        let path_str = file.path.to_string_lossy();
        walk_php(root, &file.source, &file.path, &path_str, &mut nodes, None);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        let path_str = file.path.to_string_lossy();
        extract_php_calls(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &path_str,
            &mut edges,
        );
        extract_php_hierarchy(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &path_str,
            &mut edges,
        );
        edges
    }
}

// ─── Node extraction ─────────────────────────────────────────────────────────

fn walk_php(
    node: TsNode,
    source: &str,
    file_path: &Path,
    path_str: &str,
    out: &mut Vec<NodeData>,
    enclosing_class: Option<&str>,
) {
    match node.kind() {
        "namespace_definition" => {
            if let Some(name) = child_by_field_text(&node, "name", source) {
                out.push(build_nd(&name, NodeKind::Module, node, file_path, path_str, &name));
            }
            walk_children(node, source, file_path, path_str, out, enclosing_class);
        }
        "class_declaration" => {
            if let Some(name) = child_by_field_text(&node, "name", source) {
                out.push(build_nd(&name, NodeKind::Struct, node, file_path, path_str, &name));
                // Walk children with class context
                walk_children(node, source, file_path, path_str, out, Some(&name));
                return;
            }
            walk_children(node, source, file_path, path_str, out, enclosing_class);
        }
        "interface_declaration" => {
            if let Some(name) = child_by_field_text(&node, "name", source) {
                out.push(build_nd(&name, NodeKind::Trait, node, file_path, path_str, &name));
                walk_children(node, source, file_path, path_str, out, Some(&name));
                return;
            }
            walk_children(node, source, file_path, path_str, out, enclosing_class);
        }
        "enum_declaration" => {
            if let Some(name) = child_by_field_text(&node, "name", source) {
                out.push(build_nd(&name, NodeKind::Enum, node, file_path, path_str, &name));
            }
            walk_children(node, source, file_path, path_str, out, enclosing_class);
        }
        "function_definition" => {
            if let Some(name) = child_by_field_text(&node, "name", source) {
                let qualified = match enclosing_class {
                    Some(cls) => format!("{cls}.{name}"),
                    None => name.clone(),
                };
                let mut nd = build_nd(&qualified, NodeKind::Function, node, file_path, path_str, &qualified);
                nd.complexity = compute_complexity(node, source.as_bytes(), "php");
                out.push(nd);
            }
            // Don't recurse into function body for more function defs
        }
        "method_declaration" => {
            if let Some(name) = child_by_field_text(&node, "name", source) {
                let qualified = match enclosing_class {
                    Some(cls) => format!("{cls}.{name}"),
                    None => name.clone(),
                };
                let mut nd = build_nd(&qualified, NodeKind::Function, node, file_path, path_str, &qualified);
                nd.complexity = compute_complexity(node, source.as_bytes(), "php");
                out.push(nd);
            }
        }
        _ => {
            walk_children(node, source, file_path, path_str, out, enclosing_class);
        }
    }
}

fn walk_children(
    node: TsNode,
    source: &str,
    file_path: &Path,
    path_str: &str,
    out: &mut Vec<NodeData>,
    enclosing_class: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_php(child, source, file_path, path_str, out, enclosing_class);
    }
}

// ─── Edge extraction ─────────────────────────────────────────────────────────

fn extract_php_calls(
    node: TsNode,
    source: &str,
    file_path: &Path,
    nodes: &[NodeData],
    path_str: &str,
    out: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    match node.kind() {
        "function_call_expression" => {
            if let Some(callee_name) = call_function_name(&node, source) {
                let caller = enclosing_function(node, source, nodes, file_path);
                if let Some(caller_id) = caller {
                    // Try to resolve callee
                    let callee_id = find_function_node(nodes, &callee_name)
                        .unwrap_or_else(|| NodeId::new(path_str, &callee_name, NodeKind::Function));
                    out.push((
                        caller_id,
                        callee_id,
                        EdgeData { kind: EdgeKind::Calls, source_span: build_span(node, file_path), weight: 1.0 },
                    ));
                }
            }
        }
        "member_call_expression" => {
            if let Some(method_name) = child_by_field_text(&node, "name", source) {
                let caller = enclosing_function(node, source, nodes, file_path);
                if let Some(caller_id) = caller {
                    // Try partial resolution — look for *.method_name
                    let callee_id = find_method_node(nodes, &method_name)
                        .unwrap_or_else(|| NodeId::new(path_str, &method_name, NodeKind::Function));
                    out.push((
                        caller_id,
                        callee_id,
                        EdgeData { kind: EdgeKind::Calls, source_span: build_span(node, file_path), weight: 1.0 },
                    ));
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_php_calls(child, source, file_path, nodes, path_str, out);
    }
}

fn extract_php_hierarchy(
    node: TsNode,
    source: &str,
    file_path: &Path,
    nodes: &[NodeData],
    path_str: &str,
    out: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    match node.kind() {
        "class_declaration" | "interface_declaration" => {
            let class_name = child_by_field_text(&node, "name", source);
            if let Some(ref cls) = class_name {
                let cls_id = NodeId::new(
                    path_str,
                    cls,
                    if node.kind() == "class_declaration" { NodeKind::Struct } else { NodeKind::Trait },
                );

                // base_clause → extends
                if let Some(base) = node.child_by_field_name("base_clause") {
                    extract_names_from_list(base, source, |name| {
                        let target_id = find_struct_or_trait(nodes, &name)
                            .unwrap_or_else(|| NodeId::new(path_str, &name, NodeKind::Struct));
                        out.push((
                            cls_id.clone(),
                            target_id,
                            EdgeData { kind: EdgeKind::UsesType, source_span: build_span(node, file_path), weight: 1.0 },
                        ));
                    });
                }

                // class_interface_clause → implements
                let mut cursor = node.walk();
                for child in node.children(&mut cursor) {
                    if child.kind() == "class_interface_clause" {
                        extract_names_from_list(child, source, |name| {
                            let target_id = find_struct_or_trait(nodes, &name)
                                .unwrap_or_else(|| NodeId::new(path_str, &name, NodeKind::Trait));
                            out.push((
                                cls_id.clone(),
                                target_id,
                                EdgeData { kind: EdgeKind::Implements, source_span: build_span(node, file_path), weight: 1.0 },
                            ));
                        });
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_php_hierarchy(child, source, file_path, nodes, path_str, out);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn build_nd(
    name: &str,
    kind: NodeKind,
    node: TsNode,
    file_path: &Path,
    path_str: &str,
    qualified_name: &str,
) -> NodeData {
    NodeData {
        id: NodeId::new(path_str, qualified_name, kind),
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        kind,
        file_path: file_path.to_path_buf(),
        span: Span {
            file: file_path.to_path_buf(),
            start_line: node.start_position().row as u32 + 1,
            start_col: node.start_position().column as u32,
            end_line: node.end_position().row as u32 + 1,
            end_col: node.end_position().column as u32,
            byte_range: node.byte_range(),
        },
        visibility: Visibility::Public,
        complexity: None,
        metadata: HashMap::new(),
        birth_revision: 0,
        last_modified_revision: 0,
    }
}

fn child_by_field_text(node: &TsNode, field: &str, source: &str) -> Option<String> {
    let child = node.child_by_field_name(field)?;
    Some(node_text(&child, source).to_string())
}

fn node_text<'a>(node: &TsNode, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

fn call_function_name(node: &TsNode, source: &str) -> Option<String> {
    let fn_node = node.child_by_field_name("function")?;
    match fn_node.kind() {
        "name" => Some(node_text(&fn_node, source).to_string()),
        "qualified_name" => {
            // Take the last segment
            let text = node_text(&fn_node, source);
            text.rsplit('\\').next().map(str::to_string)
        }
        _ => Some(node_text(&fn_node, source).to_string()),
    }
}

fn enclosing_function(
    node: TsNode,
    source: &str,
    nodes: &[NodeData],
    _file_path: &Path,
) -> Option<NodeId> {
    let mut current = node.parent();
    while let Some(parent) = current {
        match parent.kind() {
            "function_definition" | "method_declaration" => {
                if let Some(name) = parent.child_by_field_name("name") {
                    let name_str = node_text(&name, source);
                    // Try to find the node by checking if any extracted node
                    // ends with this name
                    for nd in nodes {
                        if nd.kind == NodeKind::Function
                            && (nd.name == name_str || nd.qualified_name.ends_with(name_str))
                        {
                            return Some(nd.id.clone());
                        }
                    }
                }
            }
            _ => {}
        }
        current = parent.parent();
    }
    None
}

fn find_function_node(nodes: &[NodeData], name: &str) -> Option<NodeId> {
    nodes
        .iter()
        .find(|n| n.kind == NodeKind::Function && (n.name == name || n.qualified_name.ends_with(name)))
        .map(|n| n.id.clone())
}

fn find_method_node(nodes: &[NodeData], method_name: &str) -> Option<NodeId> {
    nodes
        .iter()
        .find(|n| {
            n.kind == NodeKind::Function
                && n.qualified_name
                    .rsplit('.')
                    .next()
                    .map(|s| s == method_name)
                    .unwrap_or(false)
        })
        .map(|n| n.id.clone())
}

fn find_struct_or_trait(nodes: &[NodeData], name: &str) -> Option<NodeId> {
    nodes
        .iter()
        .find(|n| {
            (n.kind == NodeKind::Struct || n.kind == NodeKind::Trait)
                && (n.name == name || n.qualified_name == name)
        })
        .map(|n| n.id.clone())
}

fn build_span(node: TsNode, file_path: &Path) -> Span {
    Span {
        file: file_path.to_path_buf(),
        start_line: node.start_position().row as u32 + 1,
        start_col: node.start_position().column as u32,
        end_line: node.end_position().row as u32 + 1,
        end_col: node.end_position().column as u32,
        byte_range: node.byte_range(),
    }
}

fn extract_names_from_list(node: TsNode, source: &str, mut callback: impl FnMut(String)) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        match child.kind() {
            "name" | "qualified_name" => {
                let text = node_text(&child, source);
                let name = text.rsplit('\\').next().unwrap_or(text);
                callback(name.to_string());
            }
            _ => {}
        }
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn parse(src: &str) -> ParsedFile {
        let adapter = PhpAdapter::new();
        adapter
            .parse_file(Path::new("test.php"), src)
            .expect("parse failed")
    }

    #[test]
    fn extract_class_and_methods() {
        let src = r#"<?php
class UserService {
    public function findById(int $id): User {
        return $this->repository->find($id);
    }

    public function save(User $user): void {
        $this->repository->save($user);
    }
}
"#;
        let file = parse(src);
        let adapter = PhpAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let classes: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Struct).collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "UserService");

        let fns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert_eq!(fns.len(), 2);
        assert!(fns.iter().any(|f| f.qualified_name == "UserService.findById"));
        assert!(fns.iter().any(|f| f.qualified_name == "UserService.save"));
    }

    #[test]
    fn extract_interface() {
        let src = r#"<?php
interface Renderable {
    public function render(): string;
}
"#;
        let file = parse(src);
        let adapter = PhpAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let traits: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Trait).collect();
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].name, "Renderable");
    }

    #[test]
    fn extract_namespace() {
        let src = r#"<?php
namespace App\Models;

class User {}
"#;
        let file = parse(src);
        let adapter = PhpAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let modules: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Module).collect();
        assert_eq!(modules.len(), 1);
        assert!(modules[0].name.contains("Models") || modules[0].name.contains("App"));
    }

    #[test]
    fn extract_enum() {
        let src = r#"<?php
enum Status {
    case Active;
    case Inactive;
}
"#;
        let file = parse(src);
        let adapter = PhpAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let enums: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Enum).collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Status");
    }

    #[test]
    fn extract_standalone_function() {
        let src = r#"<?php
function helper(int $x): int {
    return $x * 2;
}
"#;
        let file = parse(src);
        let adapter = PhpAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let fns: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Function).collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].name, "helper");
        assert_eq!(fns[0].qualified_name, "helper");
    }

    #[test]
    fn extract_call_edges() {
        let src = r#"<?php
function greet(string $name): void {
    echo format($name);
}

function format(string $s): string {
    return strtolower($s);
}
"#;
        let file = parse(src);
        let adapter = PhpAdapter::new();
        let nodes = adapter.extract_nodes(&file);
        let edges = adapter.extract_edges(&file, &nodes);

        // greet → format
        let calls: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Calls))
            .collect();
        assert!(!calls.is_empty(), "expected at least one call edge");
    }

    #[test]
    fn extract_implements_edge() {
        let src = r#"<?php
interface Serializable {
    public function serialize(): string;
}

class User implements Serializable {
    public function serialize(): string {
        return json_encode($this);
    }
}
"#;
        let file = parse(src);
        let adapter = PhpAdapter::new();
        let nodes = adapter.extract_nodes(&file);
        let edges = adapter.extract_edges(&file, &nodes);

        let impl_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Implements))
            .collect();
        assert!(!impl_edges.is_empty(), "expected implements edge");
    }

    #[test]
    fn complexity_computed_for_methods() {
        let src = r#"<?php
class Calculator {
    public function compute(int $x): int {
        if ($x > 0) {
            for ($i = 0; $i < $x; $i++) {
                if ($i % 2 == 0) {
                    return $i;
                }
            }
        }
        return 0;
    }
}
"#;
        let file = parse(src);
        let adapter = PhpAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let compute = nodes
            .iter()
            .find(|n| n.qualified_name == "Calculator.compute")
            .expect("compute method not found");
        assert!(
            compute.complexity.is_some(),
            "complexity should be computed for methods"
        );
        let metrics = compute.complexity.as_ref().unwrap();
        assert!(metrics.cyclomatic >= 3, "expect cyclomatic >= 3, got {}", metrics.cyclomatic);
    }
}

