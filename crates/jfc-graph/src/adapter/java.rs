//! Java adapter.
//!
//! Produces `NodeData` / `EdgeData` from `.java` files using
//! `tree-sitter-java`. Extracts:
//!
//! - Classes (`class_declaration`) → `NodeKind::Struct`.
//! - Interfaces (`interface_declaration`) → `NodeKind::Trait`.
//! - Methods (`method_declaration`) → `NodeKind::Function` (qualified as `ClassName.method_name`).
//! - Constructors (`constructor_declaration`) → `NodeKind::Function` (qualified as `ClassName.ClassName`).
//! - Enums (`enum_declaration`) → `NodeKind::Enum`.
//! - Packages (`package_declaration`) → `NodeKind::Module`.
//!
//! Edges:
//! - `method_invocation` → `EdgeKind::Calls`
//! - `object_creation_expression` → `EdgeKind::Calls` (constructor call)
//! - `class_declaration` with `implements` → `EdgeKind::Implements`
//! - `class_declaration` with `extends` / type refs → `EdgeKind::UsesType`

use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind};

pub struct JavaAdapter {
    language: Language,
}

impl JavaAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_java::LANGUAGE.into(),
        }
    }
}

impl LanguageAdapter for JavaAdapter {
    fn language_id(&self) -> &str {
        "java"
    }

    fn file_extensions(&self) -> &[&str] {
        &["java"]
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
        walk_java(root, &file.source, &file.path, &path_str, None, &mut nodes);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        extract_java_edges(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &mut edges,
        );
        edges
    }
}

/// Recursively walk the tree, extracting nodes.
/// `enclosing_class` tracks the current class/interface/enum name for qualified naming.
fn walk_java(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    enclosing_class: Option<&str>,
    out: &mut Vec<NodeData>,
) {
    match node.kind() {
        "package_declaration" => {
            // The package name is the first named child (scoped_identifier or identifier).
            let mut cursor = node.walk();
            if let Some(name_node) = node.named_children(&mut cursor).next() {
                let name = text(name_node, source);
                out.push(build_nd(
                    &name,
                    NodeKind::Module,
                    node,
                    path,
                    path_str,
                    &name,
                ));
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                out.push(build_nd(
                    &name,
                    NodeKind::Struct,
                    node,
                    path,
                    path_str,
                    &name,
                ));
                // Recurse into class body with this class as enclosing.
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    walk_java(child, source, path, path_str, Some(&name), out);
                }
                return; // Don't recurse again below.
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                out.push(build_nd(
                    &name,
                    NodeKind::Trait,
                    node,
                    path,
                    path_str,
                    &name,
                ));
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    walk_java(child, source, path, path_str, Some(&name), out);
                }
                return;
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                out.push(build_nd(&name, NodeKind::Enum, node, path, path_str, &name));
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    walk_java(child, source, path, path_str, Some(&name), out);
                }
                return;
            }
        }
        "method_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = match enclosing_class {
                    Some(cls) => format!("{cls}.{name}"),
                    None => name.clone(),
                };
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &qn);
                nd.complexity = compute_complexity(node, source.as_bytes(), "java");
                out.push(nd);
            }
        }
        "constructor_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = match enclosing_class {
                    Some(cls) => format!("{cls}.{name}"),
                    None => name.clone(),
                };
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &qn);
                nd.complexity = compute_complexity(node, source.as_bytes(), "java");
                out.push(nd);
            }
        }
        _ => {}
    }

    // Default: recurse into children preserving the enclosing class.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_java(child, source, path, path_str, enclosing_class, out);
    }
}

