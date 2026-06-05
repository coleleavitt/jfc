//! PHP adapter.
//!
//! Produces `NodeData` / `EdgeData` from `.php` files using
//! `tree-sitter-php`. Extracts:
//!
//! - Classes → `NodeKind::Struct`.
//! - Interfaces → `NodeKind::Trait`.
//! - Functions/methods (`function_definition`, `method_declaration`).
//! - Enums → `NodeKind::Enum`.
//! - Namespaces → `NodeKind::Module`.
//! - Call edges (`function_call_expression`, `method_call_expression`).
//! - Implements edges (class implements interface).
//! - UsesType edges (extends, type references).

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind, Visibility};

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
        walk_php(
            root,
            &file.source,
            &file.path,
            &file.path.to_string_lossy(),
            &[],
            None,
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
        let root = file.tree.root_node();

        // Extract call edges
        extract_php_calls(root, &file.source, &file.path, nodes, &mut edges);

        // Extract implements/extends edges from class declarations
        extract_php_inheritance(root, &file.source, &file.path, nodes, &mut edges);

        // Extract type usage edges from function/method parameters and return types
        extract_php_type_usages(root, &file.source, &file.path, nodes, &mut edges);

        edges
    }
}

// ─── Node Extraction ────────────────────────────────────────────────────────

fn walk_php(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    scope: &[&str],
    current_class: Option<&str>,
    out: &mut Vec<NodeData>,
) {
    match node.kind() {
        "namespace_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Module, node, path, path_str, &qn));

                // Recurse into namespace body
                if let Some(body) = node.child_by_field_name("body") {
                    let binding = name;
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&binding);
                    walk_php(body, source, path, path_str, &child_scope, None, out);
                    return;
                }
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Struct, node, path, path_str, &qn));

                // Extract methods inside the class body
                if let Some(body) = node.child_by_field_name("body") {
                    let binding = name;
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&binding);
                    walk_php(
                        body,
                        source,
                        path,
                        path_str,
                        &child_scope,
                        Some(&binding),
                        out,
                    );
                }
                return;
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Trait, node, path, path_str, &qn));

                // Extract method signatures inside interface body
                if let Some(body) = node.child_by_field_name("body") {
                    let binding = name;
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&binding);
                    walk_php(
                        body,
                        source,
                        path,
                        path_str,
                        &child_scope,
                        Some(&binding),
                        out,
                    );
                }
                return;
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Enum, node, path, path_str, &qn));
                return;
            }
        }
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &qn);
                nd.complexity = compute_complexity(node, source.as_bytes(), "php");
                out.push(nd);
                return;
            }
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                // Qualify method as ClassName.method_name
                let qn = if let Some(class_name) = current_class {
                    let method_qualified = format!("{class_name}.{name}");
                    // scope already includes the class name, so we build qualified
                    // relative to the parent scope (namespace) plus the dotted form
                    let parent_scope: Vec<&str> = scope
                        .iter()
                        .take(scope.len().saturating_sub(1))
                        .copied()
                        .collect();
                    qualified(&parent_scope, &method_qualified)
                } else {
                    qualified(scope, &name)
                };
                let vis = detect_php_visibility(node, source);
                let span = build_span(node, path);
                let id = NodeId::new(path_str, &qn, NodeKind::Function);
                let mut nd = NodeData {
                    id,
                    kind: NodeKind::Function,
                    name,
                    qualified_name: qn,
                    file_path: path.to_path_buf(),
                    span,
                    visibility: vis,
                    metadata: HashMap::new(),
                    birth_revision: 0,
                    last_modified_revision: 0,
                    complexity: None,
                    cfg: None,
                    dataflow: None,
                };
                nd.complexity = compute_complexity(node, source.as_bytes(), "php");
                out.push(nd);
                return;
            }
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_php(child, source, path, path_str, scope, current_class, out);
    }
}

// ─── Edge Extraction: Calls ─────────────────────────────────────────────────

