//! Backward control-flow predicate extraction for the
//! `preconditions` DSL operator (Magic's path-dependent analysis).
//!
//! Given a byte offset inside a Rust source file (typically a call
//! site span), walk the tree-sitter AST upward from the deepest
//! covering node and collect the text of every enclosing `if`,
//! `match`, `while`, `for`, or `loop` expression. The collected
//! predicates form the path conditions that must have been true for
//! control flow to reach that call site.
//!
//! Output ordering: innermost predicate first, outermost last —
//! matches how a human would read the code (the closest gate is the
//! most recent decision). The renderer joins them with `→` so the
//! reading order is "outer condition → inner condition → call
//! site", which is the actual evaluation order even though we
//! collect inside-out.
//!
//! Why a separate module: the analysis is Rust-specific (tree-
//! sitter Rust grammar node names like `if_expression`,
//! `match_expression`) but the QueryEngine in `dsl::mod` is
//! language-agnostic. Future language adapters can ship their own
//! `predicates` analogues without disturbing the DSL.

use std::path::Path;

use tree_sitter::{Node as TsNode, Parser};

/// One predicate found on the path from a function entry to a
/// target byte position. `kind` is the tree-sitter node kind
/// (`if_expression`, `match_expression`, `while_expression`,
/// `for_expression`, `loop_expression`); `text` is the verbatim
/// predicate slice (for if/while/for: the condition; for match:
/// the scrutinee; for loop: just `"loop"`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Predicate {
    pub kind: &'static str,
    pub text: String,
}

/// Parse `source` and walk upward from `byte_pos` collecting
/// enclosing branch predicates. Returns innermost-first; the
/// iterator's first element is the closest gate, the last is the
/// outermost. Returns an empty Vec when the position has no
/// enclosing branch (e.g. straight-line code at function top
/// level), or when parsing fails (caller should treat absence as
/// "no preconditions known", not as an error).
pub fn extract_predicates(source: &str, byte_pos: usize) -> Vec<Predicate> {
    let mut parser = Parser::new();
    if parser
        .set_language(&tree_sitter_rust::LANGUAGE.into())
        .is_err()
    {
        return Vec::new();
    }
    let Some(tree) = parser.parse(source, None) else {
        return Vec::new();
    };
    let root = tree.root_node();
    let Some(start) = root.descendant_for_byte_range(byte_pos, byte_pos) else {
        return Vec::new();
    };
    walk_up(start, source.as_bytes())
}

/// Convenience wrapper: read `path` from disk and call
/// `extract_predicates`. Errors silently to an empty Vec — same
/// rationale as the in-memory variant: the caller treats absence
/// as "unknown", not as a hard failure that should fail the whole
/// query.
pub fn extract_predicates_at_file(path: &Path, byte_pos: usize) -> Vec<Predicate> {
    let Ok(source) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    extract_predicates(&source, byte_pos)
}

