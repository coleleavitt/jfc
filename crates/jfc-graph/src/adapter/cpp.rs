//! C++ adapter.
//!
//! Produces `NodeData` / `EdgeData` from `.cpp`, `.cc`, `.cxx`, `.hpp`,
//! `.hh`, `.hxx` files using `tree-sitter-cpp`. Extracts:
//!
//! - `function_definition` → `NodeKind::Function`
//! - `class_specifier` → `NodeKind::Struct`
//! - `struct_specifier` → `NodeKind::Struct`
//! - `enum_specifier` → `NodeKind::Enum`
//! - `namespace_definition` → `NodeKind::Module`
//! - Methods inside classes: qualified as `ClassName::method`
//! - Call edges (`call_expression` → `EdgeKind::Calls`)
//! - Base class specifiers → `EdgeKind::Implements`
//! - Type references → `EdgeKind::UsesType`

use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind};

pub struct CppAdapter {
    language: Language,
}

impl CppAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_cpp::LANGUAGE.into(),
        }
    }
}

impl LanguageAdapter for CppAdapter {
    fn language_id(&self) -> &'static str {
        "cpp"
    }

    fn file_extensions(&self) -> &[&str] {
        &["cpp", "cc", "cxx", "hpp", "hh", "hxx"]
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
        walk_cpp(root, &file.source, &file.path, &path_str, &[], &mut nodes);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        extract_cpp_edges(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &mut edges,
        );
        edges
    }
}

/// Recursively walk the tree-sitter tree extracting nodes.
///
/// `scope` tracks the nesting path of namespaces/classes for qualified naming.
fn walk_cpp(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    scope: &[String],
    out: &mut Vec<NodeData>,
) {
    match node.kind() {
        "namespace_definition" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, source))
                .unwrap_or_default();

            if !name.is_empty() {
                let qn = qualify(scope, &name);
                out.push(build_nd(&name, NodeKind::Module, node, path, path_str, &qn));
            }

            // Recurse into namespace body with extended scope.
            let mut new_scope = scope.to_vec();
            if !name.is_empty() {
                new_scope.push(name);
            }
            if let Some(body) = node.child_by_field_name("body") {
                let mut cursor = body.walk();
                for child in body.named_children(&mut cursor) {
                    walk_cpp(child, source, path, path_str, &new_scope, out);
                }
            }
            return; // Don't recurse further — we already recursed into body.
        }
        "class_specifier" | "struct_specifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| strip_template_params(&text(n, source)))
                .unwrap_or_default();

            if !name.is_empty() {
                let qn = qualify(scope, &name);
                out.push(build_nd(&name, NodeKind::Struct, node, path, path_str, &qn));

                // Extract methods inside the class body.
                let mut new_scope = scope.to_vec();
                new_scope.push(name);
                if let Some(body) = node.child_by_field_name("body") {
                    let mut cursor = body.walk();
                    for child in body.named_children(&mut cursor) {
                        walk_cpp(child, source, path, path_str, &new_scope, out);
                    }
                }
            }
            return; // Don't double-recurse.
        }
        "enum_specifier" => {
            let name = node
                .child_by_field_name("name")
                .map(|n| text(n, source))
                .unwrap_or_default();

            if !name.is_empty() {
                let qn = qualify(scope, &name);
                out.push(build_nd(&name, NodeKind::Enum, node, path, path_str, &qn));
            }
            // Don't recurse into enum body — no functions inside.
            return;
        }
        "function_definition" => {
            let name = extract_function_name(node, source);
            if !name.is_empty() {
                let qn = qualify(scope, &name);
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &qn);
                nd.complexity = compute_complexity(node, source.as_bytes(), "cpp");
                out.push(nd);
            }
            // Don't recurse into function body for node extraction.
            return;
        }
        _ => {}
    }

    // Default: recurse into children.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_cpp(child, source, path, path_str, scope, out);
    }
}

