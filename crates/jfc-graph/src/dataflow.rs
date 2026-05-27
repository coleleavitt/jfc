//! Per-function dataflow analysis from tree-sitter ASTs.
//!
//! Extracts parameter flows, return expressions, assignments, argument flows
//! to callees, and mutation detection for each function body.
//! The resulting [`FunctionDataflow`] can be stored on [`crate::nodes::NodeData`]
//! and queried via the DSL `dataflow` operator.

use serde::{Deserialize, Serialize};
use tree_sitter::Node as TsNode;

use crate::dataflow_rules::DataflowRules;

// ─── Core Types ──────────────────────────────────────────────────────────────

/// Dataflow summary for a single function.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionDataflow {
    pub params: Vec<DataflowParam>,
    pub returns: Vec<DataflowReturn>,
    pub assignments: Vec<DataflowAssignment>,
    pub arg_flows: Vec<DataflowArgFlow>,
    pub mutations: Vec<DataflowMutation>,
}

/// A function parameter.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataflowParam {
    pub name: String,
    pub position: u32,
    pub type_annotation: Option<String>,
    pub has_default: bool,
}

/// A return expression found in the function body.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataflowReturn {
    pub line: u32,
    pub expression: String,
}

/// A variable assignment or declaration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataflowAssignment {
    pub target: String,
    pub source_kind: AssignSourceKind,
    pub line: u32,
}

/// Classification of the right-hand side of an assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AssignSourceKind {
    Literal,
    Param,
    CallResult,
    FieldAccess,
    Other,
}

/// Records when a function parameter flows directly into a callee argument.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataflowArgFlow {
    pub callee: String,
    pub arg_position: u32,
    pub source_param: Option<String>,
    pub line: u32,
}

/// A detected mutation (method call on an identifier with a mutating method name).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataflowMutation {
    pub target: String,
    pub method: String,
    pub line: u32,
}

impl FunctionDataflow {
    /// Format a compact human-readable summary of the dataflow.
    pub fn format_summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "params={} returns={} assignments={} arg_flows={} mutations={}\n",
            self.params.len(),
            self.returns.len(),
            self.assignments.len(),
            self.arg_flows.len(),
            self.mutations.len(),
        ));
        for p in &self.params {
            let ty = p.type_annotation.as_deref().unwrap_or("?");
            let def = if p.has_default { " [default]" } else { "" };
            out.push_str(&format!(
                "  param[{}]: {} : {}{}\n",
                p.position, p.name, ty, def
            ));
        }
        for r in &self.returns {
            let expr = if r.expression.len() > 60 {
                format!("{}...", &r.expression[..57])
            } else {
                r.expression.clone()
            };
            out.push_str(&format!("  return L{}: {}\n", r.line, expr));
        }
        for a in &self.assignments {
            out.push_str(&format!(
                "  assign L{}: {} <- {:?}\n",
                a.line, a.target, a.source_kind
            ));
        }
        for f in &self.arg_flows {
            let src = f.source_param.as_deref().unwrap_or("?");
            out.push_str(&format!(
                "  flow L{}: {} -> {}[{}]\n",
                f.line, src, f.callee, f.arg_position
            ));
        }
        for m in &self.mutations {
            out.push_str(&format!(
                "  mutate L{}: {}.{}()\n",
                m.line, m.target, m.method
            ));
        }
        out
    }
}

// ─── Extraction ──────────────────────────────────────────────────────────────

/// Extract dataflow information from a function node.
/// Returns None if no rules exist for the language or the node has no body.
pub fn extract_dataflow(
    function_node: TsNode<'_>,
    source: &[u8],
    language_id: &str,
) -> Option<FunctionDataflow> {
    let rules = DataflowRules::for_language(language_id)?;

    // Verify this is a recognized function node kind.
    if !rules.function_nodes.contains(&function_node.kind()) {
        return None;
    }

    // Get function body.
    let body = function_node.child_by_field_name(rules.body_field)?;

    // Extract parameters.
    let params = extract_params(function_node, source, rules);
    let param_names: Vec<&str> = params.iter().map(|p| p.name.as_str()).collect();

    // Extract returns.
    let returns = extract_returns(body, source, rules);

    // Extract assignments.
    let assignments = extract_assignments(body, source, rules, &param_names);

    // Extract arg flows and mutations.
    let mut arg_flows = Vec::new();
    let mut mutations = Vec::new();
    extract_calls_and_mutations(
        body,
        source,
        rules,
        &param_names,
        &mut arg_flows,
        &mut mutations,
    );

    Some(FunctionDataflow {
        params,
        returns,
        assignments,
        arg_flows,
        mutations,
    })
}

// ─── Parameter Extraction ────────────────────────────────────────────────────

