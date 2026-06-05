//! Ruby adapter.
//!
//! Produces `NodeData` / `EdgeData` from `.rb` files using
//! `tree-sitter-ruby`. Extracts:
//!
//! - Methods (`method`, `singleton_method`) → `NodeKind::Function`.
//! - Classes (`class`) → `NodeKind::Struct`.
//! - Modules (`module`) → `NodeKind::Module`.
//! - Call edges (`call`) → `EdgeKind::Calls`.
//! - Inheritance (`class` with superclass) → `EdgeKind::UsesType`.

use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind};

pub struct RubyAdapter {
    language: Language,
}

impl RubyAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_ruby::LANGUAGE.into(),
        }
    }
}

impl LanguageAdapter for RubyAdapter {
    fn language_id(&self) -> &'static str {
        "ruby"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rb"]
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
        walk_ruby(
            root,
            &file.source,
            &file.path,
            &file.path.to_string_lossy(),
            &[],
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
        extract_ruby_edges(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &mut edges,
        );
        edges
    }
}

/// Recursively walk the AST and extract nodes (modules, classes, methods).
fn walk_ruby(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    scope: &[&str],
    out: &mut Vec<NodeData>,
) {
    match node.kind() {
        "module" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Module, node, path, path_str, &qn));
                // Descend into module body with updated scope.
                if let Some(body) = node.child_by_field_name("body") {
                    let binding = name;
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&binding);
                    walk_ruby(body, source, path, path_str, &child_scope, out);
                }
                return;
            }
        }
        "class" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Struct, node, path, path_str, &qn));
                // Descend into class body with class name in scope.
                if let Some(body) = node.child_by_field_name("body") {
                    let binding = name;
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&binding);
                    walk_ruby(body, source, path, path_str, &child_scope, out);
                }
                return;
            }
        }
        "method" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &qn);
                nd.complexity = compute_complexity(node, source.as_bytes(), "ruby");
                out.push(nd);
            }
            // Don't recurse into method bodies for more methods — Ruby doesn't nest defs idiomatically.
            return;
        }
        "singleton_method" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &qn);
                nd.complexity = compute_complexity(node, source.as_bytes(), "ruby");
                out.push(nd);
            }
            return;
        }
        _ => {}
    }
    // Default: recurse into children.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_ruby(child, source, path, path_str, scope, out);
    }
}

