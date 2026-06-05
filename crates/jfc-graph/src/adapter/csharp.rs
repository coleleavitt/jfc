//! C# adapter.
//!
//! Produces `NodeData` / `EdgeData` from `.cs` files using
//! `tree-sitter-c-sharp`. Extracts:
//!
//! - Classes (`class_declaration`) → `NodeKind::Struct`.
//! - Interfaces (`interface_declaration`) → `NodeKind::Trait`.
//! - Methods (`method_declaration`) → `NodeKind::Function` (qualified as `ClassName.method_name`).
//! - Constructors (`constructor_declaration`) → `NodeKind::Function` (qualified as `ClassName.ClassName`).
//! - Enums (`enum_declaration`) → `NodeKind::Enum`.
//! - Namespaces (`namespace_declaration` / `file_scoped_namespace_declaration`) → `NodeKind::Module`.
//! - Structs (`struct_declaration`) → `NodeKind::Struct`.
//! - Records (`record_declaration`) → `NodeKind::Struct`.
//!
//! Edges:
//! - `invocation_expression` → `EdgeKind::Calls`
//! - `object_creation_expression` → `EdgeKind::Calls` (constructor call)
//! - `class_declaration` with `base_list` containing interfaces → `EdgeKind::Implements`
//! - `class_declaration` with `base_list` containing class → `EdgeKind::UsesType`

use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind, Visibility};

pub struct CSharpAdapter {
    language: Language,
}

impl CSharpAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_c_sharp::LANGUAGE.into(),
        }
    }
}

impl LanguageAdapter for CSharpAdapter {
    fn language_id(&self) -> &str {
        "csharp"
    }

    fn file_extensions(&self) -> &[&str] {
        &["cs"]
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
        walk_csharp(root, &file.source, &file.path, &path_str, None, &mut nodes);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        extract_csharp_edges(
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
/// `enclosing_class` tracks the current class/interface/enum/struct name for qualified naming.
fn walk_csharp(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    enclosing_class: Option<&str>,
    out: &mut Vec<NodeData>,
) {
    match node.kind() {
        "namespace_declaration" | "file_scoped_namespace_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
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
            // Recurse into namespace body
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                walk_csharp(child, source, path, path_str, enclosing_class, out);
            }
            return;
        }
        "class_declaration" | "struct_declaration" | "record_declaration" => {
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
                    walk_csharp(child, source, path, path_str, Some(&name), out);
                }
                return;
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
                    walk_csharp(child, source, path, path_str, Some(&name), out);
                }
                return;
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                out.push(build_nd(&name, NodeKind::Enum, node, path, path_str, &name));
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
                nd.complexity = compute_complexity(node, source.as_bytes(), "csharp");
                nd.visibility = extract_visibility(node, source);
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
                nd.complexity = compute_complexity(node, source.as_bytes(), "csharp");
                nd.visibility = extract_visibility(node, source);
                out.push(nd);
            }
        }
        _ => {}
    }

    // Default: recurse into children preserving the enclosing class.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_csharp(child, source, path, path_str, enclosing_class, out);
    }
}