fn extract_php_calls(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    match node.kind() {
        "function_call_expression" => {
            if let Some(func_node) = node.child_by_field_name("function") {
                let callee_name = text(func_node, source);
                if let Some(caller) = find_enclosing_function_php(node, source, path, nodes) {
                    let span = build_span(node, path);
                    if let Some(target) = nodes
                        .iter()
                        .find(|n| n.kind == NodeKind::Function && n.name == callee_name)
                    {
                        edges.push((
                            caller.id.clone(),
                            target.id.clone(),
                            EdgeData {
                                kind: EdgeKind::Calls,
                                source_span: span,
                                weight: 1.0,
                            },
                        ));
                    }
                }
            }
        }
        "member_call_expression" | "method_call_expression" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let method_name = text(name_node, source);
                if let Some(caller) = find_enclosing_function_php(node, source, path, nodes) {
                    let span = build_span(node, path);
                    // Try to find a method node with this name
                    if let Some(target) = nodes
                        .iter()
                        .find(|n| n.kind == NodeKind::Function && n.name == method_name)
                    {
                        edges.push((
                            caller.id.clone(),
                            target.id.clone(),
                            EdgeData {
                                kind: EdgeKind::Calls,
                                source_span: span,
                                weight: 1.0,
                            },
                        ));
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_php_calls(child, source, path, nodes, edges);
    }
}

// ─── Edge Extraction: Inheritance ───────────────────────────────────────────

fn extract_php_inheritance(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "class_declaration" {
        if let Some(name_node) = node.child_by_field_name("name") {
            let class_name = text(name_node, source);
            let class_data = nodes
                .iter()
                .find(|n| n.kind == NodeKind::Struct && n.name == class_name);

            if let Some(class_nd) = class_data {
                // Check for base_clause (extends)
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "base_clause" {
                        // The extended class name
                        let mut bc = child.walk();
                        for name_child in child.named_children(&mut bc) {
                            if name_child.kind() == "name" || name_child.kind() == "qualified_name"
                            {
                                let extends_name = text(name_child, source);
                                // Extract just the class name (last segment)
                                let base_name =
                                    extends_name.rsplit('\\').next().unwrap_or(&extends_name);
                                if let Some(target) = nodes.iter().find(|n| {
                                    (n.kind == NodeKind::Struct || n.kind == NodeKind::Trait)
                                        && n.name == base_name
                                }) {
                                    edges.push((
                                        class_nd.id.clone(),
                                        target.id.clone(),
                                        EdgeData {
                                            kind: EdgeKind::UsesType,
                                            source_span: build_span(child, path),
                                            weight: 1.0,
                                        },
                                    ));
                                }
                            }
                        }
                    } else if child.kind() == "class_interface_clause" {
                        // implements
                        let mut ic = child.walk();
                        for iface_child in child.named_children(&mut ic) {
                            if iface_child.kind() == "name"
                                || iface_child.kind() == "qualified_name"
                            {
                                let iface_name = text(iface_child, source);
                                let base_name =
                                    iface_name.rsplit('\\').next().unwrap_or(&iface_name);
                                if let Some(target) = nodes
                                    .iter()
                                    .find(|n| n.kind == NodeKind::Trait && n.name == base_name)
                                {
                                    edges.push((
                                        class_nd.id.clone(),
                                        target.id.clone(),
                                        EdgeData {
                                            kind: EdgeKind::Implements,
                                            source_span: build_span(iface_child, path),
                                            weight: 1.0,
                                        },
                                    ));
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_php_inheritance(child, source, path, nodes, edges);
    }
}

// ─── Edge Extraction: Type Usages ───────────────────────────────────────────

fn extract_php_type_usages(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    let is_func = node.kind() == "function_definition" || node.kind() == "method_declaration";
    if is_func {
        if let Some(name_node) = node.child_by_field_name("name") {
            let fn_name = text(name_node, source);
            if let Some(func_nd) = nodes
                .iter()
                .find(|n| n.kind == NodeKind::Function && n.name == fn_name)
            {
                // Check parameters for type hints
                if let Some(params) = node.child_by_field_name("parameters") {
                    collect_php_type_refs(params, source, path, func_nd, nodes, edges);
                }
                // Check return type
                if let Some(ret) = node.child_by_field_name("return_type") {
                    collect_php_type_refs(ret, source, path, func_nd, nodes, edges);
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_php_type_usages(child, source, path, nodes, edges);
    }
}

fn collect_php_type_refs(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    func_nd: &NodeData,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    // Look for named_type or name nodes that reference types
    if node.kind() == "named_type" || node.kind() == "name" {
        let type_name = text(node, source);
        // Skip primitive types
        if !is_php_primitive(&type_name) {
            let base_name = type_name.rsplit('\\').next().unwrap_or(&type_name);
            if let Some(target) = nodes.iter().find(|n| {
                matches!(n.kind, NodeKind::Struct | NodeKind::Enum | NodeKind::Trait)
                    && n.name == base_name
            }) {
                // Avoid duplicate edges
                let already = edges.iter().any(|(src, dst, e)| {
                    *src == func_nd.id && *dst == target.id && matches!(e.kind, EdgeKind::UsesType)
                });
                if !already {
                    edges.push((
                        func_nd.id.clone(),
                        target.id.clone(),
                        EdgeData {
                            kind: EdgeKind::UsesType,
                            source_span: build_span(node, path),
                            weight: 1.0,
                        },
                    ));
                }
            }
        }
        return;
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_php_type_refs(child, source, path, func_nd, nodes, edges);
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn find_enclosing_function_php<'a>(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &'a [NodeData],
) -> Option<&'a NodeData> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_definition" || parent.kind() == "method_declaration" {
            if let Some(name_node) = parent.child_by_field_name("name") {
                let name = text(name_node, source);
                let parent_span = build_span(parent, path);
                return nodes.iter().find(|n| {
                    n.kind == NodeKind::Function
                        && n.name == name
                        && n.span.start_line == parent_span.start_line
                });
            }
        }
        current = parent.parent();
    }
    None
}

fn detect_php_visibility(node: TsNode<'_>, source: &str) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let vis_text = text(child, source);
            return match vis_text.as_str() {
                "public" => Visibility::Public,
                "protected" => Visibility::Super,
                "private" => Visibility::Private,
                _ => Visibility::Public,
            };
        }
    }
    Visibility::Public
}

fn is_php_primitive(name: &str) -> bool {
    matches!(
        name,
        "int"
            | "float"
            | "string"
            | "bool"
            | "array"
            | "void"
            | "null"
            | "mixed"
            | "object"
            | "callable"
            | "iterable"
            | "never"
            | "self"
            | "static"
            | "parent"
            | "true"
            | "false"
    )
}

use super::{build_nd, build_span, node_text as text};

fn qualified(scope: &[&str], name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", scope.join("::"), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn php_adapter_parses_class_with_methods() {
        let a = PhpAdapter::new();
        let src = r#"<?php
namespace App\Models;

class User {
    public function getName(): string {
        return $this->name;
    }

    private function validate(): bool {
        return true;
    }
}
"#;
        let parsed = a.parse_file(Path::new("User.php"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        // Should have namespace, class, and 2 methods
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "App\\Models" && n.kind == NodeKind::Module),
            "missing namespace, got: {:?}",
            nodes.iter().map(|n| (&n.name, n.kind)).collect::<Vec<_>>()
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "User" && n.kind == NodeKind::Struct),
            "missing User class"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "getName" && n.kind == NodeKind::Function),
            "missing getName method"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "validate" && n.kind == NodeKind::Function),
            "missing validate method"
        );
    }

    #[test]
    fn php_adapter_extracts_functions() {
        let a = PhpAdapter::new();
        let src = r#"<?php
function hello(): void {
    echo "hello";
}

function world(): string {
    return "world";
}
"#;
        let parsed = a.parse_file(Path::new("funcs.php"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let functions: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(functions.len(), 2);
        assert!(functions.iter().any(|n| n.name == "hello"));
        assert!(functions.iter().any(|n| n.name == "world"));
    }

    #[test]
    fn php_adapter_extracts_call_edges() {
        let a = PhpAdapter::new();
        let src = r#"<?php
function caller(): void {
    callee();
}

function callee(): void {
    // ...
}
"#;
        let parsed = a.parse_file(Path::new("calls.php"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        let call_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Calls))
            .collect();
        assert!(
            !call_edges.is_empty(),
            "expected at least one call edge, got none"
        );

        let caller = nodes.iter().find(|n| n.name == "caller").unwrap();
        let callee = nodes.iter().find(|n| n.name == "callee").unwrap();
        assert!(
            call_edges
                .iter()
                .any(|(src, dst, _)| *src == caller.id && *dst == callee.id),
            "expected caller -> callee edge"
        );
    }

    #[test]
    fn php_adapter_extracts_namespace() {
        let a = PhpAdapter::new();
        let src = r#"<?php
namespace App\Services;

function doStuff(): void {}
"#;
        let parsed = a.parse_file(Path::new("svc.php"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        assert!(
            nodes
                .iter()
                .any(|n| n.name == "App\\Services" && n.kind == NodeKind::Module),
            "missing namespace node, got: {:?}",
            nodes.iter().map(|n| (&n.name, n.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn php_adapter_extracts_interface_and_implements() {
        let a = PhpAdapter::new();
        let src = r#"<?php
interface Renderable {
    public function render(): string;
}

class Widget implements Renderable {
    public function render(): string {
        return "<div></div>";
    }
}
"#;
        let parsed = a.parse_file(Path::new("iface.php"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        // Check interface exists as Trait
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Renderable" && n.kind == NodeKind::Trait),
            "missing Renderable interface"
        );

        // Check Widget exists as Struct
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Widget" && n.kind == NodeKind::Struct),
            "missing Widget class"
        );

        // Check implements edge
        let impl_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Implements))
            .collect();
        assert!(
            !impl_edges.is_empty(),
            "expected implements edge, got none. All edges: {:?}",
            edges.iter().map(|(_, _, e)| &e.kind).collect::<Vec<_>>()
        );

        let widget = nodes
            .iter()
            .find(|n| n.name == "Widget" && n.kind == NodeKind::Struct)
            .unwrap();
        let renderable = nodes
            .iter()
            .find(|n| n.name == "Renderable" && n.kind == NodeKind::Trait)
            .unwrap();
        assert!(
            impl_edges
                .iter()
                .any(|(src, dst, _)| *src == widget.id && *dst == renderable.id),
            "expected Widget implements Renderable edge"
        );
    }

    #[test]
    fn php_adapter_extracts_enum() {
        let a = PhpAdapter::new();
        let src = r#"<?php
enum Status {
    case Active;
    case Inactive;
}
"#;
        let parsed = a.parse_file(Path::new("enum.php"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Status" && n.kind == NodeKind::Enum),
            "missing Status enum, got: {:?}",
            nodes.iter().map(|n| (&n.name, n.kind)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn php_adapter_method_qualified_name() {
        let a = PhpAdapter::new();
        let src = r#"<?php
class Foo {
    public function bar(): void {}
}
"#;
        let parsed = a.parse_file(Path::new("foo.php"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let method = nodes
            .iter()
            .find(|n| n.name == "bar" && n.kind == NodeKind::Function)
            .expect("missing bar method");
        assert!(
            method.qualified_name.contains("Foo.bar"),
            "expected qualified name to contain 'Foo.bar', got: {}",
            method.qualified_name
        );
    }

    #[test]
    fn php_adapter_visibility() {
        let a = PhpAdapter::new();
        let src = r#"<?php
class MyClass {
    public function pub_method(): void {}
    private function priv_method(): void {}
    protected function prot_method(): void {}
}
"#;
        let parsed = a.parse_file(Path::new("vis.php"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let pub_m = nodes.iter().find(|n| n.name == "pub_method").unwrap();
        assert_eq!(pub_m.visibility, Visibility::Public);

        let priv_m = nodes.iter().find(|n| n.name == "priv_method").unwrap();
        assert_eq!(priv_m.visibility, Visibility::Private);

        let prot_m = nodes.iter().find(|n| n.name == "prot_method").unwrap();
        assert_eq!(prot_m.visibility, Visibility::Super);
    }
}
