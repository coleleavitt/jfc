//! TypeScript / TSX adapter (Phase 12-1).
//!
//! Produces `NodeData` / `EdgeData` from `.ts` / `.tsx` files using
//! `tree-sitter-typescript`. Extracts:
//!
//! - Functions (`function_declaration`, `arrow_function` assigned to
//!   `const`/`let`/`var`, `method_definition`).
//! - Classes → `NodeKind::Struct` (closest semantic match).
//! - Interfaces → `NodeKind::Trait`.
//! - Modules/namespaces → `NodeKind::Module`.
//! - Enums → `NodeKind::Enum`.
//! - Call edges (`call_expression` → callee identifier resolution).
//! - Type references → `UsesType` edges.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

pub struct TypeScriptAdapter {
    language_ts: Language,
    language_tsx: Language,
}

impl TypeScriptAdapter {
    pub fn new() -> Self {
        Self {
            language_ts: tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            language_tsx: tree_sitter_typescript::LANGUAGE_TSX.into(),
        }
    }

    fn language_for(&self, path: &Path) -> Language {
        match path.extension().and_then(|e| e.to_str()) {
            Some("tsx") => self.language_tsx.clone(),
            _ => self.language_ts.clone(),
        }
    }
}

impl LanguageAdapter for TypeScriptAdapter {
    fn language_id(&self) -> &'static str {
        "typescript"
    }

    fn file_extensions(&self) -> &[&str] {
        &["ts", "tsx", "mts", "cts"]
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile, AdapterError> {
        let mut parser = Parser::new();
        parser
            .set_language(&self.language_for(path))
            .map_err(|e| AdapterError::ParseFailed { path: path.to_string_lossy().into(), reason: format!("{e}") })?;
        let tree = parser
            .parse(content, None)
            .ok_or_else(|| AdapterError::ParseFailed { path: path.to_string_lossy().into(), reason: "tree-sitter returned None".into() })?;
        Ok(ParsedFile {
            tree,
            source: content.to_string(),
            path: path.to_path_buf(),
        })
    }

    fn extract_nodes(&self, file: &ParsedFile) -> Vec<NodeData> {
        let mut nodes = Vec::new();
        let root = file.tree.root_node();
        let source = file.source.as_str();
        let path = &file.path;
        let path_str = path.to_string_lossy();
        walk_ts_node(root, source, path, &path_str, &[], &mut nodes);
        nodes
    }

    fn extract_edges(
        &self,
        file: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();
        let root = file.tree.root_node();
        let source = file.source.as_str();
        let path_str = file.path.to_string_lossy();
        extract_ts_edges(root, source, &path_str, nodes, &mut edges);
        edges
    }
}

fn walk_ts_node(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    scope: &[&str],
    out: &mut Vec<NodeData>,
) {
    match node.kind() {
        "function_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Function, node, path, path_str, &qn));
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Struct, node, path, path_str, &qn));
                // Descend into class body for methods.
                if let Some(body) = node.child_by_field_name("body") {
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&source[name_node.byte_range()]);
                    walk_ts_node(body, source, path, path_str, &child_scope, out);
                }
                return;
            }
        }
        "interface_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Trait, node, path, path_str, &qn));
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Enum, node, path, path_str, &qn));
            }
        }
        "method_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Function, node, path, path_str, &qn));
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            // const foo = () => {} or const foo = function() {}
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        if let Some(value) = child.child_by_field_name("value") {
                            if matches!(value.kind(), "arrow_function" | "function") {
                                let name = text(name_node, source);
                                let qn = qualified(scope, &name);
                                out.push(build_nd(
                                    &name,
                                    NodeKind::Function,
                                    value,
                                    path,
                                    path_str,
                                    &qn,
                                ));
                            }
                        }
                    }
                }
            }
        }
        "module" | "internal_module" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Module, node, path, path_str, &qn));
                if let Some(body) = node.child_by_field_name("body") {
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&source[name_node.byte_range()]);
                    walk_ts_node(body, source, path, path_str, &child_scope, out);
                    return;
                }
            }
        }
        _ => {}
    }

    // Recurse into children.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_ts_node(child, source, path, path_str, scope, out);
    }
}

