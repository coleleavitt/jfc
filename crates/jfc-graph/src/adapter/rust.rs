use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use tracing::warn;
use tree_sitter::{Node as TsNode, Parser, Tree};

use crate::adapter::{AdapterError, LanguageAdapter, ParseOutcome, ParsedFile, first_syntax_error};
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

pub struct RustAdapter {
    parser: Mutex<Parser>,
}

impl RustAdapter {
    pub fn new() -> Self {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .expect("failed to set rust language");
        Self {
            parser: Mutex::new(parser),
        }
    }

    fn parse_tree(&self, path: &Path, content: &str) -> Result<Tree, AdapterError> {
        let mut parser = self.parser.lock().map_err(|_| AdapterError::ParseFailed {
            path: path.display().to_string(),
            reason: "rust parser mutex poisoned".into(),
        })?;
        parser
            .parse(content, None)
            .ok_or_else(|| AdapterError::ParseFailed {
                path: path.display().to_string(),
                reason: "tree-sitter returned None".into(),
            })
    }
}

impl Default for RustAdapter {
    fn default() -> Self {
        Self::new()
    }
}

impl LanguageAdapter for RustAdapter {
    fn language_id(&self) -> &str {
        "rust"
    }

    fn file_extensions(&self) -> &[&str] {
        &["rs"]
    }

    fn parse_file(&self, path: &Path, content: &str) -> Result<ParsedFile, AdapterError> {
        let tree = self.parse_tree(path, content)?;

        if let Some(err) = first_syntax_error(&tree, path, content) {
            // Surface the typed error. Callers that want the partial tree
            // anyway should use `parse_file_lenient`.
            return Err(err);
        }

        Ok(ParsedFile {
            path: path.to_path_buf(),
            source: content.to_string(),
            tree,
        })
    }

    fn parse_file_lenient(&self, path: &Path, content: &str) -> Result<ParseOutcome, AdapterError> {
        let tree = self.parse_tree(path, content)?;

        let error = first_syntax_error(&tree, path, content);
        let parsed = ParsedFile {
            path: path.to_path_buf(),
            source: content.to_string(),
            tree,
        };

        if let Some(ref err) = error {
            if let AdapterError::SyntaxError {
                start,
                end,
                summary,
                ..
            } = err
            {
                warn!(
                    target: "jfc::graph::parser",
                    path = %path.display(),
                    byte_range = ?(*start..*end),
                    summary = %summary,
                    "tree-sitter ERROR node — graph may be incomplete"
                );
            }
        }

        Ok(ParseOutcome { parsed, error })
    }

    fn extract_nodes(&self, parsed: &ParsedFile) -> Vec<NodeData> {
        let mut nodes = Vec::new();
        let root = parsed.tree.root_node();
        let file_path_str = parsed.path.to_string_lossy().to_string();

        extract_nodes_recursive(
            root,
            &parsed.source,
            &parsed.path,
            &file_path_str,
            &[],
            &mut nodes,
        );

        nodes
    }

    fn extract_edges(
        &self,
        parsed: &ParsedFile,
        nodes: &[NodeData],
    ) -> Vec<(NodeId, NodeId, EdgeData)> {
        let mut edges = Vec::new();

        let mut name_to_node: HashMap<&str, &NodeData> = HashMap::new();
        for node in nodes {
            name_to_node.insert(&node.name, node);
        }

        let root = parsed.tree.root_node();

        extract_call_edges(
            root,
            &parsed.source,
            &parsed.path,
            nodes,
            &name_to_node,
            &mut edges,
        );

        extract_type_usage_edges(
            root,
            &parsed.source,
            &parsed.path,
            nodes,
            &name_to_node,
            &mut edges,
        );

        extract_impl_edges(
            root,
            &parsed.source,
            &parsed.path,
            &name_to_node,
            &mut edges,
        );

        edges
    }
}