fn extract_params(
    function_node: TsNode<'_>,
    source: &[u8],
    rules: &DataflowRules,
) -> Vec<DataflowParam> {
    let mut params = Vec::new();

    let param_list = match function_node.child_by_field_name(rules.param_list_field) {
        Some(pl) => pl,
        None => return params,
    };

    let mut cursor = param_list.walk();
    let mut position: u32 = 0;

    for child in param_list.named_children(&mut cursor) {
        // Skip self parameters for Rust.
        if child.kind() == "self_parameter" {
            continue;
        }

        // For Rust `parameter` nodes: field `pattern` has the name, field `type` has the type.
        if child.kind() == "parameter" {
            let name = child
                .child_by_field_name("pattern")
                .map(|n| node_text(n, source))
                .unwrap_or_default();
            let type_annotation = child
                .child_by_field_name("type")
                .map(|n| node_text(n, source));
            if !name.is_empty() {
                params.push(DataflowParam {
                    name,
                    position,
                    type_annotation,
                    has_default: false,
                });
                position += 1;
            }
            continue;
        }

        // For TypeScript/Python: look for identifier + optional type + optional default.
        // Generic approach: find the identifier child.
        let name = if child.kind() == rules.param_identifier {
            node_text(child, source)
        } else {
            // Try to find a named identifier child or `name` field.
            child
                .child_by_field_name("name")
                .or_else(|| find_child_by_kind(child, rules.param_identifier))
                .map(|n| node_text(n, source))
                .unwrap_or_default()
        };

        if name.is_empty() || name == "self" {
            continue;
        }

        let type_annotation = child
            .child_by_field_name("type")
            .map(|n| node_text(n, source));

        let has_default = child.child_by_field_name("value").is_some()
            || child.child_by_field_name("default_value").is_some();

        params.push(DataflowParam {
            name,
            position,
            type_annotation,
            has_default,
        });
        position += 1;
    }

    params
}

// ─── Return Extraction ───────────────────────────────────────────────────────

fn extract_returns(body: TsNode<'_>, source: &[u8], rules: &DataflowRules) -> Vec<DataflowReturn> {
    let mut returns = Vec::new();
    collect_returns(body, source, rules, &mut returns);
    returns
}

fn collect_returns(
    node: TsNode<'_>,
    source: &[u8],
    rules: &DataflowRules,
    out: &mut Vec<DataflowReturn>,
) {
    if node.kind() == rules.return_node {
        // The returned expression is the first named child (for Rust: return_expression's child).
        let expr = if node.named_child_count() > 0 {
            let child = node.named_child(0).unwrap();
            truncate_expr(&node_text(child, source), 120)
        } else {
            // Return with no value (e.g. bare `return;`).
            String::new()
        };
        out.push(DataflowReturn {
            line: node.start_position().row as u32 + 1,
            expression: expr,
        });
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        // Skip nested functions.
        if child.kind().contains("function") && child.kind() != rules.return_node {
            let is_fn = rules.function_nodes.iter().any(|&k| k == child.kind());
            if is_fn {
                continue;
            }
        }
        collect_returns(child, source, rules, out);
    }
}

// ─── Assignment Extraction ───────────────────────────────────────────────────

fn extract_assignments(
    body: TsNode<'_>,
    source: &[u8],
    rules: &DataflowRules,
    param_names: &[&str],
) -> Vec<DataflowAssignment> {
    let mut assignments = Vec::new();
    collect_assignments(body, source, rules, param_names, &mut assignments);
    assignments
}

