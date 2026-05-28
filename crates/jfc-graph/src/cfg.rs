//! Per-function control flow graph (CFG) construction from tree-sitter ASTs.
//!
//! Builds a basic-block graph with typed edges for each function body.
//! The resulting [`FunctionCfg`] can be stored on [`crate::nodes::NodeData`]
//! and queried via the DSL `cfg` operator.

use serde::{Deserialize, Serialize};
use tree_sitter::Node as TsNode;

use crate::cfg_rules::CfgRules;

// ─── Core Types ──────────────────────────────────────────────────────────────

/// A control flow graph for a single function.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FunctionCfg {
    pub blocks: Vec<CfgBlock>,
    pub edges: Vec<CfgEdge>,
}

/// A basic block in the CFG.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CfgBlock {
    pub id: u32,
    pub label: String,
    pub start_line: u32,
    pub end_line: u32,
    pub kind: CfgBlockKind,
}

/// Classification of a basic block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CfgBlockKind {
    Entry,
    Exit,
    Normal,
    Branch,
    Loop,
    Exception,
}

/// An edge between two basic blocks.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CfgEdge {
    pub from: u32,
    pub to: u32,
    pub kind: CfgEdgeKind,
}

/// Classification of a CFG edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CfgEdgeKind {
    Normal,
    BranchTrue,
    BranchFalse,
    LoopBack,
    Exception,
    Break,
    Continue,
    Return,
}

// ─── Builder ─────────────────────────────────────────────────────────────────

/// Internal state for incremental CFG construction.
struct CfgBuilder {
    blocks: Vec<CfgBlock>,
    edges: Vec<CfgEdge>,
    next_id: u32,
    /// Stack of loop header block IDs for break/continue resolution.
    loop_stack: Vec<LoopContext>,
}

struct LoopContext {
    header_id: u32,
    /// Block ID that follows the loop (filled in after loop body is processed).
    after_id: Option<u32>,
}

impl CfgBuilder {
    fn new() -> Self {
        Self {
            blocks: Vec::new(),
            edges: Vec::new(),
            next_id: 0,
            loop_stack: Vec::new(),
        }
    }

    fn new_block(
        &mut self,
        label: impl Into<String>,
        kind: CfgBlockKind,
        start_line: u32,
        end_line: u32,
    ) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        self.blocks.push(CfgBlock {
            id,
            label: label.into(),
            start_line,
            end_line,
            kind,
        });
        id
    }

    fn add_edge(&mut self, from: u32, to: u32, kind: CfgEdgeKind) {
        self.edges.push(CfgEdge { from, to, kind });
    }

    fn build(self) -> FunctionCfg {
        FunctionCfg {
            blocks: self.blocks,
            edges: self.edges,
        }
    }
}

// ─── Public API ──────────────────────────────────────────────────────────────

/// Build a CFG for a function body node.
///
/// Returns `None` if no rules exist for the given language or the node has no body.
pub fn build_cfg(
    function_node: TsNode<'_>,
    source: &[u8],
    language_id: &str,
) -> Option<FunctionCfg> {
    let rules = CfgRules::for_language(language_id)?;

    // Verify this is a function node.
    if !rules.function_nodes.contains(&function_node.kind()) {
        return None;
    }

    // Find the function body.
    let body = function_node.child_by_field_name(rules.body_field)?;

    let mut builder = CfgBuilder::new();

    // Create ENTRY and EXIT blocks.
    let fn_start = function_node.start_position().row as u32 + 1;
    let fn_end = function_node.end_position().row as u32 + 1;

    let entry_id = builder.new_block("ENTRY", CfgBlockKind::Entry, fn_start, fn_start);
    let exit_id = builder.new_block("EXIT", CfgBlockKind::Exit, fn_end, fn_end);

    // Walk the body and build blocks.
    let last_block = walk_block(&mut builder, body, source, rules, entry_id, exit_id);

    // Connect last block to EXIT if it hasn't already been connected.
    if let Some(last) = last_block {
        if !builder
            .edges
            .iter()
            .any(|e| e.from == last && e.to == exit_id)
        {
            builder.add_edge(last, exit_id, CfgEdgeKind::Normal);
        }
    }

    Some(builder.build())
}