/// Extract edges: calls, inheritance, type references.
fn extract_cpp_edges(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    match node.kind() {
        "call_expression" => {
            let callee_name = node
                .child_by_field_name("function")
                .map(|n| extract_callee_name(n, source))
                .unwrap_or_default();

            if !callee_name.is_empty() {
                // Find enclosing function.
                if let Some(caller_id) = find_enclosing_function(node, source, nodes) {
                    // Try to resolve callee.
                    if let Some(callee) = resolve_callee(&callee_name, nodes) {
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
        "class_specifier" | "struct_specifier" => {
            // Extract base class specifiers for inheritance edges.
            let class_name = node
                .child_by_field_name("name")
                .map(|n| strip_template_params(&text(n, source)))
                .unwrap_or_default();

            if !class_name.is_empty() {
                // Find base_class_clause children.
                let mut cursor = node.walk();
                for child in node.named_children(&mut cursor) {
                    if child.kind() == "base_class_clause" {
                        extract_base_classes(child, source, path, &class_name, nodes, edges);
                    }
                }
            }
        }
        _ => {}
    }

    // Recurse.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_cpp_edges(child, source, path, nodes, edges);
    }
}

/// Extract base class names from a base_class_clause and create Implements edges.
fn extract_base_classes(
    clause: TsNode<'_>,
    source: &str,
    path: &Path,
    class_name: &str,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    // Find the derived class node.
    let derived = nodes
        .iter()
        .find(|n| n.name == class_name && n.kind == NodeKind::Struct);

    if let Some(derived_node) = derived {
        let mut cursor = clause.walk();
        for child in clause.named_children(&mut cursor) {
            // Look for type_identifier or qualified_identifier in base specifiers.
            let base_name = extract_type_name_from_node(child, source);
            if !base_name.is_empty() {
                // Try to find the base class as a Struct node (closest C++ representation).
                if let Some(base_node) = nodes
                    .iter()
                    .find(|n| n.name == base_name && n.kind == NodeKind::Struct)
                {
                    edges.push((
                        derived_node.id.clone(),
                        base_node.id.clone(),
                        EdgeData {
                            kind: EdgeKind::Implements,
                            source_span: build_span(clause, path),
                            weight: 1.0,
                        },
                    ));
                }
            }
        }
    }
}

/// Extract a type name from a node (handles type_identifier, qualified_identifier,
/// template_type, etc.)
fn extract_type_name_from_node(node: TsNode<'_>, source: &str) -> String {
    match node.kind() {
        "type_identifier" | "identifier" => text(node, source),
        "qualified_identifier" => {
            // Get the last identifier segment.
            let full = text(node, source);
            full.rsplit("::").next().unwrap_or(&full).to_string()
        }
        "template_type" => {
            // Extract the name before template params.
            node.child_by_field_name("name")
                .map(|n| text(n, source))
                .unwrap_or_default()
        }
        _ => {
            // Recurse into children looking for a type name.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let name = extract_type_name_from_node(child, source);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
    }
}

/// Extract the function name from a function_definition node.
///
/// Handles:
/// - Simple functions: `void foo()` → "foo"
/// - Qualified names: `void Class::foo()` → "foo"  
/// - Constructors: `ClassName::ClassName()` → "ClassName"
/// - Destructors: `ClassName::~ClassName()` → "~ClassName"
/// - Templates: strips template parameters
fn extract_function_name(node: TsNode<'_>, source: &str) -> String {
    // tree-sitter-cpp uses "declarator" field for the function declarator.
    let declarator = match node.child_by_field_name("declarator") {
        Some(d) => d,
        None => return String::new(),
    };

    extract_declarator_name(declarator, source)
}

/// Recursively extract the name from a declarator node.
fn extract_declarator_name(node: TsNode<'_>, source: &str) -> String {
    match node.kind() {
        "function_declarator" => {
            // The declarator field of a function_declarator holds the name.
            node.child_by_field_name("declarator")
                .map(|d| extract_declarator_name(d, source))
                .unwrap_or_default()
        }
        "identifier" | "field_identifier" => text(node, source),
        "qualified_identifier" => {
            // e.g. Class::method — extract just the last segment for the name.
            node.child_by_field_name("name")
                .map(|n| extract_declarator_name(n, source))
                .unwrap_or_else(|| {
                    // Fallback: last :: segment.
                    let full = text(node, source);
                    full.rsplit("::").next().unwrap_or(&full).to_string()
                })
        }
        "destructor_name" => {
            // ~ClassName
            let full = text(node, source);
            full
        }
        "template_function" => {
            // template<...> name — get the name part.
            node.child_by_field_name("name")
                .map(|n| extract_declarator_name(n, source))
                .unwrap_or_default()
        }
        "operator_name" => {
            // operator+, operator<<, etc.
            text(node, source)
        }
        "pointer_declarator" | "reference_declarator" => {
            // *foo or &foo — get inner declarator.
            node.child_by_field_name("declarator")
                .map(|d| extract_declarator_name(d, source))
                .unwrap_or_default()
        }
        _ => {
            // Unknown declarator kind — try first named child.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                let name = extract_declarator_name(child, source);
                if !name.is_empty() {
                    return name;
                }
            }
            String::new()
        }
    }
}

/// Extract the callee name from a call_expression's function node.
///
/// Handles:
/// - Simple: `foo()` → "foo"
/// - Member: `obj.foo()` → "foo"
/// - Scoped: `ns::foo()` → "foo"
/// - `obj->foo()` → "foo"
fn extract_callee_name(node: TsNode<'_>, source: &str) -> String {
    match node.kind() {
        "identifier" => text(node, source),
        "field_expression" => {
            // obj.method or obj->method
            node.child_by_field_name("field")
                .map(|f| text(f, source))
                .unwrap_or_default()
        }
        "qualified_identifier" => {
            // ns::func — use last segment.
            node.child_by_field_name("name")
                .map(|n| text(n, source))
                .unwrap_or_else(|| {
                    let full = text(node, source);
                    full.rsplit("::").next().unwrap_or(&full).to_string()
                })
        }
        "template_function" => node
            .child_by_field_name("name")
            .map(|n| extract_callee_name(n, source))
            .unwrap_or_default(),
        _ => text(node, source),
    }
}

/// Find the enclosing function for a node and return its NodeId.
fn find_enclosing_function(node: TsNode<'_>, source: &str, nodes: &[NodeData]) -> Option<NodeId> {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "function_definition" {
            let name = extract_function_name(p, source);
            if !name.is_empty() {
                // Match by name. Since qualified names include the scope,
                // we need to check if any function node's name matches.
                return nodes
                    .iter()
                    .find(|n| n.name == name && n.kind == NodeKind::Function)
                    .map(|n| n.id.clone());
            }
        }
        parent = p.parent();
    }
    None
}

/// Resolve a callee name to a node. Tries exact match first, then
/// unqualified match against function names.
fn resolve_callee<'a>(callee_name: &str, nodes: &'a [NodeData]) -> Option<&'a NodeData> {
    // Try exact qualified name match.
    nodes
        .iter()
        .find(|n| n.qualified_name == callee_name && n.kind == NodeKind::Function)
        .or_else(|| {
            // Try match by simple name.
            nodes
                .iter()
                .find(|n| n.name == callee_name && n.kind == NodeKind::Function)
        })
}