fn collect_assignments(
    node: TsNode<'_>,
    source: &[u8],
    rules: &DataflowRules,
    param_names: &[&str],
    out: &mut Vec<DataflowAssignment>,
) {
    if rules.assignment_nodes.contains(&node.kind()) {
        let target = node
            .child_by_field_name(rules.assign_left_field)
            .map(|n| node_text(n, source))
            .unwrap_or_default();
        let source_kind = node
            .child_by_field_name(rules.assign_right_field)
            .map(|rhs| classify_rhs(rhs, source, rules, param_names))
            .unwrap_or(AssignSourceKind::Other);

        if !target.is_empty() {
            out.push(DataflowAssignment {
                target,
                source_kind,
                line: node.start_position().row as u32 + 1,
            });
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        // Skip nested functions.
        if rules.function_nodes.contains(&child.kind()) {
            continue;
        }
        collect_assignments(child, source, rules, param_names, out);
    }
}

fn classify_rhs(
    rhs: TsNode<'_>,
    source: &[u8],
    rules: &DataflowRules,
    param_names: &[&str],
) -> AssignSourceKind {
    let kind = rhs.kind();

    // Check if it's a literal.
    if rules.literal_nodes.contains(&kind) {
        return AssignSourceKind::Literal;
    }

    // Check if it's a call expression.
    if rules.call_nodes.contains(&kind) {
        return AssignSourceKind::CallResult;
    }

    // Check if it's a field/member access.
    if kind == rules.member_node {
        return AssignSourceKind::FieldAccess;
    }

    // Check if it's an identifier that matches a parameter name.
    if kind == rules.identifier_node {
        let text = node_text(rhs, source);
        if param_names.contains(&text.as_str()) {
            return AssignSourceKind::Param;
        }
    }

    AssignSourceKind::Other
}

// ─── Call & Mutation Extraction ──────────────────────────────────────────────

fn extract_calls_and_mutations(
    node: TsNode<'_>,
    source: &[u8],
    rules: &DataflowRules,
    param_names: &[&str],
    arg_flows: &mut Vec<DataflowArgFlow>,
    mutations: &mut Vec<DataflowMutation>,
) {
    let kind = node.kind();

    // For Rust: method_call_expression is a special pattern
    // `receiver.method(args)` — tree-sitter-rust uses `call_expression` for
    // free functions and `call_expression` with a field_expression function
    // for method calls. Actually in tree-sitter-rust, method calls are NOT
    // `call_expression` — they are inlined in the call_expression with a
    // field_expression as the function. Let me handle both patterns:
    //
    // Pattern 1: `call_expression` with `function` field = identifier → free fn call
    // Pattern 2: `call_expression` with `function` field = field_expression → method call

    if rules.call_nodes.contains(&kind) {
        if let Some(func_node) = node.child_by_field_name(rules.call_function_field) {
            let callee_name: String;
            let is_method_call: bool;
            let receiver: Option<String>;

            if func_node.kind() == rules.member_node && !rules.member_node.is_empty() {
                // Method call: func_node is a field_expression/member_expression.
                let obj = func_node
                    .child_by_field_name(rules.member_object_field)
                    .map(|n| node_text(n, source))
                    .unwrap_or_default();
                let method = func_node
                    .child_by_field_name(rules.member_property_field)
                    .map(|n| node_text(n, source))
                    .unwrap_or_default();
                callee_name = method.clone();
                is_method_call = true;
                receiver = Some(obj.clone());

                // Check for mutation.
                if !obj.is_empty()
                    && !method.is_empty()
                    && rules.mutating_methods.contains(&method.as_str())
                {
                    mutations.push(DataflowMutation {
                        target: obj,
                        method,
                        line: node.start_position().row as u32 + 1,
                    });
                }
            } else {
                callee_name = node_text(func_node, source);
                is_method_call = false;
                receiver = None;
            }

            // Extract arg flows.
            if let Some(args_node) = node.child_by_field_name(rules.call_args_field) {
                let mut cursor = args_node.walk();
                let mut arg_pos: u32 = 0;
                for arg in args_node.named_children(&mut cursor) {
                    let arg_text = node_text(arg, source);
                    let source_param = if arg.kind() == rules.identifier_node
                        && param_names.contains(&arg_text.as_str())
                    {
                        Some(arg_text)
                    } else {
                        None
                    };

                    if source_param.is_some() || !is_method_call {
                        // Only record arg_flow when parameter flows into callee.
                        if source_param.is_some() {
                            arg_flows.push(DataflowArgFlow {
                                callee: if is_method_call {
                                    format!(
                                        "{}.{}",
                                        receiver.as_deref().unwrap_or("?"),
                                        &callee_name
                                    )
                                } else {
                                    callee_name.clone()
                                },
                                arg_position: arg_pos,
                                source_param,
                                line: node.start_position().row as u32 + 1,
                            });
                        }
                    }
                    arg_pos += 1;
                }
            }
        }
    }

    // Recurse into children.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        // Skip nested functions.
        if rules.function_nodes.contains(&child.kind()) {
            continue;
        }
        extract_calls_and_mutations(child, source, rules, param_names, arg_flows, mutations);
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn node_text(node: TsNode<'_>, source: &[u8]) -> String {
    node.utf8_text(source).unwrap_or("").to_string()
}

fn find_child_by_kind<'a>(node: TsNode<'a>, kind: &str) -> Option<TsNode<'a>> {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if child.kind() == kind {
            return Some(child);
        }
    }
    None
}

