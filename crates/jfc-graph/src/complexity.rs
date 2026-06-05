//! Per-function complexity metrics computed from tree-sitter ASTs.
//!
//! Provides cognitive complexity, cyclomatic complexity, max nesting depth,
//! Halstead metrics, LOC metrics, and maintainability index.

use serde::{Deserialize, Serialize};
use tree_sitter::Node as TsNode;

use crate::complexity_rules::LangRules;

/// Halstead software science metrics derived from operator/operand counts.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HalsteadMetrics {
    /// Unique operators (η₁).
    pub unique_operators: u32,
    /// Unique operands (η₂).
    pub unique_operands: u32,
    /// Total operators (N₁).
    pub total_operators: u32,
    /// Total operands (N₂).
    pub total_operands: u32,
    /// Vocabulary: η = η₁ + η₂.
    pub vocabulary: u32,
    /// Program length: N = N₁ + N₂.
    pub length: u32,
    /// Volume: V = N × log₂(η).
    pub volume: f64,
    /// Difficulty: D = (η₁ / 2) × (N₂ / η₂).
    pub difficulty: f64,
    /// Effort: E = D × V.
    pub effort: f64,
    /// Estimated bugs: B = V / 3000.
    pub bugs: f64,
}

/// Lines-of-code metrics for a function body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LocMetrics {
    /// Total lines (including blank lines).
    pub total: u32,
    /// Source lines (non-blank, non-comment).
    pub source: u32,
    /// Comment lines (lines with only comments or with leading comments).
    pub comment: u32,
}

/// Aggregate complexity metrics for a single function.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComplexityMetrics {
    /// Cognitive complexity — nesting-aware increments for breaks in linear flow.
    pub cognitive: u32,
    /// Cyclomatic complexity — linearly independent paths through the function.
    pub cyclomatic: u32,
    /// Maximum nesting depth reached in the function body.
    pub max_nesting: u32,
    /// Halstead software science metrics (None if function has no operators/operands).
    pub halstead: Option<HalsteadMetrics>,
    /// Lines-of-code breakdown.
    pub loc: Option<LocMetrics>,
    /// Maintainability Index: 171 - 5.2×ln(V) - 0.23×G - 16.2×ln(LOC).
    /// None if Halstead or LOC is unavailable.
    pub maintainability_index: Option<f64>,
}

/// Compute all complexity metrics for a function body node.
///
/// # Arguments
/// - `function_node`: The tree-sitter node for the entire function (including signature).
/// - `source`: The full source text of the file.
/// - `language_id`: Language identifier ("rust", "typescript", "python", "go").
///
/// Returns `None` if no rules exist for the given language or the node has no body.
pub fn compute_complexity(
    function_node: TsNode<'_>,
    source: &[u8],
    language_id: &str,
) -> Option<ComplexityMetrics> {
    let rules = LangRules::for_language(language_id)?;

    // Find the function body node. The body field name varies by language.
    let body = find_function_body(function_node, &rules);
    let body_node = body.unwrap_or(function_node);

    let cognitive = compute_cognitive(body_node, source, &rules);
    let cyclomatic = compute_cyclomatic(body_node, source, &rules);
    let max_nesting = compute_max_nesting(body_node, &rules);
    let halstead = compute_halstead(body_node, source, &rules);
    let loc = compute_loc(function_node, source);

    let maintainability_index = match (&halstead, &loc) {
        (Some(h), Some(l)) if h.volume > 0.0 && l.source > 0 => {
            let v = h.volume;
            let g = cyclomatic as f64;
            let loc_val = l.source as f64;
            let mi = 171.0 - 5.2 * v.ln() - 0.23 * g - 16.2 * loc_val.ln();
            Some(mi.max(0.0))
        }
        _ => None,
    };

    Some(ComplexityMetrics {
        cognitive,
        cyclomatic,
        max_nesting,
        halstead,
        loc,
        maintainability_index,
    })
}

/// Locate the function body according to language rules.
fn find_function_body<'a>(node: TsNode<'a>, rules: &LangRules) -> Option<TsNode<'a>> {
    for field in rules.body_field_names {
        if let Some(body) = node.child_by_field_name(field) {
            return Some(body);
        }
    }
    None
}

// ─── Cognitive Complexity ────────────────────────────────────────────────────

