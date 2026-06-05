//! C adapter.
//!
//! Produces `NodeData` / `EdgeData` from `.c` and `.h` files using
//! `tree-sitter-c`. Extracts:
//!
//! - Functions (`function_definition`) → `NodeKind::Function`.
//! - Structs (`struct_specifier` with body) → `NodeKind::Struct`.
//! - Enums (`enum_specifier` with body) → `NodeKind::Enum`.
//! - Translation unit → `NodeKind::Module` (one per file).
//! - Call edges (`call_expression`) → `EdgeKind::Calls`.
//! - Type references in parameters/struct fields → `EdgeKind::UsesType`.

use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind};

pub struct CAdapter {
    language: Language,
}

impl CAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_c::LANGUAGE.into(),
        }
    }
}

impl LanguageAdapter for CAdapter {
    fn language_id(&self) -> &'static str {
        "c"
    }

    fn file_extensions(&self) -> &[&str] {
        &["c", "h"]
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

        // Emit a module node for the translation unit (the file itself).
        let file_name = file
            .path
            .file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| path_str.to_string());
        nodes.push(build_nd(
            &file_name,
            NodeKind::Module,
            root,
            &file.path,
            &path_str,
            &file_name,
        ));

        walk_c(root, &file.source, &file.path, &path_str, &mut nodes);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        let path_str = file.path.to_string_lossy();
        extract_c_calls(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &mut edges,
        );
        extract_c_uses_type(
            file.tree.root_node(),
            &file.source,
            &file.path,
            &path_str,
            nodes,
            &mut edges,
        );
        edges
    }
}

/// Recursively walk the tree, extracting function definitions, structs, and enums.
fn walk_c(node: TsNode<'_>, source: &str, path: &Path, path_str: &str, out: &mut Vec<NodeData>) {
    match node.kind() {
        "function_definition" => {
            // The declarator contains the function name.
            if let Some(name) = extract_function_name(node, source) {
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &name);
                nd.complexity = compute_complexity(node, source.as_bytes(), "c");
                out.push(nd);
            }
        }
        "struct_specifier" => {
            // Only extract if it has a body (field_declaration_list).
            if has_child_kind(node, "field_declaration_list") {
                if let Some(name) = extract_type_name(node, source) {
                    out.push(build_nd(
                        &name,
                        NodeKind::Struct,
                        node,
                        path,
                        path_str,
                        &name,
                    ));
                }
            }
        }
        "enum_specifier" => {
            // Only extract if it has a body (enumerator_list).
            if has_child_kind(node, "enumerator_list") {
                if let Some(name) = extract_type_name(node, source) {
                    out.push(build_nd(&name, NodeKind::Enum, node, path, path_str, &name));
                }
            }
        }
        "type_definition" => {
            // `typedef struct { ... } Name;`
            // The struct/enum inside may be anonymous, use the typedef name.
            extract_typedef_nodes(node, source, path, path_str, out);
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_c(child, source, path, path_str, out);
    }
}

/// Extract the function name from a `function_definition` node.
///
/// In C, the declarator field holds a `function_declarator` which in turn
/// has a `declarator` field (the identifier or pointer_declarator).
fn extract_function_name(node: TsNode<'_>, source: &str) -> Option<String> {
    let declarator = node.child_by_field_name("declarator")?;
    extract_identifier_from_declarator(declarator, source)
}

/// Recursively extract the identifier from a declarator chain.
/// Handles: `identifier`, `function_declarator`, `pointer_declarator`, `parenthesized_declarator`.
fn extract_identifier_from_declarator(node: TsNode<'_>, source: &str) -> Option<String> {
    match node.kind() {
        "identifier" => Some(text(node, source)),
        "function_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_identifier_from_declarator(inner, source)
        }
        "pointer_declarator" => {
            let inner = node.child_by_field_name("declarator")?;
            extract_identifier_from_declarator(inner, source)
        }
        "parenthesized_declarator" => {
            // `(declarator)` — recurse into named children.
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if let Some(name) = extract_identifier_from_declarator(child, source) {
                    return Some(name);
                }
            }
            None
        }
        _ => {
            // Try the declarator field if it exists.
            if let Some(inner) = node.child_by_field_name("declarator") {
                return extract_identifier_from_declarator(inner, source);
            }
            None
        }
    }
}

