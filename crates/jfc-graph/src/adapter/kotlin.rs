//! Kotlin adapter.
//!
//! Extracts from `.kt` files:
//! - Classes → `NodeKind::Struct`
//! - Interfaces → `NodeKind::Trait`
//! - Functions/methods → `NodeKind::Function`
//! - Objects → `NodeKind::Module`
//! - Enums → `NodeKind::Enum`
//! - Call edges, inheritance edges.

use std::path::Path;

use tree_sitter::{Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind};

pub struct KotlinAdapter;

impl KotlinAdapter {
    pub fn new() -> Self {
        Self
    }
}

impl LanguageAdapter for KotlinAdapter {
    fn language_id(&self) -> &'static str {
        "kotlin"
    }

    fn file_extensions(&self) -> &[&str] {
        &["kt", "kts"]
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile, AdapterError> {
        let mut parser = Parser::new();
        let lang: tree_sitter::Language = tree_sitter_kotlin_sg::LANGUAGE.into();
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
        walk_kotlin(root, &file.source, &file.path, &path_str, &mut nodes, None);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        let path_str = file.path.to_string_lossy();
        extract_calls(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &path_str,
            &mut edges,
        );
        extract_inheritance(
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

fn walk_kotlin(
    node: TsNode,
    source: &str,
    file_path: &Path,
    path_str: &str,
    out: &mut Vec<NodeData>,
    enclosing: Option<&str>,
) {
    match node.kind() {
        "class_declaration" => {
            if let Some(name) = find_name(&node, source) {
                // Check if it's an interface or enum class
                let first_child_kind = node.child(0).map(|c| text(&c, source));
                if first_child_kind == Some("interface") {
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
                if first_child_kind == Some("enum") {
                    out.push(build_nd(
                        &name,
                        NodeKind::Enum,
                        node,
                        file_path,
                        path_str,
                        &name,
                    ));
                    walk_children(node, source, file_path, path_str, out, Some(&name));
                    return;
                }
                out.push(build_nd(
                    &name,
                    NodeKind::Struct,
                    node,
                    file_path,
                    path_str,
                    &name,
                ));
                walk_children(node, source, file_path, path_str, out, Some(&name));
                return;
            }
        }
        "interface_declaration" => {
            if let Some(name) = find_name(&node, source) {
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
        "object_declaration" => {
            if let Some(name) = find_name(&node, source) {
                out.push(build_nd(
                    &name,
                    NodeKind::Module,
                    node,
                    file_path,
                    path_str,
                    &name,
                ));
                walk_children(node, source, file_path, path_str, out, Some(&name));
                return;
            }
        }
        "enum_class_body" => {
            // Parent is enum — already handled
        }
        "function_declaration" => {
            if let Some(name) = find_name(&node, source) {
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
                nd.complexity = compute_complexity(node, source.as_bytes(), "kotlin");
                out.push(nd);
                return;
            }
        }
        _ => {
            // Check for enum_class at parent level
            if node.kind() == "class_declaration" {
                // Already handled above
            }
        }
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
        walk_kotlin(child, source, file_path, path_str, out, enclosing);
    }
}

fn extract_calls(
    node: TsNode,
    source: &str,
    file_path: &Path,
    nodes: &[NodeData],
    path_str: &str,
    out: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "call_expression" {
        if let Some(callee_name) = call_target_name(&node, source) {
            if let Some(caller_id) = enclosing_fn(node, source, nodes) {
                let callee_id = nodes
                    .iter()
                    .find(|n| {
                        n.kind == NodeKind::Function && n.qualified_name.ends_with(&callee_name)
                    })
                    .map(|n| n.id.clone())
                    .unwrap_or_else(|| NodeId::new(path_str, &callee_name, NodeKind::Function));
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

    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        extract_calls(child, source, file_path, nodes, path_str, out);
    }
}

fn extract_inheritance(
    node: TsNode,
    source: &str,
    file_path: &Path,
    nodes: &[NodeData],
    path_str: &str,
    out: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if matches!(node.kind(), "class_declaration") {
        if let Some(name) = find_name(&node, source) {
            // Determine source kind based on first keyword
            let first_kw = node.child(0).map(|c| text(&c, source));
            let kind = match first_kw {
                Some("interface") => NodeKind::Trait,
                _ => NodeKind::Struct,
            };
            let src_id = NodeId::new(path_str, &name, kind);

            // Look for delegation_specifier children (direct children of class_declaration)
            let mut cursor = node.walk();
            for child in node.children(&mut cursor) {
                if child.kind() == "delegation_specifier" {
                    if let Some(type_name) = extract_type_name(child, source) {
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
        extract_inheritance(child, source, file_path, nodes, path_str, out);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn find_name(node: &TsNode, source: &str) -> Option<String> {
    // Kotlin grammar: name is typically a `simple_identifier` child
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "type_identifier" || child.kind() == "simple_identifier" {
            return Some(text(&child, source).to_string());
        }
    }
    None
}

fn call_target_name(node: &TsNode, source: &str) -> Option<String> {
    let first = node.child(0)?;
    match first.kind() {
        "simple_identifier" => Some(text(&first, source).to_string()),
        "navigation_expression" => {
            // obj.method — take last segment
            let mut cursor = first.walk();
            let mut last = None;
            for child in first.children(&mut cursor) {
                if child.kind() == "simple_identifier" {
                    last = Some(text(&child, source).to_string());
                }
            }
            last
        }
        _ => Some(text(&first, source).to_string()),
    }
}

fn extract_type_name(node: TsNode, source: &str) -> Option<String> {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "user_type"
            || child.kind() == "type_identifier"
            || child.kind() == "simple_identifier"
        {
            let t = text(&child, source);
            // Strip generic args
            return Some(t.split('<').next().unwrap_or(t).trim().to_string());
        }
        // Recurse one level
        if let Some(name) = extract_type_name(child, source) {
            return Some(name);
        }
    }
    None
}

fn enclosing_fn(node: TsNode, source: &str, nodes: &[NodeData]) -> Option<NodeId> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_declaration" {
            if let Some(name) = find_name(&parent, source) {
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
        let adapter = KotlinAdapter::new();
        adapter
            .parse_file(Path::new("test.kt"), src)
            .expect("parse")
    }

    #[test]
    fn extract_class_and_methods() {
        let src = r#"
class UserService {
    fun findById(id: Int): User {
        return repository.find(id)
    }

    fun save(user: User) {
        repository.save(user)
    }
}
"#;
        let file = parse(src);
        let adapter = KotlinAdapter::new();
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
    fn extract_interface() {
        let src = r#"
interface Repository {
    fun findAll(): List<Item>
}
"#;
        let file = parse(src);
        let adapter = KotlinAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let traits: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Trait).collect();
        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].name, "Repository");
    }

    #[test]
    fn extract_object_as_module() {
        let src = r#"
object DatabaseConfig {
    fun getUrl(): String {
        return "jdbc:..."
    }
}
"#;
        let file = parse(src);
        let adapter = KotlinAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let modules: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .collect();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "DatabaseConfig");
    }

    #[test]
    fn extract_standalone_function() {
        let src = r#"
fun helper(x: Int): Int {
    return x * 2
}
"#;
        let file = parse(src);
        let adapter = KotlinAdapter::new();
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
fun greet(name: String) {
    println(format(name))
}

fun format(s: String): String {
    return s.uppercase()
}
"#;
        let file = parse(src);
        let adapter = KotlinAdapter::new();
        let nodes = adapter.extract_nodes(&file);
        let edges = adapter.extract_edges(&file, &nodes);

        let calls: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Calls))
            .collect();
        assert!(!calls.is_empty(), "expected call edges");
    }

    #[test]
    fn extract_inheritance() {
        let src = r#"
interface Printable {
    fun print()
}

class Document : Printable {
    override fun print() {}
}
"#;
        let file = parse(src);
        let adapter = KotlinAdapter::new();
        let nodes = adapter.extract_nodes(&file);
        let edges = adapter.extract_edges(&file, &nodes);

        let impl_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Implements))
            .collect();
        assert!(!impl_edges.is_empty(), "expected implements edge");
    }

    #[test]
    fn complexity_for_functions() {
        let src = r#"
fun compute(x: Int): Int {
    if (x > 0) {
        for (i in 0 until x) {
            if (i % 2 == 0) {
                return i
            }
        }
    }
    return 0
}
"#;
        let file = parse(src);
        let adapter = KotlinAdapter::new();
        let nodes = adapter.extract_nodes(&file);

        let f = nodes.iter().find(|n| n.name == "compute").expect("compute");
        assert!(f.complexity.is_some());
    }
}