/// Compute cognitive complexity using the SonarSource model:
/// - +1 for each break in linear flow (if, for, while, match/switch, catch, goto)
/// - +nesting for each nesting-inducing structure when inside a nested context
/// - No increment for `else` (it's a continuation of the if)
/// - Logical operators (&&, ||) get +1 each (but sequences of the same op are free)
fn compute_cognitive(body: TsNode<'_>, source: &[u8], rules: &LangRules) -> u32 {
    let mut score: u32 = 0;
    walk_cognitive(body, source, rules, 0, &mut score);
    score
}

fn walk_cognitive(
    node: TsNode<'_>,
    source: &[u8],
    rules: &LangRules,
    nesting: u32,
    score: &mut u32,
) {
    let kind = node.kind();

    // Check if this node is an increment-inducing structure.
    let is_branch = rules.branch_nodes.contains(&kind);
    let is_nesting = rules.nesting_nodes.contains(&kind);

    if is_branch {
        // +1 for the break in flow, +nesting for structural complexity
        *score += 1 + nesting;
    }

    // Check for logical operators (&&, ||, ??) — +1 each
    if rules.logical_op_nodes.contains(&kind) {
        if let Some(op_node) = node.child_by_field_name("operator") {
            let op_text = &source[op_node.byte_range()];
            if rules
                .logical_operators
                .iter()
                .any(|op| op.as_bytes() == op_text)
            {
                *score += 1;
            }
        }
    }

    // Recurse into children with updated nesting level.
    let new_nesting = if is_nesting { nesting + 1 } else { nesting };

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_cognitive(child, source, rules, new_nesting, score);
    }
}

// ─── Cyclomatic Complexity ──────────────────────────────────────────────────

/// Cyclomatic complexity: start at 1, +1 for each decision point.
fn compute_cyclomatic(body: TsNode<'_>, source: &[u8], rules: &LangRules) -> u32 {
    let mut count: u32 = 1;
    walk_cyclomatic(body, source, rules, &mut count);
    count
}

fn walk_cyclomatic(node: TsNode<'_>, source: &[u8], rules: &LangRules, count: &mut u32) {
    let kind = node.kind();

    // Branch nodes add a decision point.
    if rules.branch_nodes.contains(&kind) {
        *count += 1;
    }

    // Case nodes (match arms, switch cases) add decision points.
    if rules.case_nodes.contains(&kind) {
        *count += 1;
    }

    // Logical operators (&&, ||) add a decision point.
    if rules.logical_op_nodes.contains(&kind) {
        if let Some(op_node) = node.child_by_field_name("operator") {
            let op_text = &source[op_node.byte_range()];
            if rules
                .logical_operators
                .iter()
                .any(|op| op.as_bytes() == op_text)
            {
                *count += 1;
            }
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_cyclomatic(child, source, rules, count);
    }
}

// ─── Max Nesting Depth ──────────────────────────────────────────────────────

/// Compute maximum nesting depth of control-flow structures.
fn compute_max_nesting(body: TsNode<'_>, rules: &LangRules) -> u32 {
    let mut max_depth: u32 = 0;
    walk_nesting(body, rules, 0, &mut max_depth);
    max_depth
}

fn walk_nesting(node: TsNode<'_>, rules: &LangRules, current: u32, max_depth: &mut u32) {
    let kind = node.kind();
    let new_depth = if rules.nesting_nodes.contains(&kind) {
        let d = current + 1;
        if d > *max_depth {
            *max_depth = d;
        }
        d
    } else {
        current
    };

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_nesting(child, rules, new_depth, max_depth);
    }
}

// ─── Halstead Metrics ───────────────────────────────────────────────────────