/// Walk a block node (compound statement / function body) and return the ID
/// of the last block that was active when the block ended, or None if all
/// paths ended with a jump (return/break/continue).
#[allow(clippy::only_used_in_recursion)]
fn walk_block(
    builder: &mut CfgBuilder,
    block_node: TsNode<'_>,
    source: &[u8],
    rules: &CfgRules,
    predecessor: u32,
    exit_id: u32,
) -> Option<u32> {
    let mut current = predecessor;
    let mut cursor = block_node.walk();
    let mut terminated = false;

    for child in block_node.named_children(&mut cursor) {
        if terminated {
            // Dead code after a return/break/continue — skip.
            break;
        }

        // Unwrap expression_statement to get the actual expression.
        // In Rust's tree-sitter grammar, control flow expressions like
        // if_expression, for_expression, etc. are wrapped in expression_statement
        // when used at the statement level.
        let effective = if child.kind() == "expression_statement" {
            child.named_child(0).unwrap_or(child)
        } else {
            child
        };

        let kind = effective.kind();
        let start = effective.start_position().row as u32 + 1;
        let end = effective.end_position().row as u32 + 1;

        if rules.if_nodes.contains(&kind) {
            // ─── If/Else ─────────────────────────────────────────────
            let branch_id = builder.new_block("if", CfgBlockKind::Branch, start, end);
            builder.add_edge(current, branch_id, CfgEdgeKind::Normal);

            // True branch: find the body/consequence.
            let true_body = effective
                .child_by_field_name("consequence")
                .or_else(|| effective.child_by_field_name("body"));

            let after_id = builder.new_block("after_if", CfgBlockKind::Normal, end, end);

            let true_end = if let Some(tbody) = true_body {
                walk_block(builder, tbody, source, rules, branch_id, exit_id)
            } else {
                Some(branch_id)
            };

            // Connect true branch with BranchTrue edge from branch to first stmt.
            // The walk_block already created its own flow from branch_id, but we
            // need to re-label the edge. Instead, we trust that walk_block's first
            // edge from branch_id is the true path. We'll add the edge type at
            // the branch→true_body level.
            // Actually: let's just mark the edge we added (current→branch) as Normal
            // and add true/false from branch.
            if let Some(te) = true_end {
                if !builder
                    .edges
                    .iter()
                    .any(|e| e.from == te && e.to == exit_id)
                {
                    builder.add_edge(te, after_id, CfgEdgeKind::Normal);
                }
            }

            // False branch: else clause.
            let else_child = rules.else_node.and_then(|en| {
                let mut c2 = effective.walk();
                effective.named_children(&mut c2).find(|n| n.kind() == en)
            });

            if let Some(else_node) = else_child {
                // Create an else block and add BranchFalse edge to it.
                let else_start = else_node.start_position().row as u32 + 1;
                let else_end = else_node.end_position().row as u32 + 1;
                let else_block_id =
                    builder.new_block("else", CfgBlockKind::Normal, else_start, else_end);
                builder.add_edge(branch_id, else_block_id, CfgEdgeKind::BranchFalse);
                let false_end =
                    walk_block(builder, else_node, source, rules, else_block_id, exit_id);
                if let Some(fe) = false_end {
                    if !builder
                        .edges
                        .iter()
                        .any(|e| e.from == fe && e.to == exit_id)
                    {
                        builder.add_edge(fe, after_id, CfgEdgeKind::Normal);
                    }
                }
            } else {
                // No else — false edge goes directly to after.
                builder.add_edge(branch_id, after_id, CfgEdgeKind::BranchFalse);
            }

            current = after_id;
        } else if rules.for_nodes.contains(&kind) || rules.while_nodes.contains(&kind) {
            // ─── For/While Loop ──────────────────────────────────────
            let header_id = builder.new_block("loop_header", CfgBlockKind::Loop, start, start);
            builder.add_edge(current, header_id, CfgEdgeKind::Normal);

            let after_id = builder.new_block("after_loop", CfgBlockKind::Normal, end, end);

            // Push loop context.
            builder.loop_stack.push(LoopContext {
                header_id,
                after_id: Some(after_id),
            });

            // False edge: loop condition fails → after.
            builder.add_edge(header_id, after_id, CfgEdgeKind::BranchFalse);

            // True edge: into body.
            let loop_body = effective.child_by_field_name("body");
            if let Some(body) = loop_body {
                let body_end = walk_block(builder, body, source, rules, header_id, exit_id);
                if let Some(be) = body_end {
                    builder.add_edge(be, header_id, CfgEdgeKind::LoopBack);
                }
            }

            builder.loop_stack.pop();
            current = after_id;
        } else if rules.loop_node == Some(kind) {
            // ─── Infinite Loop (Rust `loop`) ─────────────────────────
            let header_id = builder.new_block("loop", CfgBlockKind::Loop, start, start);
            builder.add_edge(current, header_id, CfgEdgeKind::Normal);

            let after_id = builder.new_block("after_loop", CfgBlockKind::Normal, end, end);

            builder.loop_stack.push(LoopContext {
                header_id,
                after_id: Some(after_id),
            });

            let loop_body = effective.child_by_field_name("body");
            if let Some(body) = loop_body {
                let body_end = walk_block(builder, body, source, rules, header_id, exit_id);
                if let Some(be) = body_end {
                    builder.add_edge(be, header_id, CfgEdgeKind::LoopBack);
                }
            }

            builder.loop_stack.pop();
            current = after_id;
        } else if rules.switch_nodes.contains(&kind) {
            // ─── Match/Switch ────────────────────────────────────────
            let branch_id = builder.new_block("match", CfgBlockKind::Branch, start, end);
            builder.add_edge(current, branch_id, CfgEdgeKind::Normal);

            let after_id = builder.new_block("after_match", CfgBlockKind::Normal, end, end);

            // Find case/arm children. They may be direct children of the node
            // or nested inside a body/match_block.
            let search_node = effective.child_by_field_name("body").unwrap_or(effective);
            let mut case_cursor = search_node.walk();
            let mut found_case = false;
            for case_child in search_node.named_children(&mut case_cursor) {
                if rules.case_nodes.contains(&case_child.kind()) {
                    found_case = true;
                    let arm_end =
                        walk_block(builder, case_child, source, rules, branch_id, exit_id);
                    if let Some(ae) = arm_end {
                        builder.add_edge(ae, after_id, CfgEdgeKind::Normal);
                    }
                }
            }

            if !found_case {
                // Degenerate: no arms found, connect directly.
                builder.add_edge(branch_id, after_id, CfgEdgeKind::Normal);
            }

            current = after_id;
        } else if rules.try_nodes.contains(&kind) {
            // ─── Try/Catch ───────────────────────────────────────────
            let try_block_id = builder.new_block("try", CfgBlockKind::Normal, start, end);
            builder.add_edge(current, try_block_id, CfgEdgeKind::Normal);

            let after_id = builder.new_block("after_try", CfgBlockKind::Normal, end, end);

            // Walk try body.
            let try_body = effective.child_by_field_name("body");
            let try_end = if let Some(tbody) = try_body {
                walk_block(builder, tbody, source, rules, try_block_id, exit_id)
            } else {
                Some(try_block_id)
            };
            if let Some(te) = try_end {
                builder.add_edge(te, after_id, CfgEdgeKind::Normal);
            }

            // Find catch clause.
            if let Some(catch_kind) = rules.catch_node {
                let mut try_cursor = effective.walk();
                for try_child in effective.named_children(&mut try_cursor) {
                    if try_child.kind() == catch_kind {
                        let catch_id = builder.new_block(
                            "catch",
                            CfgBlockKind::Exception,
                            try_child.start_position().row as u32 + 1,
                            try_child.end_position().row as u32 + 1,
                        );
                        builder.add_edge(try_block_id, catch_id, CfgEdgeKind::Exception);
                        let catch_end =
                            walk_block(builder, try_child, source, rules, catch_id, exit_id);
                        if let Some(ce) = catch_end {
                            builder.add_edge(ce, after_id, CfgEdgeKind::Normal);
                        }
                    }
                }
            }

            current = after_id;
        } else if rules.return_node == Some(kind) {
            // ─── Return ──────────────────────────────────────────────
            let ret_id = builder.new_block("return", CfgBlockKind::Normal, start, end);
            builder.add_edge(current, ret_id, CfgEdgeKind::Normal);
            builder.add_edge(ret_id, exit_id, CfgEdgeKind::Return);
            terminated = true;
        } else if rules.break_node == Some(kind) {
            // ─── Break ───────────────────────────────────────────────
            let brk_id = builder.new_block("break", CfgBlockKind::Normal, start, end);
            builder.add_edge(current, brk_id, CfgEdgeKind::Normal);

            if let Some(ctx) = builder.loop_stack.last() {
                if let Some(after) = ctx.after_id {
                    builder.add_edge(brk_id, after, CfgEdgeKind::Break);
                }
            }
            terminated = true;
        } else if rules.continue_node == Some(kind) {
            // ─── Continue ────────────────────────────────────────────
            let cont_id = builder.new_block("continue", CfgBlockKind::Normal, start, end);
            builder.add_edge(current, cont_id, CfgEdgeKind::Normal);

            if let Some(ctx) = builder.loop_stack.last() {
                builder.add_edge(cont_id, ctx.header_id, CfgEdgeKind::Continue);
            }
            terminated = true;
        } else {
            // ─── Normal Statement ────────────────────────────────────
            // Accumulate into current block by just updating its end line.
            // But if current is the predecessor (ENTRY or a branch), create a new block.
            if current == predecessor
                || builder.blocks[current as usize].kind != CfgBlockKind::Normal
            {
                let stmt_id = builder.new_block("stmt", CfgBlockKind::Normal, start, end);
                builder.add_edge(current, stmt_id, CfgEdgeKind::Normal);
                current = stmt_id;
            } else {
                // Extend current block's end line.
                builder.blocks[current as usize].end_line = end;
            }
        }
    }

    if terminated { None } else { Some(current) }
}

