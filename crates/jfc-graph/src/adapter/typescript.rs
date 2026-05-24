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
//! - Enums → `NodeKind::Enum`, with variants emitted as
//!   `NodeKind::EnumVariant` children.
//! - Class fields (`public_field_definition`) and interface property
//!   signatures (`property_signature`) → `NodeKind::Field`.
//! - Type aliases (`type_alias_declaration`) → `NodeKind::TypeAlias`.
//! - Top-level / class-level `const` declarations → `NodeKind::Constant`.
//! - Call edges (`call_expression` → callee identifier resolution).
//! - Type references → `UsesType` edges.

use std::collections::HashMap;
use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
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
                let mut nd = build_fn_nd(&name, node, path, path_str, &qn, source);
                if let Some(gp) = extract_ts_type_params(node, source) {
                    nd.metadata.insert("generic_params".into(), gp);
                }
                if let Some(cta) = extract_ts_callee_type_args(node, source) {
                    nd.metadata.insert("callee_type_args".into(), cta);
                }
                out.push(nd);
            }
        }
        "class_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                let mut nd = build_nd(&name, NodeKind::Struct, node, path, path_str, &qn);
                if let Some(gp) = extract_ts_type_params(node, source) {
                    nd.metadata.insert("generic_params".into(), gp);
                }
                out.push(nd);
                // Emit Field nodes for class fields before descending so
                // methods picked up by the recursive walk don't shadow them.
                extract_ts_class_fields(node, source, path, path_str, &qn, out);
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
                // Interface properties also become Field nodes.
                extract_ts_interface_fields(node, source, path, path_str, &qn, out);
            }
        }
        "enum_declaration" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Enum, node, path, path_str, &qn));
                extract_ts_enum_variants(node, source, path, path_str, &qn, out);
            }
        }
        "type_alias_declaration" => {
            // `type Foo = Bar` — first named child / `name` field is the type identifier.
            let name_node = node
                .child_by_field_name("name")
                .or_else(|| node.named_child(0));
            if let Some(name_node) = name_node {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(
                    &name,
                    NodeKind::TypeAlias,
                    node,
                    path,
                    path_str,
                    &qn,
                ));
            }
        }
        "method_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                let mut nd = build_fn_nd(&name, node, path, path_str, &qn, source);
                if let Some(gp) = extract_ts_type_params(node, source) {
                    nd.metadata.insert("generic_params".into(), gp);
                }
                if let Some(cta) = extract_ts_callee_type_args(node, source) {
                    nd.metadata.insert("callee_type_args".into(), cta);
                }
                out.push(nd);
            }
        }
        "lexical_declaration" | "variable_declaration" => {
            // Determine whether this is a `const` declaration.
            let is_const = node
                .child(0)
                .map(|c| &source[c.byte_range()] == "const")
                .unwrap_or(false);

            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if child.kind() == "variable_declarator" {
                    if let Some(name_node) = child.child_by_field_name("name") {
                        let name = text(name_node, source);
                        let qn = qualified(scope, &name);
                        if let Some(value) = child.child_by_field_name("value") {
                            if matches!(value.kind(), "arrow_function" | "function") {
                                let mut nd =
                                    build_fn_nd(&name, value, path, path_str, &qn, source);
                                if let Some(gp) = extract_ts_type_params(value, source) {
                                    nd.metadata.insert("generic_params".into(), gp);
                                }
                                if let Some(cta) = extract_ts_callee_type_args(value, source) {
                                    nd.metadata.insert("callee_type_args".into(), cta);
                                }
                                out.push(nd);
                                continue;
                            }
                        }
                        // Non-function `const` → emit as a Constant node.
                        if is_const {
                            out.push(build_nd(
                                &name,
                                NodeKind::Constant,
                                child,
                                path,
                                path_str,
                                &qn,
                            ));
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

/// Emit `NodeKind::EnumVariant` nodes for each `enum_assignment` /
/// `property_identifier` directly under an `enum_body`.
///
/// `parent_qn` is the qualified name of the enclosing enum (e.g. `Color`)
/// — variants are namespaced under it (`Color::Red`).
fn extract_ts_enum_variants(
    enum_node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    parent_qn: &str,
    out: &mut Vec<NodeData>,
) {
    let body = match enum_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        // `enum_assignment` (with explicit value) wraps a property_identifier;
        // bare variants are `property_identifier` directly.
        let name_node = match child.kind() {
            "enum_assignment" => child.named_child(0),
            "property_identifier" => Some(child),
            _ => continue,
        };
        let Some(name_node) = name_node else { continue };
        if name_node.kind() != "property_identifier" {
            continue;
        }
        let name = text(name_node, source);
        let qn = format!("{parent_qn}::{name}");
        out.push(build_nd(
            &name,
            NodeKind::EnumVariant,
            child,
            path,
            path_str,
            &qn,
        ));
    }
}

/// Emit `NodeKind::Field` nodes for each `public_field_definition` in a class.
fn extract_ts_class_fields(
    class_node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    class_qn: &str,
    out: &mut Vec<NodeData>,
) {
    let body = match class_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() != "public_field_definition" {
            continue;
        }
        let name_node = child
            .child_by_field_name("name")
            .or_else(|| find_first_kind(child, "property_identifier"));
        let Some(name_node) = name_node else { continue };
        let name = text(name_node, source);
        let qn = format!("{class_qn}::{name}");
        out.push(build_nd(&name, NodeKind::Field, child, path, path_str, &qn));
    }
}

/// Emit `NodeKind::Field` nodes for each `property_signature` in an interface.
fn extract_ts_interface_fields(
    iface_node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    iface_qn: &str,
    out: &mut Vec<NodeData>,
) {
    let body = match iface_node.child_by_field_name("body") {
        Some(b) => b,
        None => return,
    };
    let mut cursor = body.walk();
    for child in body.named_children(&mut cursor) {
        if child.kind() != "property_signature" {
            continue;
        }
        let name_node = child
            .child_by_field_name("name")
            .or_else(|| find_first_kind(child, "property_identifier"));
        let Some(name_node) = name_node else { continue };
        let name = text(name_node, source);
        let qn = format!("{iface_qn}::{name}");
        out.push(build_nd(&name, NodeKind::Field, child, path, path_str, &qn));
    }
}

/// First named child of `node` whose `kind()` matches `kind`.
fn find_first_kind<'a>(node: TsNode<'a>, kind: &str) -> Option<TsNode<'a>> {
    let mut cursor = node.walk();
    node.named_children(&mut cursor).find(|c| c.kind() == kind)
}