/// Compute Halstead metrics by walking the AST and counting operators/operands.
fn compute_halstead(body: TsNode<'_>, source: &[u8], rules: &LangRules) -> Option<HalsteadMetrics> {
    use std::collections::HashSet;

    let mut unique_ops: HashSet<String> = HashSet::new();
    let mut unique_opnds: HashSet<String> = HashSet::new();
    let mut total_ops: u32 = 0;
    let mut total_opnds: u32 = 0;

    walk_halstead(
        body,
        source,
        rules,
        &mut unique_ops,
        &mut unique_opnds,
        &mut total_ops,
        &mut total_opnds,
    );

    if unique_ops.is_empty() && unique_opnds.is_empty() {
        return None;
    }

    let n1 = unique_ops.len() as u32;
    let n2 = unique_opnds.len() as u32;
    let big_n1 = total_ops;
    let big_n2 = total_opnds;
    let vocabulary = n1 + n2;
    let length = big_n1 + big_n2;
    let volume = if vocabulary > 0 {
        length as f64 * (vocabulary as f64).log2()
    } else {
        0.0
    };
    let difficulty = if n2 > 0 {
        (n1 as f64 / 2.0) * (big_n2 as f64 / n2 as f64)
    } else {
        0.0
    };
    let effort = difficulty * volume;
    let bugs = volume / 3000.0;

    Some(HalsteadMetrics {
        unique_operators: n1,
        unique_operands: n2,
        total_operators: big_n1,
        total_operands: big_n2,
        vocabulary,
        length,
        volume,
        difficulty,
        effort,
        bugs,
    })
}

fn walk_halstead(
    node: TsNode<'_>,
    source: &[u8],
    rules: &LangRules,
    unique_ops: &mut std::collections::HashSet<String>,
    unique_opnds: &mut std::collections::HashSet<String>,
    total_ops: &mut u32,
    total_opnds: &mut u32,
) {
    let kind = node.kind();
    let text_lazy = || {
        std::str::from_utf8(&source[node.byte_range()])
            .unwrap_or("")
            .to_string()
    };

    if rules.operator_nodes.contains(&kind) {
        *total_ops += 1;
        unique_ops.insert(text_lazy());
    } else if rules.operand_nodes.contains(&kind) {
        *total_opnds += 1;
        unique_opnds.insert(text_lazy());
    }

    // For binary/unary expressions, extract the operator token itself.
    if rules.operator_container_nodes.contains(&kind) {
        if let Some(op_node) = node.child_by_field_name("operator") {
            let op_text = std::str::from_utf8(&source[op_node.byte_range()])
                .unwrap_or("")
                .to_string();
            *total_ops += 1;
            unique_ops.insert(op_text);
        }
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        walk_halstead(
            child,
            source,
            rules,
            unique_ops,
            unique_opnds,
            total_ops,
            total_opnds,
        );
    }
}

// ─── LOC Metrics ────────────────────────────────────────────────────────────