// ─── Display ─────────────────────────────────────────────────────────────────

impl FunctionCfg {
    /// Format the CFG as a human-readable string for DSL output.
    pub fn format_summary(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "blocks={} edges={}\n",
            self.blocks.len(),
            self.edges.len()
        ));
        for block in &self.blocks {
            out.push_str(&format!(
                "  B{}: {:?} \"{}\" L{}-{}\n",
                block.id, block.kind, block.label, block.start_line, block.end_line
            ));
        }
        for edge in &self.edges {
            out.push_str(&format!(
                "  B{} -> B{} [{:?}]\n",
                edge.from, edge.to, edge.kind
            ));
        }
        out
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

    fn find_function(tree: &tree_sitter::Tree) -> TsNode<'_> {
        let root = tree.root_node();
        let mut cursor = root.walk();
        for child in root.named_children(&mut cursor) {
            if child.kind() == "function_item" {
                return child;
            }
        }
        panic!("no function_item found in source");
    }

    #[test]
    fn cfg_simple_function_no_branches() {
        let src = r#"
fn foo() {
    let x = 1;
    let y = 2;
    let z = x + y;
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have ENTRY, EXIT, and one statement block.
        assert!(
            cfg.blocks.len() >= 3,
            "expected >=3 blocks, got {}",
            cfg.blocks.len()
        );
        assert_eq!(cfg.blocks[0].kind, CfgBlockKind::Entry);
        assert_eq!(cfg.blocks[1].kind, CfgBlockKind::Exit);

        // At least 2 Normal edges: ENTRY→stmt, stmt→EXIT.
        let normal_edges: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.kind == CfgEdgeKind::Normal)
            .collect();
        assert!(
            normal_edges.len() >= 2,
            "expected >=2 normal edges, got {}",
            normal_edges.len()
        );
    }

    #[test]
    fn cfg_if_else() {
        let src = r#"
fn foo(x: i32) {
    if x > 0 {
        let a = 1;
    } else {
        let b = 2;
    }
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have a Branch block.
        let branch_blocks: Vec<_> = cfg
            .blocks
            .iter()
            .filter(|b| b.kind == CfgBlockKind::Branch)
            .collect();
        assert!(
            !branch_blocks.is_empty(),
            "expected at least one Branch block"
        );

        // Should have BranchFalse edge (from branch to else path).
        let false_edges: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.kind == CfgEdgeKind::BranchFalse)
            .collect();
        assert!(
            !false_edges.is_empty(),
            "expected at least one BranchFalse edge"
        );
    }

    #[test]
    fn cfg_for_loop() {
        let src = r#"
fn foo() {
    for i in 0..10 {
        let x = i;
    }
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have a Loop block.
        let loop_blocks: Vec<_> = cfg
            .blocks
            .iter()
            .filter(|b| b.kind == CfgBlockKind::Loop)
            .collect();
        assert!(!loop_blocks.is_empty(), "expected at least one Loop block");

        // Should have a LoopBack edge.
        let loopback_edges: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.kind == CfgEdgeKind::LoopBack)
            .collect();
        assert!(
            !loopback_edges.is_empty(),
            "expected at least one LoopBack edge"
        );
    }

    #[test]
    fn cfg_while_loop() {
        let src = r#"
fn foo() {
    let mut x = 0;
    while x < 10 {
        x += 1;
    }
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have a Loop block.
        let loop_blocks: Vec<_> = cfg
            .blocks
            .iter()
            .filter(|b| b.kind == CfgBlockKind::Loop)
            .collect();
        assert!(!loop_blocks.is_empty(), "expected at least one Loop block");

        // Should have a LoopBack edge.
        let loopback_edges: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.kind == CfgEdgeKind::LoopBack)
            .collect();
        assert!(
            !loopback_edges.is_empty(),
            "expected at least one LoopBack edge"
        );

        // Should have BranchFalse edge (condition false → after loop).
        let false_edges: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.kind == CfgEdgeKind::BranchFalse)
            .collect();
        assert!(
            !false_edges.is_empty(),
            "expected BranchFalse edge for while condition"
        );
    }

    #[test]
    fn cfg_match_expression() {
        let src = r#"
fn foo(x: i32) {
    match x {
        1 => { let a = 1; }
        2 => { let b = 2; }
        _ => { let c = 3; }
    }
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have a Branch block for the match.
        let branch_blocks: Vec<_> = cfg
            .blocks
            .iter()
            .filter(|b| b.kind == CfgBlockKind::Branch)
            .collect();
        assert!(!branch_blocks.is_empty(), "expected Branch block for match");

        // The branch block should have multiple outgoing edges (one per arm).
        let branch_id = branch_blocks[0].id;
        let outgoing: Vec<_> = cfg.edges.iter().filter(|e| e.from == branch_id).collect();
        assert!(
            outgoing.len() >= 3,
            "expected >=3 edges from match branch, got {}",
            outgoing.len()
        );
    }

    #[test]
    fn cfg_return_statement() {
        let src = r#"
fn foo(x: i32) -> i32 {
    if x > 0 {
        return x;
    }
    0
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have a Return edge to EXIT.
        let return_edges: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.kind == CfgEdgeKind::Return)
            .collect();
        assert!(
            !return_edges.is_empty(),
            "expected at least one Return edge"
        );

        // The Return edge should point to the EXIT block (id=1).
        assert!(
            return_edges.iter().any(|e| e.to == 1),
            "Return edge should point to EXIT (id=1)"
        );
    }

    #[test]
    fn cfg_nested_if_in_loop() {
        let src = r#"
fn foo() {
    for i in 0..10 {
        if i > 5 {
            let x = i;
        }
    }
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have both Loop and Branch blocks.
        let loop_blocks: Vec<_> = cfg
            .blocks
            .iter()
            .filter(|b| b.kind == CfgBlockKind::Loop)
            .collect();
        let branch_blocks: Vec<_> = cfg
            .blocks
            .iter()
            .filter(|b| b.kind == CfgBlockKind::Branch)
            .collect();

        assert!(!loop_blocks.is_empty(), "expected Loop block");
        assert!(
            !branch_blocks.is_empty(),
            "expected Branch block inside loop"
        );

        // Should have both LoopBack and BranchFalse edges.
        assert!(cfg.edges.iter().any(|e| e.kind == CfgEdgeKind::LoopBack));
        assert!(cfg.edges.iter().any(|e| e.kind == CfgEdgeKind::BranchFalse));
    }

    #[test]
    fn cfg_rust_loop_with_break() {
        let src = r#"
fn foo() {
    loop {
        break;
    }
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have a Loop block (for infinite loop).
        let loop_blocks: Vec<_> = cfg
            .blocks
            .iter()
            .filter(|b| b.kind == CfgBlockKind::Loop)
            .collect();
        assert!(!loop_blocks.is_empty(), "expected Loop block for `loop`");

        // Should have a Break edge.
        let break_edges: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.kind == CfgEdgeKind::Break)
            .collect();
        assert!(!break_edges.is_empty(), "expected Break edge");
    }

    #[test]
    fn cfg_returns_none_for_non_function() {
        let src = r#"
struct Foo {
    x: i32,
}
"#;
        let tree = parse_rust(src);
        let root = tree.root_node();
        let mut cursor = root.walk();
        let struct_node = root
            .named_children(&mut cursor)
            .find(|c| c.kind() == "struct_item")
            .unwrap();

        let result = build_cfg(struct_node, src.as_bytes(), "rust");
        assert!(result.is_none());
    }

    #[test]
    fn cfg_continue_statement() {
        let src = r#"
fn foo() {
    for i in 0..10 {
        if i == 5 {
            continue;
        }
        let x = i;
    }
}
"#;
        let tree = parse_rust(src);
        let func = find_function(&tree);
        let cfg = build_cfg(func, src.as_bytes(), "rust").unwrap();

        // Should have a Continue edge pointing back to loop header.
        let continue_edges: Vec<_> = cfg
            .edges
            .iter()
            .filter(|e| e.kind == CfgEdgeKind::Continue)
            .collect();
        assert!(!continue_edges.is_empty(), "expected Continue edge");

        // The Continue edge target should be a Loop block.
        for ce in &continue_edges {
            let target = &cfg.blocks[ce.to as usize];
            assert_eq!(
                target.kind,
                CfgBlockKind::Loop,
                "Continue should target a Loop block"
            );
        }
    }
}