/// Extract edges: calls, constructor calls, implements, extends.
fn extract_java_edges(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    match node.kind() {
        "method_invocation" => {
            // The method name is in field "name".
            if let Some(name_node) = node.child_by_field_name("name") {
                let callee_name = text(name_node, source);
                if let Some(caller_id) = find_enclosing_function(node, source, nodes) {
                    // Try to find the callee among known functions.
                    if let Some(callee) = nodes
                        .iter()
                        .find(|n| n.kind == NodeKind::Function && n.name == callee_name)
                    {
                        edges.push((
                            caller_id,
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
        "object_creation_expression" => {
            // `new Foo(...)` — the type is in field "type".
            if let Some(type_node) = node.child_by_field_name("type") {
                let type_name = text(type_node, source);
                if let Some(caller_id) = find_enclosing_function(node, source, nodes) {
                    // Constructor call: look for a Function node named same as type.
                    if let Some(ctor) = nodes
                        .iter()
                        .find(|n| n.kind == NodeKind::Function && n.name == type_name)
                    {
                        edges.push((
                            caller_id,
                            ctor.id.clone(),
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
        "class_declaration" => {
            // Check for `implements` and `extends`.
            if let Some(class_name_node) = node.child_by_field_name("name") {
                let class_name = text(class_name_node, source);
                let class_node_data = nodes
                    .iter()
                    .find(|n| n.kind == NodeKind::Struct && n.name == class_name);

                if let Some(class_nd) = class_node_data {
                    // interfaces (implements clause)
                    if let Some(interfaces) = node.child_by_field_name("interfaces") {
                        extract_type_list_edges(
                            interfaces,
                            source,
                            path,
                            nodes,
                            &class_nd.id,
                            EdgeKind::Implements,
                            edges,
                        );
                    }
                    // superclass (extends clause)
                    if let Some(superclass) = node.child_by_field_name("superclass") {
                        extract_type_list_edges(
                            superclass,
                            source,
                            path,
                            nodes,
                            &class_nd.id,
                            EdgeKind::UsesType,
                            edges,
                        );
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_java_edges(child, source, path, nodes, edges);
    }
}

/// Extract type references from an implements/extends list node.
fn extract_type_list_edges(
    list_node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    source_id: &NodeId,
    edge_kind: EdgeKind,
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    let mut cursor = list_node.walk();
    for child in list_node.named_children(&mut cursor) {
        // type_list children are type_identifiers or generic_types
        let type_name = match child.kind() {
            "type_identifier" => text(child, source),
            "generic_type" => {
                // First child is the type_identifier
                child
                    .named_child(0)
                    .map(|c| text(c, source))
                    .unwrap_or_default()
            }
            _ => text(child, source),
        };
        if type_name.is_empty() {
            continue;
        }
        // Find the target node (Trait for implements, Struct for extends).
        if let Some(target) = nodes.iter().find(|n| {
            n.name == type_name
                && matches!(n.kind, NodeKind::Trait | NodeKind::Struct | NodeKind::Enum)
        }) {
            edges.push((
                source_id.clone(),
                target.id.clone(),
                EdgeData {
                    kind: edge_kind.clone(),
                    source_span: build_span(child, path),
                    weight: 1.0,
                },
            ));
        }
    }
}

/// Walk up from a node to find the enclosing method/constructor and return its NodeId.
fn find_enclosing_function(node: TsNode<'_>, source: &str, nodes: &[NodeData]) -> Option<NodeId> {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if matches!(p.kind(), "method_declaration" | "constructor_declaration") {
            if let Some(n) = p.child_by_field_name("name") {
                let name = text(n, source);
                return nodes
                    .iter()
                    .find(|nd| nd.name == name && nd.kind == NodeKind::Function)
                    .map(|nd| nd.id.clone());
            }
        }
        parent = p.parent();
    }
    None
}

use super::{build_nd, build_span, node_text as text};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_adapter_extracts_class_and_methods() {
        let a = JavaAdapter::new();
        let src = r#"
package com.example;

public class UserService {
    public User findById(int id) {
        return repository.find(id);
    }

    public void save(User user) {
        repository.save(user);
        logger.info("saved");
    }
}
"#;
        let parsed = a.parse_file(Path::new("UserService.java"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        // Package
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "com.example" && n.kind == NodeKind::Module),
            "expected package node, got: {nodes:?}"
        );

        // Class
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "UserService" && n.kind == NodeKind::Struct),
            "expected class node"
        );

        // Methods with qualified names
        assert!(
            nodes.iter().any(|n| n.qualified_name == "UserService.findById" && n.kind == NodeKind::Function),
            "expected findById method, got: {:?}",
            nodes.iter().filter(|n| n.kind == NodeKind::Function).collect::<Vec<_>>()
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.qualified_name == "UserService.save" && n.kind == NodeKind::Function),
            "expected save method"
        );
    }

    #[test]
    fn java_adapter_extracts_interface() {
        let a = JavaAdapter::new();
        let src = r#"
public interface Repository {
    User find(int id);
    void save(User user);
}
"#;
        let parsed = a.parse_file(Path::new("Repository.java"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Repository" && n.kind == NodeKind::Trait),
            "expected interface node"
        );
    }

    #[test]
    fn java_adapter_extracts_enum() {
        let a = JavaAdapter::new();
        let src = r#"
public enum Status {
    ACTIVE,
    INACTIVE;
}
"#;
        let parsed = a.parse_file(Path::new("Status.java"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Status" && n.kind == NodeKind::Enum),
            "expected enum node"
        );
    }

    #[test]
    fn java_adapter_extracts_constructor() {
        let a = JavaAdapter::new();
        let src = r#"
public class Foo {
    public Foo(int x) {
        this.x = x;
    }
}
"#;
        let parsed = a.parse_file(Path::new("Foo.java"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        assert!(
            nodes
                .iter()
                .any(|n| n.qualified_name == "Foo.Foo" && n.kind == NodeKind::Function),
            "expected constructor node, got: {:?}",
            nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Function)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn java_adapter_extracts_call_edges() {
        let a = JavaAdapter::new();
        let src = r#"
public class Svc {
    public void caller() {
        callee();
    }

    public void callee() {
    }
}
"#;
        let parsed = a.parse_file(Path::new("Svc.java"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        assert!(
            !edges.is_empty(),
            "expected at least one call edge, got none"
        );
        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::Calls),
            "expected Calls edge"
        );
    }

    #[test]
    fn java_adapter_extracts_implements_edge() {
        let a = JavaAdapter::new();
        let src = r#"
public interface Runnable {
    void run();
}

public class Worker implements Runnable {
    public void run() {}
}
"#;
        let parsed = a.parse_file(Path::new("Worker.java"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::Implements),
            "expected Implements edge, got: {edges:?}"
        );
    }

    #[test]
    fn java_adapter_extracts_extends_edge() {
        let a = JavaAdapter::new();
        let src = r#"
public class Base {
    public void doStuff() {}
}

public class Derived extends Base {
    public void extra() {}
}
"#;
        let parsed = a.parse_file(Path::new("Derived.java"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::UsesType),
            "expected UsesType edge for extends, got: {edges:?}"
        );
    }

    #[test]
    fn java_adapter_complexity_metrics() {
        let a = JavaAdapter::new();
        let src = r#"
public class Logic {
    public int compute(int x) {
        if (x > 0) {
            for (int i = 0; i < x; i++) {
                if (i % 2 == 0) {
                    x += i;
                }
            }
        }
        return x;
    }
}
"#;
        let parsed = a.parse_file(Path::new("Logic.java"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let compute = nodes
            .iter()
            .find(|n| n.qualified_name == "Logic.compute")
            .expect("compute method not found");
        let cx = compute
            .complexity
            .as_ref()
            .expect("complexity metrics missing");

        assert!(
            cx.cyclomatic >= 4,
            "expected cyclomatic >= 4, got {}",
            cx.cyclomatic
        );
        assert!(
            cx.cognitive >= 3,
            "expected cognitive >= 3, got {}",
            cx.cognitive
        );
        assert!(
            cx.max_nesting >= 3,
            "expected max_nesting >= 3, got {}",
            cx.max_nesting
        );
    }
}