fn extract_nodes_recursive(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    file_path_str: &str,
    scope: &[&str],
    out: &mut Vec<NodeData>,
) {
    let mut cursor = node.walk();

    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "function_item" => {
                if let Some(nd) = extract_function(child, source, file_path, file_path_str, scope) {
                    out.push(nd);
                }
            }
            "struct_item" => {
                if let Some(nd) = extract_struct(child, source, file_path, file_path_str, scope) {
                    out.push(nd);
                }
            }
            "enum_item" => {
                if let Some(nd) = extract_enum(child, source, file_path, file_path_str, scope) {
                    out.push(nd);
                }
            }
            "mod_item" => {
                if let Some(name) = get_node_name(child, "name", source) {
                    let nd = build_node_data(
                        &name,
                        NodeKind::Module,
                        child,
                        source,
                        file_path,
                        file_path_str,
                        scope,
                        HashMap::new(),
                    );
                    out.push(nd);

                    if let Some(body) = child.child_by_field_name("body") {
                        let mut new_scope: Vec<&str> = scope.to_vec();
                        new_scope.push(name.as_str());
                        extract_nodes_recursive(
                            body,
                            source,
                            file_path,
                            file_path_str,
                            &new_scope,
                            out,
                        );
                    }
                }
            }
            "trait_item" => {
                if let Some(nd) = extract_trait(child, source, file_path, file_path_str, scope, out)
                {
                    out.push(nd);
                }
            }
            "impl_item" => {
                extract_impl(child, source, file_path, file_path_str, scope, out);
            }
            _ => {
                extract_nodes_recursive(child, source, file_path, file_path_str, scope, out);
            }
        }
    }
}

fn extract_function(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    file_path_str: &str,
    scope: &[&str],
) -> Option<NodeData> {
    let name = get_node_name(node, "name", source)?;

    // Phase 9: typed metadata — count parameters and detect `async`
    // so KindData::from_node sees structured fields rather than
    // re-parsing raw bytes. Tree-sitter-rust exposes these as
    // children of the `function_item` node.
    let mut metadata = HashMap::new();
    let param_count = node
        .child_by_field_name("parameters")
        .map(|p| {
            let mut c = p.walk();
            p.named_children(&mut c)
                .filter(|n| {
                    matches!(
                        n.kind(),
                        "parameter" | "self_parameter" | "variadic_parameter"
                    )
                })
                .count()
        })
        .unwrap_or(0);
    if param_count > 0 {
        metadata.insert("param_count".into(), param_count.to_string());
    }

    // Detect `async fn` — look for `async` modifier child.
    let mut cursor = node.walk();
    let is_async = node.children(&mut cursor).any(|c| {
        c.kind() == "async"
            || c.kind() == "function_modifiers" && {
                let mut mc = c.walk();
                c.children(&mut mc).any(|m| m.kind() == "async")
            }
    });
    if is_async {
        metadata.insert("async".into(), "true".into());
    }

    let accessed = accessed_field_names(node, source);
    if !accessed.is_empty() {
        metadata.insert("accessed_fields".into(), accessed.join(","));
    }

    Some(build_node_data(
        &name,
        NodeKind::Function,
        node,
        source,
        file_path,
        file_path_str,
        scope,
        metadata,
    ))
}

fn accessed_field_names(node: TsNode<'_>, source: &str) -> Vec<String> {
    fn walk(node: TsNode<'_>, source: &str, out: &mut Vec<String>) {
        if node.kind() == "field_expression"
            && let Some(field) = node.child_by_field_name("field")
        {
            let name = node_text(field, source);
            if !name.is_empty() && !out.contains(&name) {
                out.push(name);
            }
        }
        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            walk(child, source, out);
        }
    }
    let mut out = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        walk(body, source, &mut out);
    }
    out
}

fn extract_struct(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    file_path_str: &str,
    scope: &[&str],
) -> Option<NodeData> {
    let name = get_node_name(node, "name", source)?;

    let mut metadata = HashMap::new();

    if let Some(field_list) = node.child_by_field_name("body") {
        let mut fields = Vec::new();
        let mut field_cursor = field_list.walk();
        for field_child in field_list.named_children(&mut field_cursor) {
            if field_child.kind() == "field_declaration" {
                if let Some(field_name_node) = field_child.child_by_field_name("name") {
                    let field_name = node_text(field_name_node, source);
                    let field_type = field_child
                        .child_by_field_name("type")
                        .map(|t| node_text(t, source))
                        .unwrap_or_default();
                    fields.push(format!(
                        "{{\"name\":\"{field_name}\",\"type\":\"{field_type}\"}}"
                    ));
                }
            }
        }
        let count = fields.len();
        metadata.insert("fields".to_string(), format!("[{}]", fields.join(",")));
        metadata.insert("field_count".to_string(), count.to_string());
    }

    Some(build_node_data(
        &name,
        NodeKind::Struct,
        node,
        source,
        file_path,
        file_path_str,
        scope,
        metadata,
    ))
}