/// Extract a type name from a `struct_specifier` or `enum_specifier`.
///
/// The name appears as a `type_identifier` child (field "name").
fn extract_type_name(node: TsNode<'_>, source: &str) -> Option<String> {
    if let Some(name_node) = node.child_by_field_name("name") {
        return Some(text(name_node, source));
    }
    // Fallback: look for a direct type_identifier child.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "type_identifier" {
            return Some(text(child, source));
        }
    }
    None
}

/// Handle `typedef struct { ... } Name;` and `typedef enum { ... } Name;`.
///
/// If the inner struct/enum is anonymous, we use the typedef'd name.
fn extract_typedef_nodes(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    out: &mut Vec<NodeData>,
) {
    // tree-sitter-c parses `type_definition` with:
    //   - "type" field → the type specifier (struct_specifier/enum_specifier/etc.)
    //   - "declarator" field → the typedef name (type_identifier)
    let type_node = node.child_by_field_name("type");
    let decl_node = node.child_by_field_name("declarator");

    let typedef_name = decl_node.and_then(|d| {
        if d.kind() == "type_identifier" {
            Some(text(d, source))
        } else {
            extract_identifier_from_declarator(d, source)
        }
    });

    if let Some(type_spec) = type_node {
        let (kind, has_body) = match type_spec.kind() {
            "struct_specifier" => (
                NodeKind::Struct,
                has_child_kind(type_spec, "field_declaration_list"),
            ),
            "enum_specifier" => (NodeKind::Enum, has_child_kind(type_spec, "enumerator_list")),
            _ => return,
        };

        if !has_body {
            return;
        }

        // Prefer the struct/enum's own name, fall back to typedef name.
        let name = extract_type_name(type_spec, source)
            .or(typedef_name)
            .unwrap_or_default();

        if name.is_empty() {
            return;
        }

        // Don't add duplicate (the walk_c pass will also visit the inner
        // struct_specifier, so check for that). We handle this by only
        // emitting here if the inner specifier is anonymous.
        if extract_type_name(type_spec, source).is_none() {
            out.push(build_nd(&name, kind, type_spec, path, path_str, &name));
        }
    }
}