// ─── Generic Params / Type-Arg Extraction ───────────────────────────────────

/// Extract generic type parameter names from a `type_parameters` child.
/// Returns a JSON array string like `["T", "U"]`, or `None` if no generics.
fn extract_ts_type_params(node: TsNode<'_>, source: &str) -> Option<String> {
    let tp = node
        .children(&mut node.walk())
        .find(|c| c.kind() == "type_parameters")?;
    let mut params: Vec<String> = Vec::new();
    let mut cursor = tp.walk();
    for child in tp.named_children(&mut cursor) {
        if child.kind() == "type_parameter" {
            if let Some(ident) = child.child_by_field_name("name") {
                params.push(text(ident, source));
            } else {
                // Fallback: first named child that's a type_identifier
                let mut cc = child.walk();
                if let Some(ti) = child
                    .named_children(&mut cc)
                    .find(|c| c.kind() == "type_identifier")
                {
                    params.push(text(ti, source));
                }
            }
        }
    }
    if params.is_empty() {
        return None;
    }
    Some(serde_json::to_string(&params).unwrap_or_default())
}

/// Walk a function body for calls with type arguments and collect them.
/// Returns a JSON object string, or `None` if no type-arg calls found.
fn extract_ts_callee_type_args(func_node: TsNode<'_>, source: &str) -> Option<String> {
    // Look for a body/statement_block child
    let body = func_node
        .child_by_field_name("body")
        .or_else(|| {
            func_node
                .children(&mut func_node.walk())
                .find(|c| c.kind() == "statement_block")
        })?;
    let mut map: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    collect_ts_type_arg_calls(body, source, &mut map);
    if map.is_empty() {
        return None;
    }
    Some(serde_json::to_string(&map).unwrap_or_default())
}

/// Recursively find `call_expression` nodes with `type_arguments` children.
fn collect_ts_type_arg_calls(
    node: TsNode<'_>,
    source: &str,
    map: &mut std::collections::BTreeMap<String, Vec<String>>,
) {
    if node.kind() == "call_expression" {
        // In TS, call_expression has: function (identifier), type_arguments, arguments
        let func_node = node.child_by_field_name("function");
        let ta_node = node
            .children(&mut node.walk())
            .find(|c| c.kind() == "type_arguments");
        if let (Some(func), Some(ta)) = (func_node, ta_node) {
            let callee_name = text(func, source);
            let args = collect_ts_type_arg_names(ta, source);
            if !args.is_empty() && !callee_name.is_empty() {
                map.entry(callee_name).or_default().extend(args);
            }
        }
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_ts_type_arg_calls(child, source, map);
    }
}

/// Collect type names from a TypeScript `type_arguments` node.
fn collect_ts_type_arg_names(ta_node: TsNode<'_>, source: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut cursor = ta_node.walk();
    for child in ta_node.named_children(&mut cursor) {
        let t = text(child, source);
        if !t.is_empty() {
            args.push(t);
        }
    }
    args
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
        complexity: None,
        cfg: None,
        dataflow: None,
    }
}