fn extract_enum(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    file_path_str: &str,
    scope: &[&str],
) -> Option<NodeData> {
    let name = get_node_name(node, "name", source)?;

    let mut metadata = HashMap::new();

    if let Some(variant_list) = node.child_by_field_name("body") {
        let mut variants = Vec::new();
        let mut variant_cursor = variant_list.walk();
        for variant_child in variant_list.named_children(&mut variant_cursor) {
            if variant_child.kind() == "enum_variant" {
                if let Some(variant_name_node) = variant_child.child_by_field_name("name") {
                    variants.push(node_text(variant_name_node, source));
                }
            }
        }
        let count = variants.len();
        metadata.insert("variants".to_string(), variants.join(","));
        metadata.insert("variant_count".to_string(), count.to_string());
    }

    Some(build_node_data(
        &name,
        NodeKind::Enum,
        node,
        source,
        file_path,
        file_path_str,
        scope,
        metadata,
    ))
}

fn extract_trait(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    file_path_str: &str,
    scope: &[&str],
    out: &mut Vec<NodeData>,
) -> Option<NodeData> {
    let name = get_node_name(node, "name", source)?;

    let mut method_names: Vec<String> = Vec::new();
    if let Some(body) = node.child_by_field_name("body") {
        let trait_name = name.as_str();
        let mut method_cursor = body.walk();
        for item in body.named_children(&mut method_cursor) {
            if item.kind() == "function_signature_item" || item.kind() == "function_item" {
                if let Some(method_name) = get_node_name(item, "name", source) {
                    method_names.push(method_name.clone());
                    let qualified =
                        build_qualified_name(scope, &format!("{trait_name}::{method_name}"));
                    let vis = detect_visibility(item, source);
                    let span = build_span(item, file_path);
                    let id = NodeId::new(file_path_str, &qualified, NodeKind::Function);

                    out.push(NodeData {
                        id,
                        kind: NodeKind::Function,
                        name: method_name,
                        qualified_name: qualified,
                        file_path: file_path.to_path_buf(),
                        span,
                        visibility: vis,
                        metadata: HashMap::new(),
                        // Stamped on insertion via `CodeGraph::add_node`.
                        birth_revision: 0,
                        last_modified_revision: 0,
                    });
                }
            }
        }
    }

    let mut trait_meta = HashMap::new();
    if !method_names.is_empty() {
        trait_meta.insert("method_count".into(), method_names.len().to_string());
        trait_meta.insert("methods".into(), method_names.join(","));
    }

    Some(build_node_data(
        &name,
        NodeKind::Trait,
        node,
        source,
        file_path,
        file_path_str,
        scope,
        trait_meta,
    ))
}