/// Extract call edges from `call_expression` nodes.
fn extract_c_calls(
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
            if let Some(caller_id) = find_enclosing_function(node, source, nodes) {
                if let Some(callee) = nodes
                    .iter()
                    .find(|n| n.name == callee_name && n.kind == NodeKind::Function)
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
                } else {
                    // Unresolved call (e.g. library function like printf).
                    edges.push((
                        caller_id,
                        // Use a synthetic node id for unresolved targets.
                        NodeId::new("", &callee_name, NodeKind::Function),
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
        extract_c_calls(child, source, path, nodes, edges);
    }
}

/// Extract UsesType edges from type references in function parameters and struct fields.
fn extract_c_uses_type(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    _path_str: &str,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "function_definition" {
        if let Some(fn_name) = extract_function_name(node, source) {
            if let Some(fn_node) = nodes
                .iter()
                .find(|n| n.name == fn_name && n.kind == NodeKind::Function)
            {
                // Collect type identifiers used in the parameter list and return type.
                let mut type_refs = Vec::new();
                collect_type_refs(node, source, &mut type_refs);
                for type_name in &type_refs {
                    if let Some(target) = nodes.iter().find(|n| {
                        &n.name == type_name && matches!(n.kind, NodeKind::Struct | NodeKind::Enum)
                    }) {
                        // Avoid duplicate edges.
                        let edge_exists = edges.iter().any(|(src, tgt, e)| {
                            *src == fn_node.id && *tgt == target.id && e.kind == EdgeKind::UsesType
                        });
                        if !edge_exists {
                            edges.push((
                                fn_node.id.clone(),
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
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_c_uses_type(child, source, path, _path_str, nodes, edges);
    }
}

/// Collect all `type_identifier` references within a node (for UsesType edges).
fn collect_type_refs(node: TsNode<'_>, source: &str, out: &mut Vec<String>) {
    if node.kind() == "type_identifier" {
        out.push(text(node, source));
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_type_refs(child, source, out);
    }
}

/// Find the enclosing function_definition and return its NodeId.
fn find_enclosing_function(node: TsNode<'_>, source: &str, nodes: &[NodeData]) -> Option<NodeId> {
    let mut parent = node.parent();
    while let Some(p) = parent {
        if p.kind() == "function_definition" {
            if let Some(name) = extract_function_name(p, source) {
                return nodes
                    .iter()
                    .find(|nd| nd.name == name && nd.kind == NodeKind::Function)
                    .map(|nd| nd.id.clone());
            }
            break;
        }
        parent = p.parent();
    }
    None
}

/// Check if a node has a named child of a given kind.
fn has_child_kind(node: TsNode<'_>, kind: &str) -> bool {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).any(|c| c.kind() == kind)
}

use super::{build_nd, build_span, node_text as text};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn c_adapter_parses_functions_and_struct() {
        let a = CAdapter::new();
        let src = r#"
#include <stdio.h>

struct Point {
    int x;
    int y;
};

int distance(struct Point a, struct Point b) {
    int dx = a.x - b.x;
    int dy = a.y - b.y;
    return dx*dx + dy*dy;
}

void print_point(struct Point p) {
    printf("(%d, %d)", p.x, p.y);
    distance(p, p);
}
"#;
        let parsed = a.parse_file(Path::new("test.c"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        // Should have 2 functions.
        let functions: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(
            functions.len(),
            2,
            "expected 2 functions, got: {:?}",
            functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
        assert!(functions.iter().any(|f| f.name == "distance"));
        assert!(functions.iter().any(|f| f.name == "print_point"));

        // Should have 1 struct.
        let structs: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Struct)
            .collect();
        assert_eq!(
            structs.len(),
            1,
            "expected 1 struct, got: {:?}",
            structs.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        assert_eq!(structs[0].name, "Point");

        // Should have 1 module (translation unit).
        let modules: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .collect();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "test");
    }

    #[test]
    fn c_adapter_extracts_call_edges() {
        let a = CAdapter::new();
        let src = r#"
int helper() { return 1; }

void caller() {
    helper();
}
"#;
        let parsed = a.parse_file(Path::new("calls.c"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        // Should have a call edge from caller → helper.
        let call_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| e.kind == EdgeKind::Calls)
            .collect();
        assert!(!call_edges.is_empty(), "expected call edges, got none");

        let caller_id = nodes
            .iter()
            .find(|n| n.name == "caller" && n.kind == NodeKind::Function)
            .unwrap()
            .id
            .clone();
        let helper_id = nodes
            .iter()
            .find(|n| n.name == "helper" && n.kind == NodeKind::Function)
            .unwrap()
            .id
            .clone();

        assert!(
            call_edges
                .iter()
                .any(|(src, tgt, _)| *src == caller_id && *tgt == helper_id),
            "expected caller → helper edge"
        );
    }

    #[test]
    fn c_adapter_extracts_call_edges_complex() {
        let a = CAdapter::new();
        let src = r#"
#include <stdio.h>

struct Point {
    int x;
    int y;
};

int distance(struct Point a, struct Point b) {
    int dx = a.x - b.x;
    int dy = a.y - b.y;
    return dx*dx + dy*dy;
}

void print_point(struct Point p) {
    printf("(%d, %d)", p.x, p.y);
    distance(p, p);
}
"#;
        let parsed = a.parse_file(Path::new("test.c"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        let call_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| e.kind == EdgeKind::Calls)
            .collect();

        let print_point_id = nodes
            .iter()
            .find(|n| n.name == "print_point" && n.kind == NodeKind::Function)
            .unwrap()
            .id
            .clone();
        let distance_id = nodes
            .iter()
            .find(|n| n.name == "distance" && n.kind == NodeKind::Function)
            .unwrap()
            .id
            .clone();

        // print_point → distance
        assert!(
            call_edges
                .iter()
                .any(|(src, tgt, _)| *src == print_point_id && *tgt == distance_id),
            "expected print_point → distance edge"
        );

        // print_point → printf (unresolved, points to synthetic id)
        let printf_id = NodeId::new("", "printf", NodeKind::Function);
        assert!(
            call_edges
                .iter()
                .any(|(src, tgt, _)| *src == print_point_id && *tgt == printf_id),
            "expected print_point → printf edge"
        );
    }

    #[test]
    fn c_adapter_extracts_uses_type_edges() {
        let a = CAdapter::new();
        let src = r#"
struct Point {
    int x;
    int y;
};

int distance(struct Point a, struct Point b) {
    return a.x - b.x;
}
"#;
        let parsed = a.parse_file(Path::new("types.c"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);

        let uses_type_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| e.kind == EdgeKind::UsesType)
            .collect();

        let distance_id = nodes
            .iter()
            .find(|n| n.name == "distance" && n.kind == NodeKind::Function)
            .unwrap()
            .id
            .clone();
        let point_id = nodes
            .iter()
            .find(|n| n.name == "Point" && n.kind == NodeKind::Struct)
            .unwrap()
            .id
            .clone();

        assert!(
            uses_type_edges
                .iter()
                .any(|(src, tgt, _)| *src == distance_id && *tgt == point_id),
            "expected distance → Point UsesType edge"
        );
    }

    #[test]
    fn c_adapter_parses_enum() {
        let a = CAdapter::new();
        let src = r#"
enum Color {
    RED,
    GREEN,
    BLUE
};
"#;
        let parsed = a.parse_file(Path::new("enums.c"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let enums: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Enum).collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Color");
    }

    #[test]
    fn c_adapter_parses_typedef_struct() {
        let a = CAdapter::new();
        let src = r#"
typedef struct {
    int width;
    int height;
} Rect;
"#;
        let parsed = a.parse_file(Path::new("typedef.c"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let structs: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Struct)
            .collect();
        assert_eq!(
            structs.len(),
            1,
            "expected 1 struct, got: {:?}",
            structs.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        assert_eq!(structs[0].name, "Rect");
    }

    #[test]
    fn c_adapter_header_file() {
        let a = CAdapter::new();
        let src = r#"
#ifndef POINT_H
#define POINT_H

struct Point {
    int x;
    int y;
};

int distance(struct Point a, struct Point b);

#endif
"#;
        let parsed = a.parse_file(Path::new("point.h"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        // Struct should be extracted from header.
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Point" && n.kind == NodeKind::Struct)
        );
        // Function declaration (prototype) should NOT be extracted — only definitions.
        let functions: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert!(
            functions.is_empty(),
            "prototypes should not be extracted as functions, got: {:?}",
            functions.iter().map(|f| &f.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn c_adapter_complexity() {
        let a = CAdapter::new();
        let src = r#"
int complex(int x, int y) {
    if (x > 0) {
        if (y > 0) {
            return x + y;
        }
    }
    for (int i = 0; i < x; i++) {
        while (y > 0) {
            y--;
        }
    }
    return 0;
}
"#;
        let parsed = a.parse_file(Path::new("complex.c"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);

        let func = nodes
            .iter()
            .find(|n| n.name == "complex" && n.kind == NodeKind::Function)
            .unwrap();
        let metrics = func.complexity.as_ref().unwrap();

        // Should have non-trivial complexity.
        assert!(metrics.cognitive > 0, "expected cognitive > 0");
        assert!(metrics.cyclomatic > 1, "expected cyclomatic > 1");
        assert!(metrics.max_nesting >= 2, "expected max_nesting >= 2");
    }
}