/// Compute LOC metrics from the function's source text.
fn compute_loc(function_node: TsNode<'_>, source: &[u8]) -> Option<LocMetrics> {
    let text = std::str::from_utf8(&source[function_node.byte_range()]).ok()?;

    let mut total: u32 = 0;
    let mut comment: u32 = 0;
    let mut blank: u32 = 0;

    for line in text.lines() {
        total += 1;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank += 1;
        } else if trimmed.starts_with("//")
            || trimmed.starts_with("/*")
            || trimmed.starts_with('*')
            || trimmed.starts_with('#')
        {
            comment += 1;
        }
    }

    // Handle last line without newline
    if total == 0 && !text.is_empty() {
        total = 1;
    }

    let source_lines = total.saturating_sub(blank + comment);

    Some(LocMetrics {
        total,
        source: source_lines,
        comment,
    })
}

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

    fn parse_typescript(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into())
            .unwrap();
        parser.parse(src, None).unwrap()
    }

    fn parse_python(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_python::LANGUAGE.into())
            .unwrap();
        parser.parse(src, None).unwrap()
    }

    fn parse_go(src: &str) -> tree_sitter::Tree {
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_go::LANGUAGE.into())
            .unwrap();
        parser.parse(src, None).unwrap()
    }

    /// Find the first function node in the tree.
    fn first_function<'a>(tree: &'a tree_sitter::Tree, lang: &str) -> tree_sitter::Node<'a> {
        let root = tree.root_node();
        find_function_in(root, lang).expect("no function found in test source")
    }

    fn find_function_in<'a>(node: TsNode<'a>, lang: &str) -> Option<TsNode<'a>> {
        let fn_kinds: &[&str] = match lang {
            "rust" => &["function_item"],
            "typescript" => &["function_declaration"],
            "python" => &["function_definition"],
            "go" => &["function_declaration"],
            _ => &[],
        };

        if fn_kinds.contains(&node.kind()) {
            return Some(node);
        }

        let mut cursor = node.walk();
        for child in node.named_children(&mut cursor) {
            if let Some(found) = find_function_in(child, lang) {
                return Some(found);
            }
        }
        None
    }

    // ─── Test: Empty function ────────────────────────────────────────────

    #[test]
    fn test_empty_function_rust() {
        let src = "fn empty() {}";
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(metrics.cognitive, 0);
        assert_eq!(metrics.cyclomatic, 1);
        assert_eq!(metrics.max_nesting, 0);
    }

    // ─── Test: Simple if ─────────────────────────────────────────────────

    #[test]
    fn test_simple_if_rust() {
        let src = r#"
fn check(x: i32) -> bool {
    if x > 0 {
        return true;
    }
    false
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        // cognitive: +1 for if (nesting=0 so just +1)
        assert_eq!(metrics.cognitive, 1);
        // cyclomatic: 1 + 1 (if) = 2
        assert_eq!(metrics.cyclomatic, 2);
        assert_eq!(metrics.max_nesting, 1);
    }

    // ─── Test: Nested if ─────────────────────────────────────────────────

    #[test]
    fn test_nested_if_rust() {
        let src = r#"
fn nested(x: i32, y: i32) -> i32 {
    if x > 0 {
        if y > 0 {
            return x + y;
        }
    }
    0
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        // cognitive: +1 for outer if (nesting=0), +1+1 for inner if (nesting=1) = 3
        assert_eq!(metrics.cognitive, 3);
        // cyclomatic: 1 + 2 (two ifs) = 3
        assert_eq!(metrics.cyclomatic, 3);
        assert_eq!(metrics.max_nesting, 2);
    }

    // ─── Test: Match expression ──────────────────────────────────────────

    #[test]
    fn test_match_expression_rust() {
        let src = r#"
fn classify(x: i32) -> &'static str {
    match x {
        0 => "zero",
        1 => "one",
        2 => "two",
        _ => "other",
    }
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        // cognitive: +1 for match (nesting=0)
        assert_eq!(metrics.cognitive, 1);
        // cyclomatic: 1 + 1 (match) + 4 (arms) = 6
        assert!(metrics.cyclomatic >= 5);
        assert_eq!(metrics.max_nesting, 1);
    }

    // ─── Test: For loop ──────────────────────────────────────────────────

    #[test]
    fn test_for_loop_rust() {
        let src = r#"
fn sum(items: &[i32]) -> i32 {
    let mut total = 0;
    for item in items {
        total += item;
    }
    total
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        // cognitive: +1 for for loop
        assert_eq!(metrics.cognitive, 1);
        // cyclomatic: 1 + 1 = 2
        assert_eq!(metrics.cyclomatic, 2);
        assert_eq!(metrics.max_nesting, 1);
    }

    // ─── Test: Logical operators ─────────────────────────────────────────

    #[test]
    fn test_logical_operators_rust() {
        let src = r#"
fn validate(x: i32, y: i32) -> bool {
    x > 0 && y > 0 || x < -10
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        // cognitive: +1 per logical operator (&&, ||) = 2
        assert!(metrics.cognitive >= 2);
        // cyclomatic: 1 + 2 logical ops = 3
        assert!(metrics.cyclomatic >= 3);
    }

    // ─── Test: Deeply nested ─────────────────────────────────────────────

    #[test]
    fn test_deeply_nested_rust() {
        let src = r#"
fn deep(a: bool, b: bool, c: bool, d: bool) -> i32 {
    if a {
        if b {
            if c {
                if d {
                    return 42;
                }
            }
        }
    }
    0
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(metrics.max_nesting, 4);
        // cognitive: +1 + (1+1) + (1+2) + (1+3) = 1+2+3+4 = 10
        assert_eq!(metrics.cognitive, 10);
        // cyclomatic: 1 + 4 = 5
        assert_eq!(metrics.cyclomatic, 5);
    }

    // ─── Test: While loop ────────────────────────────────────────────────

    #[test]
    fn test_while_loop_rust() {
        let src = r#"
fn count_down(mut n: i32) -> i32 {
    while n > 0 {
        n -= 1;
    }
    n
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        assert_eq!(metrics.cognitive, 1);
        assert_eq!(metrics.cyclomatic, 2);
        assert_eq!(metrics.max_nesting, 1);
    }

    // ─── Test: TypeScript function ───────────────────────────────────────

    #[test]
    fn test_typescript_if_else() {
        let src = r#"
function classify(x: number): string {
    if (x > 0) {
        return "positive";
    } else if (x < 0) {
        return "negative";
    } else {
        return "zero";
    }
}
"#;
        let tree = parse_typescript(src);
        let func = first_function(&tree, "typescript");
        let metrics = compute_complexity(func, src.as_bytes(), "typescript").unwrap();

        // Two if statements → cognitive at least 2
        assert!(metrics.cognitive >= 2);
        // cyclomatic: 1 + 2 = 3
        assert!(metrics.cyclomatic >= 3);
    }

    // ─── Test: Python function ───────────────────────────────────────────

    #[test]
    fn test_python_for_with_if() {
        let src = r#"
def count_positive(items):
    count = 0
    for item in items:
        if item > 0:
            count += 1
    return count
"#;
        let tree = parse_python(src);
        let func = first_function(&tree, "python");
        let metrics = compute_complexity(func, src.as_bytes(), "python").unwrap();

        // for (+1) + if at nesting=1 (+1+1) = 3
        assert!(metrics.cognitive >= 3);
        // cyclomatic: 1 + 1 + 1 = 3
        assert!(metrics.cyclomatic >= 3);
        assert!(metrics.max_nesting >= 2);
    }

    // ─── Test: Go function ───────────────────────────────────────────────

    #[test]
    fn test_go_switch() {
        let src = r#"
package main

func classify(x int) string {
    switch {
    case x > 0:
        return "positive"
    case x < 0:
        return "negative"
    default:
        return "zero"
    }
}
"#;
        let tree = parse_go(src);
        let func = first_function(&tree, "go");
        let metrics = compute_complexity(func, src.as_bytes(), "go").unwrap();

        assert!(metrics.cyclomatic >= 3);
        assert!(metrics.max_nesting >= 1);
    }

    // ─── Test: LOC metrics ───────────────────────────────────────────────

    #[test]
    fn test_loc_metrics_rust() {
        let src = r#"
fn documented(x: i32) -> i32 {
    // This is a comment
    let y = x + 1;

    // Another comment
    y * 2
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();
        let loc = metrics.loc.unwrap();

        assert!(loc.total >= 7);
        assert!(loc.comment >= 2);
        assert!(loc.source >= 3);
    }

    // ─── Test: Halstead metrics present ──────────────────────────────────

    #[test]
    fn test_halstead_present_rust() {
        let src = r#"
fn compute(a: i32, b: i32) -> i32 {
    let x = a + b;
    let y = x * 2;
    x + y
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        let h = metrics.halstead.unwrap();
        assert!(h.unique_operators > 0);
        assert!(h.unique_operands > 0);
        assert!(h.volume > 0.0);
        assert!(h.effort > 0.0);
    }

    // ─── Test: Maintainability index ─────────────────────────────────────

    #[test]
    fn test_maintainability_index() {
        let src = r#"
fn simple(x: i32) -> i32 {
    x + 1
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        // Simple function should have a high MI
        if let Some(mi) = metrics.maintainability_index {
            assert!(mi > 50.0, "expected high MI for simple fn, got {mi}");
        }
    }

    // ─── Test: Many match arms ───────────────────────────────────────────

    #[test]
    fn test_many_match_arms_rust() {
        let src = r#"
fn to_str(n: i32) -> &'static str {
    match n {
        0 => "zero",
        1 => "one",
        2 => "two",
        3 => "three",
        4 => "four",
        5 => "five",
        6 => "six",
        7 => "seven",
        8 => "eight",
        _ => "other",
    }
}
"#;
        let tree = parse_rust(src);
        let func = first_function(&tree, "rust");
        let metrics = compute_complexity(func, src.as_bytes(), "rust").unwrap();

        // High cyclomatic due to many arms
        assert!(
            metrics.cyclomatic >= 10,
            "expected high cyclomatic for many match arms, got {}",
            metrics.cyclomatic
        );
        // Cognitive is still low — just one match
        assert_eq!(metrics.cognitive, 1);
    }
}