fn extract_impl(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    file_path_str: &str,
    scope: &[&str],
    out: &mut Vec<NodeData>,
) {
    let type_name = node
        .child_by_field_name("type")
        .map(|t| node_text(t, source))
        .unwrap_or_default();

    if type_name.is_empty() {
        return;
    }

    let trait_name = node
        .child_by_field_name("trait")
        .map(|t| node_text(t, source));

    let prefix = if let Some(ref tr) = trait_name {
        format!("<{type_name} as {tr}>")
    } else {
        type_name
    };

    if let Some(body) = node.child_by_field_name("body") {
        let mut cursor = body.walk();
        for item in body.named_children(&mut cursor) {
            if item.kind() == "function_item" {
                if let Some(method_name) = get_node_name(item, "name", source) {
                    let qualified =
                        build_qualified_name(scope, &format!("{prefix}::{method_name}"));
                    let vis = detect_visibility(item, source);
                    let span = build_span(item, file_path);
                    let id = NodeId::new(file_path_str, &qualified, NodeKind::Function);

                    out.push(NodeData {
                        id,
                        kind: NodeKind::Function,
                        name: method_name,
                        qualified_name: qualified,
                        file_path: file_path.to_path_buf(),
                        span,
                        visibility: vis,
                        metadata: HashMap::new(),
                        // Stamped on insertion via `CodeGraph::add_node`.
                        birth_revision: 0,
                        last_modified_revision: 0,
                    });
                }
            }
        }
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

fn get_node_name(node: TsNode<'_>, field: &str, source: &str) -> Option<String> {
    node.child_by_field_name(field)
        .map(|n| node_text(n, source))
        .filter(|s| !s.is_empty())
}

fn node_text(node: TsNode<'_>, source: &str) -> String {
    source[node.byte_range()].to_string()
}

fn detect_visibility(node: TsNode<'_>, source: &str) -> Visibility {
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        if child.kind() == "visibility_modifier" {
            let text = node_text(child, source);
            return match text.as_str() {
                "pub" => Visibility::Public,
                s if s.contains("crate") => Visibility::Crate,
                s if s.contains("super") => Visibility::Super,
                _ => Visibility::Public,
            };
        }
    }
    Visibility::Private
}

/// Build a Span from a tree-sitter node. Lines are 1-indexed, columns 0-indexed.
fn build_span(node: TsNode<'_>, file_path: &Path) -> Span {
    let start = node.start_position();
    let end = node.end_position();
    Span {
        file: file_path.to_path_buf(),
        start_line: start.row as u32 + 1,
        start_col: start.column as u32,
        end_line: end.row as u32 + 1,
        end_col: end.column as u32,
        byte_range: node.byte_range(),
    }
}

fn build_qualified_name(scope: &[&str], name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", scope.join("::"), name)
    }
}

fn build_node_data(
    name: &str,
    kind: NodeKind,
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    file_path_str: &str,
    scope: &[&str],
    metadata: HashMap<String, String>,
) -> NodeData {
    let qualified = build_qualified_name(scope, name);
    let vis = detect_visibility(node, source);
    let span = build_span(node, file_path);
    let id = NodeId::new(file_path_str, &qualified, kind);

    NodeData {
        id,
        kind,
        name: name.to_string(),
        qualified_name: qualified,
        file_path: file_path.to_path_buf(),
        span,
        visibility: vis,
        metadata,
        // Stamped on insertion via `CodeGraph::add_node`.
        birth_revision: 0,
        last_modified_revision: 0,
    }
}

// ─── Edge Extraction ────────────────────────────────────────────────────────

fn extract_call_edges(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    nodes: &[NodeData],
    name_to_node: &HashMap<&str, &NodeData>,
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "call_expression" {
            if let Some(callee_name) = extract_callee_name(child, source) {
                if let Some(caller_node) = find_enclosing_function(child, source, file_path, nodes)
                {
                    let span = build_span(child, file_path);
                    if let Some(target) = name_to_node.get(callee_name.as_str()) {
                        if target.kind == NodeKind::Function {
                            edges.push((
                                caller_node.id.clone(),
                                target.id.clone(),
                                EdgeData {
                                    kind: EdgeKind::Calls,
                                    source_span: span,
                                    weight: 1.0,
                                },
                            ));
                        }
                    } else {
                        edges.push((
                            caller_node.id.clone(),
                            NodeId::new(
                                &file_path.to_string_lossy(),
                                &callee_name,
                                NodeKind::Function,
                            ),
                            EdgeData {
                                kind: EdgeKind::UnresolvedCall(callee_name),
                                source_span: span,
                                weight: 0.5,
                            },
                        ));
                    }
                }
            }
        } else {
            extract_call_edges(child, source, file_path, nodes, name_to_node, edges);
        }
    }
}

fn extract_callee_name(call_node: TsNode<'_>, source: &str) -> Option<String> {
    let func_child = call_node.child_by_field_name("function")?;
    match func_child.kind() {
        "identifier" => Some(node_text(func_child, source)),
        "field_expression" => {
            let field = func_child.child_by_field_name("field")?;
            Some(node_text(field, source))
        }
        "scoped_identifier" => {
            let name = func_child.child_by_field_name("name")?;
            Some(node_text(name, source))
        }
        _ => None,
    }
}