/// Extract edges: calls, constructor calls, implements, uses_type.
fn extract_csharp_edges(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    match node.kind() {
        "invocation_expression" => {
            // The callee is the first child (member_access_expression or identifier).
            // Extract the method name from the rightmost identifier.
            if let Some(callee_name) = extract_invocation_name(node, source) {
                if let Some(caller_id) = find_enclosing_function(node, source, nodes) {
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
        "class_declaration" | "struct_declaration" | "record_declaration" => {
            if let Some(class_name_node) = node.child_by_field_name("name") {
                let class_name = text(class_name_node, source);
                let class_node_data = nodes
                    .iter()
                    .find(|n| n.kind == NodeKind::Struct && n.name == class_name);

                if let Some(class_nd) = class_node_data {
                    // In C#, `base_list` contains both base class and interfaces.
                    if let Some(base_list) = find_child_by_kind(node, "base_list") {
                        extract_base_list_edges(
                            base_list,
                            source,
                            path,
                            nodes,
                            &class_nd.id,
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
        extract_csharp_edges(child, source, path, nodes, edges);
    }
}

/// Extract the method name from an invocation_expression.
/// Handles: `Foo()`, `obj.Foo()`, `ns.obj.Foo()`.
fn extract_invocation_name(node: TsNode<'_>, source: &str) -> Option<String> {
    // The function part is the first child of invocation_expression.
    let func_part = node.child(0)?;
    match func_part.kind() {
        "member_access_expression" => {
            // The "name" field is the method name.
            func_part
                .child_by_field_name("name")
                .map(|n| text(n, source))
        }
        "identifier" | "generic_name" => Some(text(func_part, source)),
        _ => {
            // Fall back: take the text of whatever it is.
            Some(text(func_part, source))
        }
    }
}

/// Extract edges from a C# base_list (inheritance / interface implementation).
fn extract_base_list_edges(
    base_list: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    source_id: &NodeId,
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    let mut cursor = base_list.walk();
    for child in base_list.named_children(&mut cursor) {
        // Children are typically identifiers, qualified_name, or generic_name.
        let type_name = extract_type_name(child, source);
        if type_name.is_empty() {
            continue;
        }

        // If the target is a Trait (interface), emit Implements; otherwise UsesType.
        if let Some(target) = nodes.iter().find(|n| n.name == type_name) {
            let edge_kind = if target.kind == NodeKind::Trait {
                EdgeKind::Implements
            } else {
                EdgeKind::UsesType
            };
            edges.push((
                source_id.clone(),
                target.id.clone(),
                EdgeData {
                    kind: edge_kind,
                    source_span: build_span(child, path),
                    weight: 1.0,
                },
            ));
        }
    }
}

/// Extract a simple type name from a type node (handles identifier, qualified_name, generic_name).
fn extract_type_name(node: TsNode<'_>, source: &str) -> String {
    match node.kind() {
        "identifier" => text(node, source),
        "qualified_name" => {
            // Take the last identifier (the simple name).
            let mut cursor = node.walk();
            let children: Vec<_> = node.named_children(&mut cursor).collect();
            children
                .last()
                .map(|n| text(*n, source))
                .unwrap_or_default()
        }
        "generic_name" => {
            // First child is the identifier.
            node.named_child(0)
                .map(|n| text(n, source))
                .unwrap_or_default()
        }
        _ => text(node, source),
    }
}

/// Find a child node by kind (non-field based).
fn find_child_by_kind<'a>(node: TsNode<'a>, kind: &str) -> Option<TsNode<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| c.kind() == kind)
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

/// Extract visibility from modifier nodes preceding a declaration.
fn extract_visibility(node: TsNode<'_>, source: &str) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "modifier" {
            let mod_text = text(child, source);
            match mod_text.as_str() {
                "public" => return Visibility::Public,
                "private" => return Visibility::Private,
                "protected" => return Visibility::Private, // map to Private for our model
                "internal" => return Visibility::Crate,    // internal ≈ crate-visible
                _ => {}
            }
        }
    }
    // Default: C# members without explicit modifier are private (in classes).
    Visibility::Private
}

use super::{build_nd, build_span, node_text as text};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csharp_adapter_extracts_namespace_class_methods_enum() {
        let a = CSharpAdapter::new();
        let src = r#"
namespace MyApp.Services;

public interface IUserService {
    User FindById(int id);
}

public class UserService : IUserService {
    public User FindById(int id) {
        return _repository.Find(id);
    }

    public void Save(User user) {
        _repository.Save(user);
        _logger.Info("saved");
    }
}

public enum Status {
    Active,
    Inactive
}
"#;
        let parsed = a.parse_file(Path::new("UserService.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        // Namespace
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "MyApp.Services" && n.kind == NodeKind::Module),
            "expected namespace node, got: {:?}",
            nodes.iter().map(|n| (&n.name, &n.kind)).collect::<Vec<_>>()
        );

        // Interface
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "IUserService" && n.kind == NodeKind::Trait),
            "expected interface node"
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
            nodes.iter().any(|n| n.qualified_name == "UserService.FindById" && n.kind == NodeKind::Function),
            "expected FindById method, got: {:?}",
            nodes.iter().filter(|n| n.kind == NodeKind::Function).map(|n| &n.qualified_name).collect::<Vec<_>>()
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.qualified_name == "UserService.Save" && n.kind == NodeKind::Function),
            "expected Save method"
        );

        // Enum
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Status" && n.kind == NodeKind::Enum),
            "expected enum node"
        );
    }

    #[test]
    fn csharp_adapter_extracts_struct_and_record() {
        let a = CSharpAdapter::new();
        let src = r#"
public struct Point {
    public int X;
    public int Y;
}

public record Person(string Name, int Age);
"#;
        let parsed = a.parse_file(Path::new("Types.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Point" && n.kind == NodeKind::Struct),
            "expected struct node"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Person" && n.kind == NodeKind::Struct),
            "expected record node as Struct"
        );
    }

    #[test]
    fn csharp_adapter_extracts_constructor() {
        let a = CSharpAdapter::new();
        let src = r#"
public class Foo {
    public Foo(int x) {
        this.x = x;
    }
}
"#;
        let parsed = a.parse_file(Path::new("Foo.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        assert!(
            nodes
                .iter()
                .any(|n| n.qualified_name == "Foo.Foo" && n.kind == NodeKind::Function),
            "expected constructor node, got: {:?}",
            nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Function)
                .map(|n| &n.qualified_name)
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn csharp_adapter_extracts_call_edges() {
        let a = CSharpAdapter::new();
        let src = r#"
public class Svc {
    public void Caller() {
        Callee();
    }

    public void Callee() {
    }
}
"#;
        let parsed = a.parse_file(Path::new("Svc.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::Calls),
            "expected Calls edge, got: {:?}",
            edges.iter().map(|(_, _, e)| &e.kind).collect::<Vec<_>>()
        );
    }

    #[test]
    fn csharp_adapter_extracts_member_call_edges() {
        let a = CSharpAdapter::new();
        let src = r#"
public class Svc {
    public void Caller() {
        _repo.Save();
    }

    public void Save() {
    }
}
"#;
        let parsed = a.parse_file(Path::new("Svc.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        // _repo.Save() should resolve to the Save method (same name match).
        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::Calls),
            "expected Calls edge for member access, got: {:?}",
            edges.iter().map(|(_, _, e)| &e.kind).collect::<Vec<_>>()
        );
    }

    #[test]
    fn csharp_adapter_extracts_implements_edge() {
        let a = CSharpAdapter::new();
        let src = r#"
public interface IUserService {
    void Run();
}

public class UserService : IUserService {
    public void Run() {}
}
"#;
        let parsed = a.parse_file(Path::new("UserService.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::Implements),
            "expected Implements edge, got: {:?}",
            edges
                .iter()
                .map(|(from, to, e)| (from, to, &e.kind))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn csharp_adapter_extracts_uses_type_edge() {
        let a = CSharpAdapter::new();
        let src = r#"
public class Base {
    public void DoStuff() {}
}

public class Derived : Base {
    public void Extra() {}
}
"#;
        let parsed = a.parse_file(Path::new("Derived.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::UsesType),
            "expected UsesType edge for class inheritance, got: {:?}",
            edges.iter().map(|(_, _, e)| &e.kind).collect::<Vec<_>>()
        );
    }

    #[test]
    fn csharp_adapter_complexity_metrics() {
        let a = CSharpAdapter::new();
        let src = r#"
public class Logic {
    public int Compute(int x) {
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
        let parsed = a.parse_file(Path::new("Logic.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let compute = nodes
            .iter()
            .find(|n| n.qualified_name == "Logic.Compute")
            .expect("Compute method not found");
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

    #[test]
    fn csharp_adapter_block_scoped_namespace() {
        let a = CSharpAdapter::new();
        let src = r#"
namespace MyApp {
    public class Foo {
        public void Bar() {}
    }
}
"#;
        let parsed = a.parse_file(Path::new("Foo.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        assert!(
            nodes
                .iter()
                .any(|n| n.name == "MyApp" && n.kind == NodeKind::Module),
            "expected block-scoped namespace"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Foo" && n.kind == NodeKind::Struct),
            "expected class inside namespace"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.qualified_name == "Foo.Bar" && n.kind == NodeKind::Function),
            "expected method inside class inside namespace"
        );
    }

    #[test]
    fn csharp_adapter_visibility() {
        let a = CSharpAdapter::new();
        let src = r#"
public class Svc {
    public void Pub() {}
    private void Priv() {}
    internal void Intern() {}
}
"#;
        let parsed = a.parse_file(Path::new("Svc.cs"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let pub_fn = nodes.iter().find(|n| n.name == "Pub").unwrap();
        assert_eq!(pub_fn.visibility, Visibility::Public);

        let priv_fn = nodes.iter().find(|n| n.name == "Priv").unwrap();
        assert_eq!(priv_fn.visibility, Visibility::Private);

        let intern_fn = nodes.iter().find(|n| n.name == "Intern").unwrap();
        assert_eq!(intern_fn.visibility, Visibility::Crate);
    }
}