/// Build a function NodeData with complexity metrics attached.
fn build_fn_nd(
    name: &str,
    node: TsNode<'_>,
    path: &Path,
    path_str: &str,
    qualified_name: &str,
    source: &str,
) -> NodeData {
    let mut nd = build_nd(
        name,
        NodeKind::Function,
        node,
        path,
        path_str,
        qualified_name,
    );
    nd.complexity = compute_complexity(node, source.as_bytes(), "typescript");
    nd
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
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "hello" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn ts_adapter_parses_class() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "class Widget { render() {} destroy() {} }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Widget" && n.kind == NodeKind::Struct)
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "render" && n.kind == NodeKind::Function)
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "destroy" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn ts_adapter_parses_interface() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "interface Iterable { next(): void; }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Iterable" && n.kind == NodeKind::Trait)
        );
    }

    #[test]
    fn ts_adapter_parses_enum() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "enum Direction { Up, Down, Left, Right }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Direction" && n.kind == NodeKind::Enum)
        );
    }

    #[test]
    fn ts_adapter_parses_arrow_function() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "const greet = (name: string) => { return `hi ${name}`; }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "greet" && n.kind == NodeKind::Function)
        );
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
    fn ts_adapter_extracts_enum_variants() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "enum Direction { Up, Down, Left, Right }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Direction" && n.kind == NodeKind::Enum));
        for v in &["Up", "Down", "Left", "Right"] {
            assert!(
                nodes.iter().any(|n| n.name == *v && n.kind == NodeKind::EnumVariant),
                "missing EnumVariant node for {v}"
            );
        }
    }

    #[test]
    fn ts_adapter_extracts_enum_variants_with_values() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = r#"enum Color { Red = "RED", Green = "GREEN" }"#;
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "Red" && n.kind == NodeKind::EnumVariant));
        assert!(nodes.iter().any(|n| n.name == "Green" && n.kind == NodeKind::EnumVariant));
    }

    #[test]
    fn ts_adapter_extracts_class_fields() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "class Widget {\n  name: string;\n  count: number;\n  render() {}\n}";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes.iter().any(|n| n.name == "name" && n.kind == NodeKind::Field),
            "expected Field node for 'name', got: {:?}",
            nodes.iter().filter(|n| n.kind == NodeKind::Field).collect::<Vec<_>>()
        );
        assert!(nodes.iter().any(|n| n.name == "count" && n.kind == NodeKind::Field));
        // render should still be a Function, not a Field.
        assert!(nodes.iter().any(|n| n.name == "render" && n.kind == NodeKind::Function));
    }

    #[test]
    fn ts_adapter_extracts_interface_fields() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "interface Config {\n  host: string;\n  port: number;\n}";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(nodes.iter().any(|n| n.name == "host" && n.kind == NodeKind::Field));
        assert!(nodes.iter().any(|n| n.name == "port" && n.kind == NodeKind::Field));
    }

    #[test]
    fn ts_adapter_extracts_type_alias() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "type ID = string;\ntype Result<T> = Success<T> | Failure;";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes.iter().any(|n| n.name == "ID" && n.kind == NodeKind::TypeAlias),
            "expected TypeAlias for ID"
        );
        assert!(
            nodes.iter().any(|n| n.name == "Result" && n.kind == NodeKind::TypeAlias),
            "expected TypeAlias for Result"
        );
    }

    #[test]
    fn ts_adapter_extracts_const_as_constant() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = "const MAX_SIZE = 100;\nconst greet = () => {};";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes.iter().any(|n| n.name == "MAX_SIZE" && n.kind == NodeKind::Constant),
            "expected Constant for MAX_SIZE"
        );
        // greet should still be a Function, not a Constant.
        assert!(
            nodes.iter().any(|n| n.name == "greet" && n.kind == NodeKind::Function),
            "expected Function for greet (arrow fn)"
        );
    }

    #[test]
    fn tsx_adapter_parses_component() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("component.tsx");
        let src = "function App() { return <div/>; }";
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "App" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn ts_adapter_generic_params_and_callee_type_args() {
        let adapter = TypeScriptAdapter::new();
        let path = Path::new("test.ts");
        let src = r#"function identity<T>(x: T): T { return x; }
function main() { identity<string>("hi"); }
"#;
        let parsed = adapter.parse_file(path, src).unwrap();
        let nodes = adapter.extract_nodes(&parsed);

        // identity should have generic_params = ["T"]
        let identity_fn = nodes
            .iter()
            .find(|n| n.name == "identity" && n.kind == NodeKind::Function)
            .expect("identity function");
        let gp = identity_fn
            .metadata
            .get("generic_params")
            .expect("missing generic_params on identity");
        let params: Vec<String> = serde_json::from_str(gp).expect("parse generic_params");
        assert_eq!(params, vec!["T"]);

        // main should have callee_type_args = {"identity": ["string"]}
        let main_fn = nodes
            .iter()
            .find(|n| n.name == "main" && n.kind == NodeKind::Function)
            .expect("main function");
        let cta = main_fn
            .metadata
            .get("callee_type_args")
            .expect("missing callee_type_args on main");
        let map: std::collections::BTreeMap<String, Vec<String>> =
            serde_json::from_str(cta).expect("parse callee_type_args");
        assert_eq!(
            map.get("identity").expect("identity key"),
            &vec!["string".to_string()]
        );
    }
}