fn find_enclosing_function<'a>(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    nodes: &'a [NodeData],
) -> Option<&'a NodeData> {
    let mut current = node.parent();
    while let Some(parent) = current {
        if parent.kind() == "function_item" {
            if let Some(name) = get_node_name(parent, "name", source) {
                let parent_span = build_span(parent, file_path);
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

fn extract_type_usage_edges(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    nodes: &[NodeData],
    name_to_node: &HashMap<&str, &NodeData>,
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "function_item" {
            if let Some(func_node) = find_node_for_function(child, source, file_path, nodes) {
                collect_type_refs_in_function(
                    child,
                    source,
                    file_path,
                    func_node,
                    name_to_node,
                    edges,
                );
            }
        } else {
            extract_type_usage_edges(child, source, file_path, nodes, name_to_node, edges);
        }
    }
}

fn find_node_for_function<'a>(
    func_ts_node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    nodes: &'a [NodeData],
) -> Option<&'a NodeData> {
    let name = get_node_name(func_ts_node, "name", source)?;
    let span = build_span(func_ts_node, file_path);
    nodes.iter().find(|n| {
        n.kind == NodeKind::Function && n.name == name && n.span.start_line == span.start_line
    })
}

fn collect_type_refs_in_function(
    func_node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    func_data: &NodeData,
    name_to_node: &HashMap<&str, &NodeData>,
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if let Some(params) = func_node.child_by_field_name("parameters") {
        collect_type_identifiers(params, source, file_path, func_data, name_to_node, edges);
    }
    if let Some(ret) = func_node.child_by_field_name("return_type") {
        collect_type_identifiers(ret, source, file_path, func_data, name_to_node, edges);
    }
}

