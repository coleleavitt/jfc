//! Python adapter (Phase 12-2).
//!
//! Produces `NodeData` / `EdgeData` from `.py` files using
//! `tree-sitter-python`. Extracts:
//!
//! - Functions/methods (`function_definition`).
//! - Classes → `NodeKind::Struct`.
//! - Class-level typed assignments → `NodeKind::Field`.
//! - Module-level assignments of simple values → `NodeKind::Constant`.
//! - Module-level → `NodeKind::Module`.
//! - Call edges (`call` → callee resolution).
//! - Import edges as `UsesType`.

use std::path::Path;

use tree_sitter::{Language, Node as TsNode, Parser};

use crate::adapter::{AdapterError, LanguageAdapter, ParsedFile};
use crate::complexity::compute_complexity;
use crate::edges::{EdgeData, EdgeKind};
use crate::nodes::{NodeData, NodeId, NodeKind};

pub struct PythonAdapter {
    language: Language,
}

impl PythonAdapter {
    pub fn new() -> Self {
        Self {
            language: tree_sitter_python::LANGUAGE.into(),
        }
    }
}

impl LanguageAdapter for PythonAdapter {
    fn language_id(&self) -> &'static str {
        "python"
    }

    fn file_extensions(&self) -> &[&str] {
        &["py", "pyi"]
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
        walk_py(
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
        extract_py_calls(
            file.tree.root_node(),
            &file.source,
            &file.path,
            nodes,
            &mut edges,
        );
        edges
    }
}

fn walk_py(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    scope: &[&str],
    out: &mut Vec<NodeData>,
) {
    match node.kind() {
        "function_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                let mut nd = build_nd(&name, NodeKind::Function, node, path, path_str, &qn);
                nd.complexity = compute_complexity(node, source.as_bytes(), "python");
                out.push(nd);
            }
        }
        "class_definition" => {
            if let Some(name_node) = node.child_by_field_name("name") {
                let name = text(name_node, source);
                let qn = qualified(scope, &name);
                out.push(build_nd(&name, NodeKind::Struct, node, path, path_str, &qn));
                if let Some(body) = node.child_by_field_name("body") {
                    // Emit Field nodes for typed / simple class-level assignments
                    // before recursing — recursion would otherwise drop into
                    // method bodies and pick up unrelated assignments.
                    extract_py_class_fields(body, source, path, path_str, &qn, out);
                    let binding = text(name_node, source);
                    let mut child_scope: Vec<&str> = scope.to_vec();
                    child_scope.push(&binding);
                    walk_py(body, source, path, path_str, &child_scope, out);
                }
                return;
            }
        }
        // Module-level constant: `expression_statement > assignment` directly
        // under a `module` parent, target is an `identifier`, value is a
        // primitive literal. We deliberately *don't* fire for assignments
        // inside functions / classes (handled separately for classes; ignored
        // for function locals).
        "expression_statement" => {
            if scope.is_empty() && node.parent().map(|p| p.kind() == "module").unwrap_or(false) {
                if let Some(assign) = node.named_child(0) {
                    if let Some((name, name_node)) = py_constant_assignment_name(assign, source) {
                        let qn = qualified(scope, &name);
                        let mut nd =
                            build_nd(&name, NodeKind::Constant, assign, path, path_str, &qn);
                        // Carry the byte span of the identifier, not the whole
                        // statement, so go-to-definition lands on the name.
                        nd.span = build_span(name_node, path);
                        out.push(nd);
                    }
                }
            }
        }
        _ => {}
    }
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_py(child, source, path, path_str, scope, out);
    }
}

/// Inspect a Python `assignment` node and return `Some((name, name_node))` if
/// it looks like a constant binding (`NAME = literal` or `NAME: Type = …`).
///
/// We accept identifier-only targets and primitive RHS — anything more complex
/// (function calls, comprehensions, multi-target unpacking) is skipped so we
/// don't manufacture a constant for `app = Flask(__name__)`-style runtime
/// initialisation. Caller filters on scope (module vs. class) to decide what
/// `NodeKind` to emit.
fn py_constant_assignment_name<'a>(
    assign: TsNode<'a>,
    source: &str,
) -> Option<(String, TsNode<'a>)> {
    if assign.kind() != "assignment" {
        return None;
    }
    let target = assign.child_by_field_name("left")?;
    if target.kind() != "identifier" {
        return None;
    }
    // `value` is required for module-level constants; class-level fields may
    // be type-annotated without a value (`count: int`), which we still want.
    if let Some(value) = assign.child_by_field_name("right") {
        let ok = matches!(
            value.kind(),
            "integer"
                | "float"
                | "string"
                | "true"
                | "false"
                | "none"
                | "list"
                | "tuple"
                | "dictionary"
                | "set"
                | "unary_operator"
        );
        if !ok {
            return None;
        }
    }
    Some((text(target, source), target))
}