fn truncate_expr(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

// ─── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tree_sitter::Parser;

    fn parse_rust(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .unwrap();
        parser.parse(src, None).unwrap()
    }

    fn find_first_function(tree: &tree_sitter::Tree) -> tree_sitter::Node<'_> {
        fn dfs<'a>(node: tree_sitter::Node<'a>) -> Option<tree_sitter::Node<'a>> {
            if node.kind() == "function_item" {
                return Some(node);
            }
            let mut cursor = node.walk();
            for child in node.named_children(&mut cursor) {
                if let Some(found) = dfs(child) {
                    return Some(found);
                }
            }
            None
        }
        dfs(tree.root_node()).expect("no function_item found in tree")
    }

    #[test]
    fn test_extract_params() {
        let src = r#"
fn add(x: i32, y: i32) -> i32 {
    x + y
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(df.params.len(), 2);
        assert_eq!(df.params[0].name, "x");
        assert_eq!(df.params[0].position, 0);
        assert_eq!(df.params[0].type_annotation.as_deref(), Some("i32"));
        assert!(!df.params[0].has_default);
        assert_eq!(df.params[1].name, "y");
        assert_eq!(df.params[1].position, 1);
        assert_eq!(df.params[1].type_annotation.as_deref(), Some("i32"));
    }

    #[test]
    fn test_extract_params_with_complex_types() {
        let src = r#"
fn process(items: Vec<String>, count: usize) -> bool {
    true
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(df.params.len(), 2);
        assert_eq!(df.params[0].name, "items");
        assert_eq!(df.params[0].type_annotation.as_deref(), Some("Vec<String>"));
        assert_eq!(df.params[1].name, "count");
        assert_eq!(df.params[1].type_annotation.as_deref(), Some("usize"));
    }

    #[test]
    fn test_extract_returns() {
        let src = r#"
fn decide(x: i32) -> i32 {
    if x > 0 {
        return x;
    }
    return 0;
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(df.returns.len(), 2);
        assert_eq!(df.returns[0].expression, "x");
        assert_eq!(df.returns[1].expression, "0");
    }

    #[test]
    fn test_assignment_from_literal() {
        let src = r#"
fn example() {
    let x = 42;
    let y = "hello";
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert!(df.assignments.len() >= 2);
        assert_eq!(df.assignments[0].target, "x");
        assert_eq!(df.assignments[0].source_kind, AssignSourceKind::Literal);
        assert_eq!(df.assignments[1].target, "y");
        assert_eq!(df.assignments[1].source_kind, AssignSourceKind::Literal);
    }

    #[test]
    fn test_assignment_from_param() {
        let src = r#"
fn copy_val(input: i32) {
    let y = input;
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(df.assignments.len(), 1);
        assert_eq!(df.assignments[0].target, "y");
        assert_eq!(df.assignments[0].source_kind, AssignSourceKind::Param);
    }

    #[test]
    fn test_assignment_from_call() {
        let src = r#"
fn example() {
    let z = foo();
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(df.assignments.len(), 1);
        assert_eq!(df.assignments[0].target, "z");
        assert_eq!(df.assignments[0].source_kind, AssignSourceKind::CallResult);
    }

    #[test]
    fn test_arg_flow() {
        let src = r#"
fn caller(x: i32, y: String) {
    bar(x);
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(df.arg_flows.len(), 1);
        assert_eq!(df.arg_flows[0].callee, "bar");
        assert_eq!(df.arg_flows[0].arg_position, 0);
        assert_eq!(df.arg_flows[0].source_param.as_deref(), Some("x"));
    }

    #[test]
    fn test_mutation_detection() {
        let src = r#"
fn mutator() {
    let mut vec = Vec::new();
    vec.push(item);
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert!(!df.mutations.is_empty());
        assert_eq!(df.mutations[0].target, "vec");
        assert_eq!(df.mutations[0].method, "push");
    }

    #[test]
    fn test_no_dataflow_for_unknown_language() {
        let src = r#"
fn example() {
    let x = 42;
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let result = extract_dataflow(func, src.as_bytes(), "unknown_lang");
        assert!(result.is_none());
    }

    #[test]
    fn test_field_access_assignment() {
        let src = r#"
fn example(obj: Foo) {
    let val = obj.field;
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(df.assignments.len(), 1);
        assert_eq!(df.assignments[0].target, "val");
        assert_eq!(df.assignments[0].source_kind, AssignSourceKind::FieldAccess);
    }

    #[test]
    fn test_multiple_arg_flows() {
        let src = r#"
fn caller(a: i32, b: String) {
    foo(a, b);
}
"#;
        let tree = parse_rust(src);
        let func = find_first_function(&tree);
        let df = extract_dataflow(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(df.arg_flows.len(), 2);
        assert_eq!(df.arg_flows[0].callee, "foo");
        assert_eq!(df.arg_flows[0].arg_position, 0);
        assert_eq!(df.arg_flows[0].source_param.as_deref(), Some("a"));
        assert_eq!(df.arg_flows[1].callee, "foo");
        assert_eq!(df.arg_flows[1].arg_position, 1);
        assert_eq!(df.arg_flows[1].source_param.as_deref(), Some("b"));
    }
}
