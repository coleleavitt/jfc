//! Swift adapter.
//!
//! Extracts from `.swift` files:
//! - Classes → `NodeKind::Struct`
//! - Structs → `NodeKind::Struct`
//! - Protocols → `NodeKind::Trait`
//! - Functions/methods → `NodeKind::Function`
//! - Enums → `NodeKind::Enum`
//! - Call edges, inheritance/conformance edges.

use std::path::Path;

use tree_sitter::{Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind};

pub struct SwiftAdapter;

impl SwiftAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageAdapter for SwiftAdapter {
    fn language_id(&self) -> &'static str {
        "swift"
    }

    fn file_extensions(&self) -> &[&str] {
        &["swift"]
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile, AdapterError> {
        let mut parser = Parser::new();
        let lang: tree_sitter::Language = tree_sitter_swift::LANGUAGE.into();
        parser
            .set_language(&lang)
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
        walk_swift(root, &file.source, &file.path, &path_str, &mut nodes, None);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        let path_str = file.path.to_string_lossy();
        extract_swift_calls(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &path_str,
            &mut edges,
        );
        extract_swift_conformance(
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

fn walk_swift(
    node: TsNode,
    source: &str,
    file_path: &Path,
    path_str: &str,
    out: &mut Vec<NodeData>,
    enclosing: Option<&str>,
) {
    match node.kind() {
        "class_declaration" => {
            if let Some(name) = find_swift_name(&node, source) {
                // Swift grammar uses class_declaration for class, struct, and enum
                let src_text = source[node.byte_range()].trim_start();
                let kind = if src_text.starts_with("enum") {
                    NodeKind::Enum
                } else {
                    NodeKind::Struct
                };
                out.push(build_nd(&name, kind, node, file_path, path_str, &name));
                walk_children(node, source, file_path, path_str, out, Some(&name));
                return;
            }
        }
        "protocol_declaration" => {
            if let Some(name) = find_swift_name(&node, source) {
                out.push(build_nd(
                    &name,
                    NodeKind::Trait,
                    node,
                    file_path,
                    path_str,
                    &name,
                ));
                walk_children(node, source, file_path, path_str, out, Some(&name));
                return;
            }
        }
        "function_declaration" => {
            if let Some(name) = find_swift_name(&node, source) {
                let qualified = match enclosing {
                    Some(cls) => format!("{cls}.{name}"),
                    None => name.clone(),
                };
                let mut nd = build_nd(
                    &name,
                    NodeKind::Function,
                    node,
                    file_path,
                    path_str,
                    &qualified,
                );
                nd.complexity = compute_complexity(node, source.as_bytes(), "swift");
                out.push(nd);
                return;
            }
        }
        "init_declaration" => {
            let name = "init".to_string();
            let qualified = match enclosing {
                Some(cls) => format!("{cls}.init"),
                None => name.clone(),
            };
            let mut nd = build_nd(
                &name,
                NodeKind::Function,
                node,
                file_path,
                path_str,
                &qualified,
            );
            nd.complexity = compute_complexity(node, source.as_bytes(), "swift");
            out.push(nd);
            return;
        }
        _ => {}
    }
    walk_children(node, source, file_path, path_str, out, enclosing);
}

fn walk_children(
    node: TsNode,
    source: &str,
    file_path: &Path,
    path_str: &str,
    out: &mut Vec<NodeData>,
    enclosing: Option<&str>,
) {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_swift(child, source, file_path, path_str, out, enclosing);
    }
}

fn extract_swift_calls(
    node: TsNode,
    source: &str,
    file_path: &Path,
    nodes: &[NodeData],
    path_str: &str,
    out: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "call_expression" {
        if let Some(callee) = node.child(0) {
            let callee_name = extract_call_name(callee, source);
            if let Some(ref name) = callee_name {
                if let Some(caller_id) = enclosing_fn(node, source, nodes) {
                    let callee_id = nodes
                        .iter()
                        .find(|n| n.kind == NodeKind::Function && n.qualified_name.ends_with(name))
                        .map(|n| n.id.clone())
                        .unwrap_or_else(|| NodeId::new(path_str, name, NodeKind::Function));
                    out.push((
                        caller_id,
                        callee_id,
                        EdgeData {
                            kind: EdgeKind::Calls,
                            source_span: span_from(node, file_path),
                            weight: 1.0,
                        },
                    ));
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_swift_calls(child, source, file_path, nodes, path_str, out);
    }
}

fn extract_swift_conformance(
    node: TsNode,
    source: &str,
    file_path: &Path,
    nodes: &[NodeData],
    path_str: &str,
    out: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "class_declaration" {
        if let Some(name) = find_swift_name(&node, source) {
            let src_text = source[node.byte_range()].trim_start();
            let kind = if src_text.starts_with("enum") {
                NodeKind::Enum
            } else {
                NodeKind::Struct
            };
            let src_id = NodeId::new(path_str, &name, kind);

            // Look for inheritance_specifier children
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "inheritance_specifier" {
                    if let Some(type_name) = extract_type_from_specifier(child, source) {
                        let target_id = nodes
                            .iter()
                            .find(|n| {
                                n.name == type_name
                                    && matches!(n.kind, NodeKind::Struct | NodeKind::Trait)
                            })
                            .map(|n| n.id.clone())
                            .unwrap_or_else(|| NodeId::new(path_str, &type_name, NodeKind::Trait));
                        out.push((
                            src_id.clone(),
                            target_id,
                            EdgeData {
                                kind: EdgeKind::Implements,
                                source_span: span_from(child, file_path),
                                weight: 1.0,
                            },
                        ));
                    }
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_swift_conformance(child, source, file_path, nodes, path_str, out);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn find_swift_name(node: &TsNode, source: &str) -> Option<String> {
    // Swift grammar: name is typically `simple_identifier` or `type_identifier`
    node.child_by_field_name("name")
        .map(|n| text(&n, source).to_string())
        .or_else(|| {
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_identifier" || child.kind() == "type_identifier" {
                    return Some(text(&child, source).to_string());
                }
            }
            None
        })
}

fn extract_type_from_specifier(node: TsNode, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "user_type"
            || child.kind() == "type_identifier"
            || child.kind() == "simple_identifier"
        {
            return Some(
                text(&child, source)
                    .split('<')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string(),
            );
        }
    }
    // Fallback: just take the trimmed text minus any punctuation
    let t = text(&node, source).trim().trim_start_matches(':').trim();
    if !t.is_empty() && t != ":" {
        Some(t.split('<').next().unwrap_or(t).trim().to_string())
    } else {
        None
    }
}

fn extract_call_name(node: TsNode, source: &str) -> Option<String> {
    match node.kind() {
        "simple_identifier" => Some(text(&node, source).to_string()),
        "navigation_expression" => {
            // a.b — take last identifier
            let mut cursor = node.walk();
            let mut last = None;
            for child in node.children(&mut cursor) {
                if child.kind() == "simple_identifier" {
                    last = Some(text(&child, source).to_string());
                }
            }
            last
        }
        _ => Some(text(&node, source).split('(').next()?.trim().to_string()),
    }
}

fn enclosing_fn(node: TsNode, source: &str, nodes: &[NodeData]) -> Option<NodeId> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if matches!(parent.kind(), "function_declaration" | "init_declaration") {
            if let Some(name) = find_swift_name(&parent, source) {
                return nodes
                    .iter()
                    .find(|n| n.kind == NodeKind::Function && n.qualified_name.ends_with(&name))
                    .map(|n| n.id.clone());
            }
        }
        current = parent.parent();
    }
    None
}

fn text<'a>(node: &TsNode, source: &'a str) -> &'a str {
    &source[node.byte_range()]
}

use super::{build_nd, build_span as span_from};

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(src: &str) -> ParsedFile {
        let adapter = SwiftAdapter::new();
        adapter
            .parse_file(Path::new("test.swift"), src)
            .expect("parse")
    }

    #[test]
    fn extract_class_and_methods() {
        let src = r#"
class UserService {
    func findById(id: Int) -> User {
        return repository.find(id)
    }

    func save(user: User) {
        repository.save(user)
    }
}
"#;
        let file = parse(src);
        let adapter = SwiftAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let classes: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Struct)
            .collect();
        assert_eq!(classes.len(), 1);
        assert_eq!(classes[0].name, "UserService");

        let fns: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(fns.len(), 2);
        assert!(
            fns.iter()
                .any(|f| f.qualified_name == "UserService.findById")
        );
        assert!(fns.iter().any(|f| f.qualified_name == "UserService.save"));
    }

    #[test]
    fn extract_protocol() {
        let src = r#"
protocol Renderable {
    func render() -> String
}
"#;
        let file = parse(src);
        let adapter = SwiftAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let traits: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Trait).collect();
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].name, "Renderable");
    }

    #[test]
    fn extract_struct() {
        let src = r#"
struct Point {
    var x: Double
    var y: Double

    func distance() -> Double {
        return sqrt(x*x + y*y)
    }
}
"#;
        let file = parse(src);
        let adapter = SwiftAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let structs: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Struct)
            .collect();
        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Point");

        let fns: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].qualified_name, "Point.distance");
    }

    #[test]
    fn extract_enum() {
        let src = r#"
enum Direction {
    case north
    case south
    case east
    case west
}
"#;
        let file = parse(src);
        let adapter = SwiftAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let enums: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Enum).collect();
        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Direction");
    }

    #[test]
    fn extract_standalone_function() {
        let src = r#"
func helper(x: Int) -> Int {
    return x * 2
}
"#;
        let file = parse(src);
        let adapter = SwiftAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let fns: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();
        assert_eq!(fns.len(), 1);
        assert_eq!(fns[0].qualified_name, "helper");
    }

    #[test]
    fn extract_call_edges() {
        let src = r#"
func greet(name: String) {
    print(format(name))
}

func format(s: String) -> String {
    return s.lowercased()
}
"#;
        let file = parse(src);
        let adapter = SwiftAdapter::new();
        let nodes = adapter.extract_nodes(&file);
        let edges = adapter.extract_edges(&file, &nodes);

        let calls: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Calls))
            .collect();
        assert!(!calls.is_empty(), "expected call edges");
    }

    #[test]
    fn extract_conformance() {
        let src = r#"
protocol Printable {
    func print()
}

class Document: Printable {
    func print() {}
}
"#;
        let file = parse(src);
        let adapter = SwiftAdapter::new();
        let nodes = adapter.extract_nodes(&file);
        let edges = adapter.extract_edges(&file, &nodes);

        let impl_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Implements))
            .collect();
        assert!(!impl_edges.is_empty(), "expected conformance edge");
    }
}