/// Build a qualified name by joining scope segments with `::`.
fn qualify(scope: &[String], name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", scope.join("::"), name)
    }
}

/// Strip template parameters from a name: `vector<int>` → `vector`.
fn strip_template_params(name: &str) -> String {
    if let Some(idx) = name.find('<') {
        name[..idx].trim().to_string()
    } else {
        name.to_string()
    }
}

use super::{build_nd, build_span, node_text as text};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpp_adapter_extracts_namespace() {
        let adapter = CppAdapter::new();
        let src = r#"
namespace engine {
    void foo() {}
}
"#;
        let parsed = adapter.parse_file(Path::new("test.cpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "engine" && n.kind == NodeKind::Module),
            "Expected namespace 'engine', got: {nodes:?}"
        );
    }

    #[test]
    fn cpp_adapter_extracts_class() {
        let adapter = CppAdapter::new();
        let src = r#"
class Renderer {
public:
    void draw() {}
};
"#;
        let parsed = adapter.parse_file(Path::new("test.cpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Renderer" && n.kind == NodeKind::Struct),
            "Expected class 'Renderer', got: {nodes:?}"
        );
    }

    #[test]
    fn cpp_adapter_extracts_method_with_qualified_name() {
        let adapter = CppAdapter::new();
        let src = r#"
class Renderer {
public:
    void draw() {}
    void clear() {}
};
"#;
        let parsed = adapter.parse_file(Path::new("test.cpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes.iter().any(|n| n.name == "draw"
                && n.kind == NodeKind::Function
                && n.qualified_name == "Renderer::draw"),
            "Expected method 'Renderer::draw', got: {nodes:?}"
        );
        assert!(
            nodes.iter().any(|n| n.name == "clear"
                && n.kind == NodeKind::Function
                && n.qualified_name == "Renderer::clear"),
            "Expected method 'Renderer::clear', got: {nodes:?}"
        );
    }

    #[test]
    fn cpp_adapter_extracts_struct() {
        let adapter = CppAdapter::new();
        let src = r#"
struct Point {
    int x;
    int y;
};
"#;
        let parsed = adapter.parse_file(Path::new("test.hpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Point" && n.kind == NodeKind::Struct),
            "Expected struct 'Point', got: {nodes:?}"
        );
    }

    #[test]
    fn cpp_adapter_extracts_enum() {
        let adapter = CppAdapter::new();
        let src = r#"
enum Color { Red, Green, Blue };
"#;
        let parsed = adapter.parse_file(Path::new("test.hpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Color" && n.kind == NodeKind::Enum),
            "Expected enum 'Color', got: {nodes:?}"
        );
    }

    #[test]
    fn cpp_adapter_extracts_call_edges() {
        let adapter = CppAdapter::new();
        let src = r#"
void callee() {}

void caller() {
    callee();
}
"#;
        let parsed = adapter.parse_file(Path::new("test.cpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);
        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::Calls),
            "Expected Calls edge, got: {edges:?}"
        );
    }

    #[test]
    fn cpp_adapter_full_example() {
        let adapter = CppAdapter::new();
        let src = r#"
namespace engine {

class Renderer {
public:
    void draw(const int& scene) {
        clear();
    }

    void clear() {}
};

void initialize() {
    Renderer r;
}

}
"#;
        let parsed = adapter.parse_file(Path::new("test.cpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);

        // Namespace extracted.
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "engine" && n.kind == NodeKind::Module),
            "Missing namespace 'engine'"
        );

        // Class extracted.
        assert!(
            nodes.iter().any(|n| n.name == "Renderer"
                && n.kind == NodeKind::Struct
                && n.qualified_name == "engine::Renderer"),
            "Missing class 'engine::Renderer', got: {:?}",
            nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Struct)
                .collect::<Vec<_>>()
        );

        // Methods extracted with qualified names.
        assert!(
            nodes.iter().any(|n| n.name == "draw"
                && n.kind == NodeKind::Function
                && n.qualified_name == "engine::Renderer::draw"),
            "Missing method 'engine::Renderer::draw', got: {:?}",
            nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Function)
                .collect::<Vec<_>>()
        );

        assert!(
            nodes.iter().any(|n| n.name == "clear"
                && n.kind == NodeKind::Function
                && n.qualified_name == "engine::Renderer::clear"),
            "Missing method 'engine::Renderer::clear'"
        );

        // Free function in namespace.
        assert!(
            nodes.iter().any(|n| n.name == "initialize"
                && n.kind == NodeKind::Function
                && n.qualified_name == "engine::initialize"),
            "Missing function 'engine::initialize'"
        );

        // Call edges.
        let edges = adapter.extract_edges(&parsed, &nodes);
        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::Calls),
            "Expected at least one Calls edge from draw -> clear"
        );
    }

    #[test]
    fn cpp_adapter_extracts_inheritance() {
        let adapter = CppAdapter::new();
        let src = r#"
class Base {
public:
    void foo() {}
};

class Derived : public Base {
public:
    void bar() {}
};
"#;
        let parsed = adapter.parse_file(Path::new("test.cpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);
        assert!(
            edges.iter().any(|(_, _, e)| e.kind == EdgeKind::Implements),
            "Expected Implements edge for inheritance, got: {edges:?}"
        );
    }

    #[test]
    fn cpp_adapter_handles_templates() {
        let adapter = CppAdapter::new();
        let src = r#"
template<typename T>
class Container {
public:
    void push(T item) {}
};
"#;
        let parsed = adapter.parse_file(Path::new("test.hpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Container" && n.kind == NodeKind::Struct),
            "Expected template class 'Container', got: {nodes:?}"
        );
    }

    #[test]
    fn cpp_adapter_file_extensions() {
        let adapter = CppAdapter::new();
        assert_eq!(
            adapter.file_extensions(),
            &["cpp", "cc", "cxx", "hpp", "hh", "hxx"]
        );
        assert_eq!(adapter.language_id(), "cpp");
    }

    #[test]
    fn cpp_adapter_member_call_edge() {
        let adapter = CppAdapter::new();
        let src = r#"
class Foo {
public:
    void helper() {}
    void run() {
        helper();
    }
};
"#;
        let parsed = adapter.parse_file(Path::new("test.cpp"), src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);

        // run() should call helper()
        let run_id = nodes
            .iter()
            .find(|n| n.name == "run" && n.kind == NodeKind::Function)
            .map(|n| n.id.clone());
        let helper_id = nodes
            .iter()
            .find(|n| n.name == "helper" && n.kind == NodeKind::Function)
            .map(|n| n.id.clone());

        assert!(run_id.is_some(), "Missing 'run' function");
        assert!(helper_id.is_some(), "Missing 'helper' function");

        assert!(
            edges
                .iter()
                .any(|(from, to, e)| *from == run_id.clone().unwrap()
                    && *to == helper_id.clone().unwrap()
                    && e.kind == EdgeKind::Calls),
            "Expected Calls edge from run -> helper, got: {edges:?}"
        );
    }
}