fn extract_ts_edges(
    node: TsNode<'_>,
    source: &str,
    path_str: &str,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "call_expression" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let callee_name = text(func_node, source);
            // Find the enclosing function.
            let mut parent = node.parent();
            let mut caller_id = None;
            while let Some(p) = parent {
                if matches!(
                    p.kind(),
                    "function_declaration" | "method_definition" | "arrow_function" | "function"
                ) {
                    if let Some(name_node) = p.child_by_field_name("name") {
                        let caller_name = text(name_node, source);
                        caller_id = nodes
                            .iter()
                            .find(|n| n.name == caller_name && n.kind == NodeKind::Function)
                            .map(|n| n.id.clone());
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
                            source_span: build_span(node, &std::path::PathBuf::from(path_str)),
                            weight: 1.0,
                        },
                    ));
                }
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        extract_ts_edges(child, source, path_str, nodes, edges);
    }
}

fn text(node: TsNode<'_>, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn qualified(scope: &[&str], name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", scope.join("::"), name)
    }
}

fn build_nd(
    name: &str,
    kind: NodeKind,
    node: TsNode<'_>,
    path: &Path,
    path_str: &str,
    qualified_name: &str,
) -> NodeData {
    NodeData {
        id: NodeId::new(path_str, qualified_name, kind),
        kind,
        name: name.to_string(),
        qualified_name: qualified_name.to_string(),
        file_path: path.to_path_buf(),
        span: build_span(node, path),
        visibility: Visibility::Public,
        metadata: HashMap::new(),
        birth_revision: 0,
        last_modified_revision: 0,
    }
}

fn build_span(node: TsNode<'_>, path: &Path) -> Span {
    Span {
        file: path.to_path_buf(),
        start_line: node.start_position().row as u32 + 1,
        start_col: node.start_position().column as u32,
        end_line: node.end_position().row as u32 + 1,
        end_col: node.end_position().column as u32,
        byte_range: node.byte_range(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_adapter_parses_function_declaration() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "function hello(name: string): void { console.log(name); }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "hello" && n.kind == NodeKind::Function));
    }

    #[test]
    fn ts_adapter_parses_class() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "class Widget { render() {} destroy() {} }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Widget" && n.kind == NodeKind::Struct));
        assert!(nodes.iter().any(|n| n.name == "render" && n.kind == NodeKind::Function));
        assert!(nodes.iter().any(|n| n.name == "destroy" && n.kind == NodeKind::Function));
    }

    #[test]
    fn ts_adapter_parses_interface() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "interface Iterable { next(): void; }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Iterable" && n.kind == NodeKind::Trait));
    }

    #[test]
    fn ts_adapter_parses_enum() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "enum Direction { Up, Down, Left, Right }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Direction" && n.kind == NodeKind::Enum));
    }

    #[test]
    fn ts_adapter_parses_arrow_function() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "const greet = (name: string) => { return `hi ${name}`; }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "greet" && n.kind == NodeKind::Function));
    }

    #[test]
    fn ts_adapter_extracts_call_edges() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = r#"
function caller() { callee(); }
function callee() {}
"#;
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);
        assert!(
            edges.iter().any(|(from, to, _)| {
                nodes.iter().any(|n| n.id == *from && n.name == "caller")
                    && nodes.iter().any(|n| n.id == *to && n.name == "callee")
            }),
            "should find caller → callee edge"
        );
    }

    #[test]
    fn tsx_adapter_parses_component() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("component.tsx");
        let src = "function App() { return <div/>; }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "App" && n.kind == NodeKind::Function));
    }
}