/// Extract edges: call edges and inheritance (UsesType).
fn extract_ruby_edges(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    match node.kind() {
        "call" => {
            if let Some(method_node) = node.child_by_field_name("method") {
                let callee_name = text(method_node, source);
                // Find the enclosing method (caller).
                let caller_id = find_enclosing_method(node, source, nodes);
                if let Some(caller) = caller_id {
                    // Try to resolve callee among known functions.
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
        "class" => {
            // Inheritance: class Foo < Bar → UsesType edge from Foo to Bar.
            if let Some(name_node) = node.child_by_field_name("name") {
                let class_name = text(name_node, source);
                if let Some(superclass_node) = node.child_by_field_name("superclass") {
                    // The superclass node wraps the constant — get the first named child.
                    let super_name = first_named_child_text(superclass_node, source)
                        .unwrap_or_else(|| text(superclass_node, source));
                    // Find source class node.
                    if let Some(class_nd) = nodes
                        .iter()
                        .find(|n| n.name == class_name && n.kind == NodeKind::Struct)
                    {
                        // Find target superclass node.
                        if let Some(super_nd) = nodes
                            .iter()
                            .find(|n| n.name == super_name && n.kind == NodeKind::Struct)
                        {
                            edges.push((
                                class_nd.id.clone(),
                                super_nd.id.clone(),
                                EdgeData {
                                    kind: EdgeKind::UsesType,
                                    source_span: build_span(superclass_node, path),
                                    weight: 1.0,
                                },
                            ));
                        }
                    }
                }
            }
        }
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_ruby_edges(child, source, path, nodes, edges);
    }
}

/// Walk up the tree to find the enclosing method/singleton_method and return its NodeId.
fn find_enclosing_method(node: TsNode<'_>, source: &str, nodes: &[NodeData]) -> Option<NodeId> {
    let mut parent = node.parent();
    while let Some(p) = parent {
        match p.kind() {
            "method" | "singleton_method" => {
                if let Some(n) = p.child_by_field_name("name") {
                    let name = text(n, source);
                    return nodes
                        .iter()
                        .find(|nd| nd.name == name && nd.kind == NodeKind::Function)
                        .map(|nd| nd.id.clone());
                }
                return None;
            }
            _ => parent = p.parent(),
        }
    }
    None
}

/// Get the text of the first named child of a node.
fn first_named_child_text(node: TsNode<'_>, source: &str) -> Option<String> {
    node.named_child(0).map(|c| text(c, source))
}

use super::{build_nd, build_span, node_text as text};

fn qualified(scope: &[&str], name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{}.{}", scope.join("."), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ruby(src: &str) -> (Vec<NodeData>, Vec<(NodeId, NodeId, EdgeData)>) {
        let adapter = RubyAdapter::new();
        let parsed = adapter.parse_file(Path::new("test.rb"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);
        (nodes, edges)
    }

    #[test]
    fn ruby_adapter_extracts_module() {
        let src = "module Authentication\n  def authenticate(user)\n    user\n  end\nend\n";
        let (nodes, _) = parse_ruby(src);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Authentication" && n.kind == NodeKind::Module),
            "expected Module node for Authentication, got: {nodes:?}"
        );
    }

    #[test]
    fn ruby_adapter_extracts_class_as_struct() {
        let src = "class User\n  def initialize(name)\n    @name = name\n  end\nend\n";
        let (nodes, _) = parse_ruby(src);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "User" && n.kind == NodeKind::Struct),
            "expected Struct node for User, got: {nodes:?}"
        );
    }

    #[test]
    fn ruby_adapter_extracts_methods_with_qualified_names() {
        let src = "class User\n  def initialize(name)\n    @name = name\n  end\n\n  def greet\n    puts \"hi\"\n  end\nend\n";
        let (nodes, _) = parse_ruby(src);
        assert!(
            nodes.iter().any(|n| n.name == "initialize"
                && n.kind == NodeKind::Function
                && n.qualified_name == "User.initialize"),
            "expected Function with qualified name User.initialize, got: {nodes:?}"
        );
        assert!(
            nodes.iter().any(|n| n.name == "greet"
                && n.kind == NodeKind::Function
                && n.qualified_name == "User.greet"),
            "expected Function with qualified name User.greet, got: {nodes:?}"
        );
    }

    #[test]
    fn ruby_adapter_extracts_singleton_method() {
        let src = "class User\n  def self.find(id)\n    id\n  end\nend\n";
        let (nodes, _) = parse_ruby(src);
        assert!(
            nodes.iter().any(|n| n.name == "find"
                && n.kind == NodeKind::Function
                && n.qualified_name == "User.find"),
            "expected singleton method User.find, got: {nodes:?}"
        );
    }

    #[test]
    fn ruby_adapter_extracts_call_edges() {
        let src = r#"
class Foo
  def caller_method
    callee_method()
  end

  def callee_method
    42
  end
end
"#;
        let (nodes, edges) = parse_ruby(src);
        assert!(
            edges.iter().any(|(from, to, e)| {
                let from_nd = nodes.iter().find(|n| n.id == *from);
                let to_nd = nodes.iter().find(|n| n.id == *to);
                from_nd.map(|n| n.name.as_str()) == Some("caller_method")
                    && to_nd.map(|n| n.name.as_str()) == Some("callee_method")
                    && e.kind == EdgeKind::Calls
            }),
            "expected Calls edge from caller_method to callee_method, got: {edges:?}"
        );
    }

    #[test]
    fn ruby_adapter_extracts_inheritance_edge() {
        let src = r#"
class BaseModel
  def save
    true
  end
end

class User < BaseModel
  def initialize(name)
    @name = name
  end
end
"#;
        let (nodes, edges) = parse_ruby(src);
        assert!(
            edges.iter().any(|(from, to, e)| {
                let from_nd = nodes.iter().find(|n| n.id == *from);
                let to_nd = nodes.iter().find(|n| n.id == *to);
                from_nd.map(|n| n.name.as_str()) == Some("User")
                    && to_nd.map(|n| n.name.as_str()) == Some("BaseModel")
                    && e.kind == EdgeKind::UsesType
            }),
            "expected UsesType edge from User to BaseModel, got: {edges:?}"
        );
    }

    #[test]
    fn ruby_adapter_module_method_qualified_name() {
        let src = "module Authentication\n  def authenticate(user)\n    user\n  end\nend\n";
        let (nodes, _) = parse_ruby(src);
        assert!(
            nodes.iter().any(|n| n.name == "authenticate"
                && n.kind == NodeKind::Function
                && n.qualified_name == "Authentication.authenticate"),
            "expected qualified name Authentication.authenticate, got: {nodes:?}"
        );
    }

    #[test]
    fn ruby_adapter_complexity_computed() {
        let src = r#"
class Logic
  def compute(x)
    if x > 0
      while x > 1
        x -= 1
      end
    end
  end
end
"#;
        let (nodes, _) = parse_ruby(src);
        let compute_node = nodes
            .iter()
            .find(|n| n.name == "compute" && n.kind == NodeKind::Function)
            .expect("compute method not found");
        assert!(
            compute_node.complexity.is_some(),
            "expected complexity to be computed for method"
        );
    }

    #[test]
    fn ruby_adapter_full_scenario() {
        let src = r#"
module Authentication
  def authenticate(user, password)
    validate(user)
    check_password(password)
  end

  def validate(user)
    user != nil
  end

  def check_password(password)
    password.length > 8
  end
end

class BaseModel
  def save
    true
  end
end

class User < BaseModel
  include Authentication

  def initialize(name)
    @name = name
  end

  def greet
    puts format_greeting(@name)
  end

  def self.find(id)
    Database.query(id)
  end
end
"#;
        let adapter = RubyAdapter::new();
        let parsed = adapter.parse_file(Path::new("test.rb"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);

        // Module extracted
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Authentication" && n.kind == NodeKind::Module),
            "missing Authentication module"
        );

        // Class extracted as Struct
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "User" && n.kind == NodeKind::Struct),
            "missing User struct"
        );

        // Methods with qualified names
        assert!(
            nodes
                .iter()
                .any(|n| n.qualified_name == "Authentication.authenticate"),
            "missing Authentication.authenticate"
        );
        assert!(
            nodes.iter().any(|n| n.qualified_name == "User.initialize"),
            "missing User.initialize"
        );
        assert!(
            nodes.iter().any(|n| n.qualified_name == "User.greet"),
            "missing User.greet"
        );
        assert!(
            nodes.iter().any(|n| n.qualified_name == "User.find"),
            "missing User.find (singleton method)"
        );

        // Call edges: authenticate calls validate and check_password
        let auth_calls: Vec<_> = edges
            .iter()
            .filter(|(from, _, e)| {
                nodes
                    .iter()
                    .any(|n| n.id == *from && n.name == "authenticate")
                    && e.kind == EdgeKind::Calls
            })
            .collect();
        assert!(
            auth_calls
                .iter()
                .any(|(_, to, _)| nodes.iter().any(|n| n.id == *to && n.name == "validate")),
            "expected call edge from authenticate to validate"
        );
        assert!(
            auth_calls.iter().any(|(_, to, _)| nodes
                .iter()
                .any(|n| n.id == *to && n.name == "check_password")),
            "expected call edge from authenticate to check_password"
        );
    }

    #[test]
    fn ruby_adapter_language_id_and_extensions() {
        let adapter = RubyAdapter::new();
        assert_eq!(adapter.language_id(), "ruby");
        assert_eq!(adapter.file_extensions(), &["rb"]);
    }
}