fn collect_type_identifiers(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    func_data: &NodeData,
    name_to_node: &HashMap<&str, &NodeData>,
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "type_identifier" {
        let type_name = node_text(node, source);
        if let Some(target) = name_to_node.get(type_name.as_str()) {
            if matches!(
                target.kind,
                NodeKind::Struct | NodeKind::Enum | NodeKind::Trait
            ) {
                let already_exists = edges.iter().any(|(src, dst, e)| {
                    *src == func_data.id
                        && *dst == target.id
                        && matches!(e.kind, EdgeKind::UsesType)
                });
                if !already_exists {
                    edges.push((
                        func_data.id.clone(),
                        target.id.clone(),
                        EdgeData {
                            kind: EdgeKind::UsesType,
                            source_span: build_span(node, file_path),
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
        collect_type_identifiers(child, source, file_path, func_data, name_to_node, edges);
    }
}

fn extract_impl_edges(
    node: TsNode<'_>,
    source: &str,
    file_path: &Path,
    name_to_node: &HashMap<&str, &NodeData>,
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == "impl_item" {
            let type_node = child.child_by_field_name("type");
            let trait_node = child.child_by_field_name("trait");

            if let (Some(type_ts), Some(trait_ts)) = (type_node, trait_node) {
                let type_name = node_text(type_ts, source);
                let trait_name = node_text(trait_ts, source);

                if let (Some(struct_data), Some(trait_data)) = (
                    name_to_node.get(type_name.as_str()),
                    name_to_node.get(trait_name.as_str()),
                ) {
                    edges.push((
                        struct_data.id.clone(),
                        trait_data.id.clone(),
                        EdgeData {
                            kind: EdgeKind::Implements,
                            source_span: build_span(child, file_path),
                            weight: 1.0,
                        },
                    ));
                }
            }
        } else {
            extract_impl_edges(child, source, file_path, name_to_node, edges);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/sample.rs")
    }

    fn parse_fixture() -> (RustAdapter, ParsedFile) {
        let adapter = RustAdapter::new();
        let path = fixture_path();
        let content = std::fs::read_to_string(&path).expect("read fixture");
        let parsed = adapter.parse_file(&path, &content).expect("parse");
        (adapter, parsed)
    }

    #[test]
    fn rust_adapter_populates_typed_metadata_function() {
        let adapter = RustAdapter::new();
        let path = std::path::PathBuf::from("/tmp/x.rs");
        let src = "async fn handle(req: Request, ctx: Context) -> Result<Response> { Ok(()) }";
        let parsed = adapter.parse_file(&path, src).expect("parse");
        let nodes = adapter.extract_nodes(&parsed);
        let f = nodes
            .iter()
            .find(|n| n.kind == crate::nodes::NodeKind::Function && n.name == "handle")
            .expect("handle function");
        let kd = f.kind_data();
        let func = kd.function.expect("function kind data");
        assert_eq!(func.is_async, Some(true));
        assert_eq!(func.param_count, Some(2));
    }

    #[test]
    fn rust_adapter_populates_typed_metadata_struct() {
        let adapter = RustAdapter::new();
        let path = std::path::PathBuf::from("/tmp/y.rs");
        let src = "struct Point { x: i32, y: i32, z: i32 }";
        let parsed = adapter.parse_file(&path, src).expect("parse");
        let nodes = adapter.extract_nodes(&parsed);
        let s = nodes
            .iter()
            .find(|n| n.kind == crate::nodes::NodeKind::Struct && n.name == "Point")
            .expect("Point struct");
        let kd = s.kind_data();
        let st = kd.struct_.expect("struct kind data");
        assert_eq!(st.field_count, Some(3));
    }

    #[test]
    fn rust_adapter_populates_typed_metadata_enum() {
        let adapter = RustAdapter::new();
        let path = std::path::PathBuf::from("/tmp/z.rs");
        let src = "enum Color { Red, Green, Blue }";
        let parsed = adapter.parse_file(&path, src).expect("parse");
        let nodes = adapter.extract_nodes(&parsed);
        let e = nodes
            .iter()
            .find(|n| n.kind == crate::nodes::NodeKind::Enum && n.name == "Color")
            .expect("Color enum");
        let kd = e.kind_data();
        let en = kd.enum_.expect("enum kind data");
        assert_eq!(en.variant_count, Some(3));
        assert!(en.variants.contains(&"Red".to_string()));
    }

    #[test]
    fn rust_adapter_populates_typed_metadata_trait() {
        let adapter = RustAdapter::new();
        let path = std::path::PathBuf::from("/tmp/t.rs");
        let src = "trait Iter { fn next(&mut self) -> Option<u32>; fn size_hint(&self) -> usize; }";
        let parsed = adapter.parse_file(&path, src).expect("parse");
        let nodes = adapter.extract_nodes(&parsed);
        let t = nodes
            .iter()
            .find(|n| n.kind == crate::nodes::NodeKind::Trait && n.name == "Iter")
            .expect("Iter trait");
        let kd = t.kind_data();
        let tr = kd.trait_.expect("trait kind data");
        assert_eq!(tr.method_count, Some(2));
    }

    #[test]
    fn test_rust_adapter_parse_file() {
        let (_, parsed) = parse_fixture();
        assert!(!parsed.source.is_empty());
        assert_eq!(parsed.tree.root_node().kind(), "source_file");
    }

    #[test]
    fn test_rust_adapter_extract_functions() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);

        let functions: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Function)
            .collect();

        let fn_names: Vec<&str> = functions.iter().map(|n| n.name.as_str()).collect();

        assert!(fn_names.contains(&"foo"), "missing foo, got: {fn_names:?}");
        assert!(fn_names.contains(&"bar"), "missing bar, got: {fn_names:?}");
        assert!(fn_names.contains(&"baz"), "missing baz, got: {fn_names:?}");
        assert!(
            fn_names.contains(&"process"),
            "missing process, got: {fn_names:?}"
        );
        assert!(
            fn_names.contains(&"helper_one"),
            "missing helper_one, got: {fn_names:?}"
        );
        assert!(
            fn_names.contains(&"validate"),
            "missing validate, got: {fn_names:?}"
        );
    }

    #[test]
    fn test_rust_adapter_extract_structs() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);

        let structs: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Struct)
            .collect();

        assert_eq!(structs.len(), 1);
        assert_eq!(structs[0].name, "Config");

        let fields_json = structs[0]
            .metadata
            .get("fields")
            .expect("missing fields metadata");
        assert!(fields_json.contains("name"));
        assert!(fields_json.contains("port"));
        assert!(fields_json.contains("debug"));
        assert!(fields_json.contains("max_connections"));
    }

    #[test]
    fn test_rust_adapter_extract_enums() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);

        let enums: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Enum).collect();

        assert_eq!(enums.len(), 1);
        assert_eq!(enums[0].name, "Status");

        let variants = enums[0]
            .metadata
            .get("variants")
            .expect("missing variants metadata");
        assert!(variants.contains("Active"));
        assert!(variants.contains("Inactive"));
        assert!(variants.contains("Error"));
    }

    #[test]
    fn test_rust_adapter_extract_traits() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);

        let traits: Vec<_> = nodes.iter().filter(|n| n.kind == NodeKind::Trait).collect();

        assert_eq!(traits.len(), 1);
        assert_eq!(traits[0].name, "Processor");
    }

    #[test]
    fn test_rust_adapter_extract_modules() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);

        let modules: Vec<_> = nodes
            .iter()
            .filter(|n| n.kind == NodeKind::Module)
            .collect();

        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "helpers");
    }

    #[test]
    fn test_rust_adapter_visibility() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);

        let foo = nodes
            .iter()
            .find(|n| n.name == "foo" && n.kind == NodeKind::Function)
            .unwrap();
        assert_eq!(foo.visibility, Visibility::Public);

        let bar = nodes
            .iter()
            .find(|n| n.name == "bar" && n.kind == NodeKind::Function && n.qualified_name == "bar")
            .unwrap();
        assert_eq!(bar.visibility, Visibility::Private);
    }

    #[test]
    fn test_rust_adapter_qualified_names() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);

        let helper = nodes
            .iter()
            .find(|n| n.name == "helper_one")
            .expect("helper_one not found");

        assert_eq!(helper.qualified_name, "helpers::helper_one");
    }

    #[test]
    fn test_rust_call_edges() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);

        let call_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Calls))
            .collect();

        let foo_node = nodes
            .iter()
            .find(|n| n.name == "foo" && n.qualified_name == "foo")
            .unwrap();
        let bar_node = nodes
            .iter()
            .find(|n| n.name == "bar" && n.qualified_name == "bar")
            .unwrap();
        let baz_node = nodes
            .iter()
            .find(|n| n.name == "baz" && n.qualified_name == "baz")
            .unwrap();

        let foo_calls_bar = call_edges
            .iter()
            .any(|(src, dst, _)| *src == foo_node.id && *dst == bar_node.id);
        assert!(foo_calls_bar, "expected foo → bar call edge");

        let bar_calls_baz = call_edges
            .iter()
            .any(|(src, dst, _)| *src == bar_node.id && *dst == baz_node.id);
        assert!(bar_calls_baz, "expected bar → baz call edge");
    }

    #[test]
    fn test_rust_unresolved_calls() {
        let adapter = RustAdapter::new();
        let source = r#"
fn caller() {
    unknown_function();
    another::thing();
}
fn known() {}
"#;
        let path = PathBuf::from("test_unresolved.rs");
        let parsed = adapter.parse_file(&path, source).unwrap();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);

        let unresolved: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::UnresolvedCall(_)))
            .collect();

        let has_unknown = unresolved
            .iter()
            .any(|(_, _, e)| matches!(&e.kind, EdgeKind::UnresolvedCall(name) if name == "unknown_function"));
        assert!(
            has_unknown,
            "expected unresolved call to 'unknown_function', got: {unresolved:?}"
        );

        let has_thing = unresolved
            .iter()
            .any(|(_, _, e)| matches!(&e.kind, EdgeKind::UnresolvedCall(name) if name == "thing"));
        assert!(
            has_thing,
            "expected unresolved call to 'thing', got: {unresolved:?}"
        );
    }

    #[test]
    fn test_rust_impl_edges() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);

        let impl_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Implements))
            .collect();

        let config_node = nodes
            .iter()
            .find(|n| n.name == "Config" && n.kind == NodeKind::Struct)
            .unwrap();
        let processor_node = nodes
            .iter()
            .find(|n| n.name == "Processor" && n.kind == NodeKind::Trait)
            .unwrap();

        let config_implements_processor = impl_edges
            .iter()
            .any(|(src, dst, _)| *src == config_node.id && *dst == processor_node.id);
        assert!(
            config_implements_processor,
            "expected Config → Processor implements edge"
        );
    }

    #[test]
    fn test_rust_uses_type() {
        let (adapter, parsed) = parse_fixture();
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);

        let uses_type_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::UsesType))
            .collect();

        let process_node = nodes
            .iter()
            .find(|n| n.name == "process" && n.qualified_name == "process")
            .unwrap();
        let config_node = nodes
            .iter()
            .find(|n| n.name == "Config" && n.kind == NodeKind::Struct)
            .unwrap();

        let process_uses_config = uses_type_edges
            .iter()
            .any(|(src, dst, _)| *src == process_node.id && *dst == config_node.id);
        assert!(
            process_uses_config,
            "expected process → Config UsesType edge"
        );

        let status_node = nodes
            .iter()
            .find(|n| n.name == "Status" && n.kind == NodeKind::Enum)
            .unwrap();
        let process_uses_status = uses_type_edges
            .iter()
            .any(|(src, dst, _)| *src == process_node.id && *dst == status_node.id);
        assert!(
            process_uses_status,
            "expected process → Status UsesType edge"
        );
    }

    #[test]
    fn test_rust_mutual_recursion_edges() {
        let adapter = RustAdapter::new();
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/mutual_recursion.rs");
        let content = std::fs::read_to_string(&path).expect("read fixture");
        let parsed = adapter.parse_file(&path, &content).expect("parse");
        let nodes = adapter.extract_nodes(&parsed);
        let edges = adapter.extract_edges(&parsed, &nodes);

        let call_edges: Vec<_> = edges
            .iter()
            .filter(|(_, _, e)| matches!(e.kind, EdgeKind::Calls))
            .collect();

        let ping_node = nodes.iter().find(|n| n.name == "ping").unwrap();
        let pong_node = nodes.iter().find(|n| n.name == "pong").unwrap();

        let ping_calls_pong = call_edges
            .iter()
            .any(|(src, dst, _)| *src == ping_node.id && *dst == pong_node.id);
        assert!(ping_calls_pong, "expected ping → pong call edge");

        let pong_calls_ping = call_edges
            .iter()
            .any(|(src, dst, _)| *src == pong_node.id && *dst == ping_node.id);
        assert!(pong_calls_ping, "expected pong → ping call edge");
    }

    #[test]
    fn test_rust_adapter_parse_file_detects_syntax_error() {
        // Unclosed brace — tree-sitter will produce ERROR/MISSING nodes.
        let adapter = RustAdapter::new();
        let path = PathBuf::from("broken.rs");
        let bad_source = "fn caller() {\n    do_thing(\n";

        let result = adapter.parse_file(&path, bad_source);
        match result {
            Err(AdapterError::SyntaxError { .. }) => {}
            other => panic!(
                "expected SyntaxError on broken source, got {:?}",
                other.map(|_| "Ok(_)").unwrap_or("Err(other)")
            ),
        }
    }

    #[test]
    fn test_rust_adapter_parse_file_lenient_returns_partial() {
        // Lenient path keeps the partial tree AND surfaces the error so the
        // builder can index what it can while warning the caller.
        let adapter = RustAdapter::new();
        let path = PathBuf::from("broken.rs");
        let bad_source = "fn first() {}\nfn second(\n";

        let outcome = match adapter.parse_file_lenient(&path, bad_source) {
            Ok(o) => o,
            Err(e) => panic!("lenient parse must produce a partial tree: {e:?}"),
        };
        assert!(
            outcome.error.is_some(),
            "expected SyntaxError on partial tree"
        );
        assert!(matches!(
            outcome.error.as_ref(),
            Some(AdapterError::SyntaxError { .. })
        ),);
        // Tree should still be usable — we should at least see `first`.
        let nodes = adapter.extract_nodes(&outcome.parsed);
        assert!(
            nodes.iter().any(|n| n.name == "first"),
            "partial tree should still surface `first`, got: {:?}",
            nodes.iter().map(|n| n.name.as_str()).collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_rust_adapter_parse_file_clean_source_is_ok() {
        let adapter = RustAdapter::new();
        let path = PathBuf::from("clean.rs");
        let good_source = "fn ok() {}\n";

        let parsed = match adapter.parse_file(&path, good_source) {
            Ok(p) => p,
            Err(e) => panic!("clean source must parse: {e:?}"),
        };
        let outcome = match adapter.parse_file_lenient(&path, good_source) {
            Ok(o) => o,
            Err(e) => panic!("clean source must parse leniently: {e:?}"),
        };
        assert!(outcome.error.is_none());
        assert_eq!(parsed.tree.root_node().kind(), "source_file");
    }
}