/// For every outgoing Calls (or UnresolvedCall) edge from `caller`,
/// return the (target name, predicates) pairs. Used by the
/// `preconditions` pipeline to surface "to call X you must have
/// passed condition Y" annotations alongside the bare caller list.
/// Reads each call site's source file once per call edge — the
/// hot path is small (typical caller has 1-3 outgoing call edges).
pub fn outgoing_call_predicates(
    graph: &crate::graph::CodeGraph,
    caller: &crate::nodes::NodeId,
) -> Vec<(String, Vec<Predicate>)> {
    use crate::edges::EdgeKind;
    let mut out = Vec::new();
    for (target_id, edge) in graph.get_edges_from(caller) {
        let is_call = matches!(edge.kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_));
        if !is_call {
            continue;
        }
        let target_name = graph
            .get_node(target_id)
            .map(|n| n.name.clone())
            .or_else(|| match &edge.kind {
                EdgeKind::UnresolvedCall(name) => Some(name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| "?".to_string());
        let preds =
            extract_predicates_at_file(&edge.source_span.file, edge.source_span.byte_range.start);
        if !preds.is_empty() {
            out.push((target_name, preds));
        }
    }
    out
}

fn walk_up(start: TsNode<'_>, source: &[u8]) -> Vec<Predicate> {
    let mut out = Vec::new();
    let mut current = Some(start);
    while let Some(node) = current {
        let kind = node.kind();
        match kind {
            "if_expression" => {
                if let Some(cond) = node.child_by_field_name("condition")
                    && let Ok(text) = cond.utf8_text(source)
                {
                    out.push(Predicate {
                        kind: "if_expression",
                        text: text.trim().to_string(),
                    });
                }
            }
            "match_expression" => {
                if let Some(value) = node.child_by_field_name("value")
                    && let Ok(text) = value.utf8_text(source)
                {
                    out.push(Predicate {
                        kind: "match_expression",
                        text: format!("match {}", text.trim()),
                    });
                }
            }
            "while_expression" => {
                if let Some(cond) = node.child_by_field_name("condition")
                    && let Ok(text) = cond.utf8_text(source)
                {
                    out.push(Predicate {
                        kind: "while_expression",
                        text: format!("while {}", text.trim()),
                    });
                }
            }
            "for_expression" => {
                if let (Some(pattern), Some(value)) = (
                    node.child_by_field_name("pattern"),
                    node.child_by_field_name("value"),
                ) && let (Ok(pat), Ok(val)) =
                    (pattern.utf8_text(source), value.utf8_text(source))
                {
                    out.push(Predicate {
                        kind: "for_expression",
                        text: format!("for {} in {}", pat.trim(), val.trim()),
                    });
                }
            }
            "loop_expression" => {
                out.push(Predicate {
                    kind: "loop_expression",
                    text: "loop".to_string(),
                });
            }
            _ => {}
        }
        current = node.parent();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: a call inside a single `if` collects exactly one
    // predicate with the condition text and the right kind.
    #[test]
    fn extract_single_if_normal() {
        let src = r#"
fn run(x: i32) {
    if x > 5 {
        helper();
    }
}
"#;
        let call_pos = src.find("helper").unwrap();
        let preds = extract_predicates(src, call_pos);
        assert_eq!(preds.len(), 1);
        assert_eq!(preds[0].kind, "if_expression");
        assert_eq!(preds[0].text, "x > 5");
    }

    // Normal: nested predicates produce innermost-first ordering.
    #[test]
    fn extract_nested_innermost_first_normal() {
        let src = r#"
fn run(x: i32, y: i32) {
    if x > 0 {
        if y < 10 {
            helper();
        }
    }
}
"#;
        let call_pos = src.find("helper").unwrap();
        let preds = extract_predicates(src, call_pos);
        assert_eq!(preds.len(), 2);
        // Innermost (y < 10) comes first.
        assert_eq!(preds[0].text, "y < 10");
        assert_eq!(preds[1].text, "x > 0");
    }

    // Normal: while / for / match all picked up with the right kinds.
    #[test]
    fn extract_mixed_loop_kinds_normal() {
        let src = r#"
fn run(items: &[i32]) {
    while !items.is_empty() {
        for item in items {
            match item {
                _ => helper(),
            }
        }
    }
}
"#;
        let call_pos = src.find("helper").unwrap();
        let preds = extract_predicates(src, call_pos);
        // Innermost-first: match, for, while.
        assert_eq!(preds[0].kind, "match_expression");
        assert_eq!(preds[1].kind, "for_expression");
        assert_eq!(preds[2].kind, "while_expression");
        assert!(preds[0].text.starts_with("match "));
        assert!(preds[1].text.starts_with("for "));
        assert!(preds[2].text.starts_with("while "));
    }

    // Robust: a call at function top level (no enclosing branch)
    // returns an empty Vec — not an error, just "no preconditions".
    #[test]
    fn extract_top_level_returns_empty_robust() {
        let src = r#"
fn run() {
    helper();
}
"#;
        let call_pos = src.find("helper").unwrap();
        assert!(extract_predicates(src, call_pos).is_empty());
    }

    // Robust: a byte position outside the source range returns an
    // empty Vec (descendant_for_byte_range gracefully None's).
    #[test]
    fn extract_out_of_bounds_returns_empty_robust() {
        let src = "fn run() {}";
        assert!(extract_predicates(src, src.len() + 100).is_empty());
    }

    // Robust: malformed source still yields a tree (tree-sitter is
    // error-tolerant) — the walk should produce whatever predicates
    // it can find without panicking.
    #[test]
    fn extract_malformed_source_does_not_panic_robust() {
        let src = "fn run() { if x > 0 { helper("; // intentionally truncated
        let _ = extract_predicates(src, 25);
    }
}