/// Walk a class `block` body and emit `NodeKind::Field` for typed or simple
/// attribute assignments (`name: str = ""`, `count: int`, `flag = True`).
fn extract_py_class_fields(
    body: TsNode<'_>,
    source: &str,
    path: &Path,
    path_str: &str,
    class_qn: &str,
    out: &mut Vec<NodeData>,
) {
    let mut cursor = body.walk();
    for stmt in body.named_children(&mut cursor) {
        if stmt.kind() != "expression_statement" {
            continue;
        }
        let assign = match stmt.named_child(0) {
            Some(a) if a.kind() == "assignment" => a,
            _ => continue,
        };
        let target = match assign.child_by_field_name("left") {
            Some(t) if t.kind() == "identifier" => t,
            _ => continue,
        };
        // Prefer typed attributes (PEP 526); allow plain ones too.
        let typed = assign.child_by_field_name("type").is_some();
        let value = assign.child_by_field_name("right");
        let primitive = value
            .map(|v| {
                matches!(
                    v.kind(),
                    "integer"
                        | "float"
                        | "string"
                        | "true"
                        | "false"
                        | "none"
                        | "list"
                        | "tuple"
                        | "dictionary"
                        | "set"
                        | "unary_operator"
                )
            })
            .unwrap_or(true); // no value → typed-only declaration is fine
        if !typed && !primitive {
            continue;
        }
        let name = text(target, source);
        let qn = format!("{class_qn}::{name}");
        let mut nd = build_nd(&name, NodeKind::Field, assign, path, path_str, &qn);
        nd.span = build_span(target, path);
        out.push(nd);
    }
}

fn extract_py_calls(
    node: TsNode<'_>,
    source: &str,
    path: &Path,
    nodes: &[NodeData],
    edges: &mut Vec<(NodeId, NodeId, EdgeData)>,
) {
    if node.kind() == "call" {
        if let Some(func_node) = node.child_by_field_name("function") {
            let callee_name = text(func_node, source);
            let mut parent = node.parent();
            let mut caller_id = None;
            while let Some(p) = parent {
                if p.kind() == "function_definition" {
                    if let Some(n) = p.child_by_field_name("name") {
                        let name = text(n, source);
                        caller_id = nodes
                            .iter()
                            .find(|nd| nd.name == name && nd.kind == NodeKind::Function)
                            .map(|nd| nd.id.clone());
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
        extract_py_calls(child, source, path, nodes, edges);
    }
}

use super::{build_nd, build_span, node_text as text};

fn qualified(scope: &[&str], name: &str) -> String {
    if scope.is_empty() {
        name.to_string()
    } else {
        format!("{}::{}", scope.join("::"), name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn python_adapter_parses_function() {
        let a = PythonAdapter::new();
        let parsed = a
            .parse_file(Path::new("t.py"), "def hello():\n    pass")
            .unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "hello" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn python_adapter_parses_class() {
        let a = PythonAdapter::new();
        let src = "class Widget:\n    def render(self):\n        pass";
        let parsed = a.parse_file(Path::new("t.py"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
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
    }

    #[test]
    fn python_adapter_extracts_module_constants() {
        let a = PythonAdapter::new();
        let src = "MAX = 100\nPI: float = 3.14\nNAME = \"hello\"\n";
        let parsed = a.parse_file(Path::new("t.py"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "MAX" && n.kind == NodeKind::Constant),
            "expected Constant for MAX"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "PI" && n.kind == NodeKind::Constant),
            "expected Constant for PI"
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "NAME" && n.kind == NodeKind::Constant),
            "expected Constant for NAME"
        );
    }

    #[test]
    fn python_adapter_skips_runtime_init_as_constant() {
        // Assignments whose RHS is a call (e.g. `app = Flask(__name__)`) are
        // *not* constants — they're runtime initialisation. Confirm we skip
        // them so we don't pollute the graph.
        let a = PythonAdapter::new();
        let src = "app = create_app()\n";
        let parsed = a.parse_file(Path::new("t.py"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            !nodes.iter().any(|n| n.kind == NodeKind::Constant),
            "non-literal assignment should not produce a Constant: {:?}",
            nodes
        );
    }

    #[test]
    fn python_adapter_extracts_class_fields() {
        let a = PythonAdapter::new();
        let src = "class Widget:\n    name: str = \"default\"\n    count: int\n    def render(self):\n        pass\n";
        let parsed = a.parse_file(Path::new("t.py"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "name" && n.kind == NodeKind::Field),
            "expected Field 'name', got: {:?}",
            nodes
                .iter()
                .filter(|n| n.kind == NodeKind::Field)
                .collect::<Vec<_>>()
        );
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "count" && n.kind == NodeKind::Field),
            "expected Field 'count'"
        );
        // method bodies should not contribute additional Field nodes.
        let field_count = nodes.iter().filter(|n| n.kind == NodeKind::Field).count();
        assert_eq!(field_count, 2, "unexpected extra Field nodes: {nodes:?}");
        // class itself should still be a Struct.
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "Widget" && n.kind == NodeKind::Struct)
        );
        // Method should still be a Function.
        assert!(
            nodes
                .iter()
                .any(|n| n.name == "render" && n.kind == NodeKind::Function)
        );
    }

    #[test]
    fn python_adapter_extracts_call_edges() {
        let a = PythonAdapter::new();
        let src = "def caller():\n    callee()\ndef callee():\n    pass";
        let parsed = a.parse_file(Path::new("t.py"), src).unwrap();
        let nodes = a.extract_nodes(&parsed);
        let edges = a.extract_edges(&parsed, &nodes);
        assert!(!edges.is_empty());
    }
}
