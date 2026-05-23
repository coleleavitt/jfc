//! Markdown rendering for context / caller / callee / impact results.
//!
//! All output goes through this module so the agent gets a consistent
//! shape: `## Header`, `**Location:**`, fenced code blocks tagged
//! `rust`, file-grouped impact, and a `--- handles ---` footer for
//! chained queries. Each renderer takes the budget so it can truncate
//! itself rather than letting a wrapper post-process the string.

use std::collections::HashMap;

use crate::context::budget::ExploreBudget;
use crate::context::heuristics::{reminder_for, TaskIntent};
use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId, NodeKind, Visibility};
use crate::symbols::SymbolTable;

/// Render a search-result list — one entry per node with kind,
/// location, signature, and a fenced docstring teaser.
pub fn render_search_results(
    graph: &CodeGraph,
    symbols: Option<&SymbolTable>,
    query: &str,
    nodes: &[NodeId],
    note: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("## Search Results ({} found)\n\n", nodes.len()));
    if nodes.is_empty() {
        out.push_str(&format!("No results for `{query}`.\n"));
        return out;
    }
    for id in nodes {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        out.push_str(&format!("### {} ({})\n", node.name, kind_label(node.kind)));
        out.push_str(&format!(
            "{}{}\n",
            node.file_path.display(),
            line_suffix(node)
        ));
        let sig = signature_for(node);
        if !sig.is_empty() {
            out.push_str(&format!("`{sig}`\n"));
        }
        if let Some(handle) = symbols.and_then(|s| s.handle_for_node(id)) {
            out.push_str(&format!("handle: `{handle}`\n"));
        }
        let vis = visibility_label(&node.visibility);
        if !vis.is_empty() {
            out.push_str(&format!("visibility: {vis}\n"));
        }
        out.push('\n');
    }
    if let Some(n) = note {
        out.push_str(&format!("\n> **Note:** {n}\n"));
    }
    out
}

/// Render a callers / callees list — compact one-line-per-result so
/// many entries fit. Includes signature.
pub fn render_node_list(
    graph: &CodeGraph,
    title: &str,
    nodes: &[NodeId],
    note: Option<&str>,
) -> String {
    let mut out = String::new();
    out.push_str(&format!("## {title} ({} found)\n\n", nodes.len()));
    for id in nodes {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        let sig = signature_for(node);
        let sig_suffix = if sig.is_empty() { String::new() } else { format!(" — `{sig}`") };
        out.push_str(&format!(
            "- {} ({}) — {}{}{}\n",
            node.name,
            kind_label(node.kind),
            node.file_path.display(),
            line_suffix(node),
            sig_suffix,
        ));
    }
    if let Some(n) = note {
        out.push_str(&format!("\n> **Note:** {n}\n"));
    }
    out
}

/// Render an impact result grouped by file. Each file block lists the
/// affected symbols inline (`name:line, name:line, ...`).
pub fn render_impact(
    graph: &CodeGraph,
    symbol: &str,
    nodes: &[NodeId],
    note: Option<&str>,
) -> String {
    let mut by_file: HashMap<std::path::PathBuf, Vec<&NodeData>> = HashMap::new();
    let mut total = 0usize;
    for id in nodes {
        if let Some(node) = graph.get_node(id) {
            by_file.entry(node.file_path.clone()).or_default().push(node);
            total += 1;
        }
    }

    let mut out = String::new();
    out.push_str(&format!(
        "## Impact: `{symbol}` affects {total} symbols across {} files\n\n",
        by_file.len()
    ));
    let mut files: Vec<_> = by_file.into_iter().collect();
    files.sort_by_key(|b| std::cmp::Reverse(b.1.len()));
    for (file, mut symbols) in files {
        out.push_str(&format!("**{}:**\n", file.display()));
        symbols.sort_by_key(|n| n.span.start_line);
        let inline: Vec<String> = symbols
            .iter()
            .map(|n| format!("{}:{}", n.name, n.span.start_line))
            .collect();
        out.push_str(&inline.join(", "));
        out.push_str("\n\n");
    }
    if let Some(n) = note {
        out.push_str(&format!("\n> **Note:** {n}\n"));
    }
    out
}

/// Render full `codegraph_context`-style output: entry points,
/// related symbols (grouped by file), code blocks for the entry
/// points themselves (caller's responsibility to feed them in).
pub fn render_context(
    graph: &CodeGraph,
    query: &str,
    entry_points: &[NodeId],
    related: &[NodeId],
    code_blocks: &[(NodeId, String)],
    intent: TaskIntent,
    budget: &ExploreBudget,
) -> String {
    let mut out = String::new();
    out.push_str("## Code Context\n\n");
    out.push_str(&format!("**Query:** {query}\n\n"));
    push_entry_points(&mut out, graph, entry_points);
    push_related_symbols(&mut out, graph, related);
    push_code_blocks(&mut out, graph, code_blocks);
    out.push_str(reminder_for(intent));
    if budget.include_completeness_signal && !code_blocks.is_empty() {
        out.push_str(&format!(
            "\n\n---\n> **Complete source code is included above for {} symbols.**\n",
            code_blocks.len(),
        ));
    }
    out
}

fn push_entry_points(out: &mut String, graph: &CodeGraph, entry_points: &[NodeId]) {
    if entry_points.is_empty() {
        return;
    }
    out.push_str("### Entry Points\n\n");
    for id in entry_points {
        let Some(node) = graph.get_node(id) else { continue };
        out.push_str(&format!(
            "- **{}** ({}) — {}{}\n",
            node.name,
            kind_label(node.kind),
            node.file_path.display(),
            line_suffix(node),
        ));
        let sig = signature_for(node);
        if !sig.is_empty() {
            out.push_str(&format!("  `{sig}`\n"));
        }
    }
    out.push('\n');
}

fn push_related_symbols(out: &mut String, graph: &CodeGraph, related: &[NodeId]) {
    if related.is_empty() {
        return;
    }
    out.push_str("### Related Symbols\n\n");
    let mut by_file: HashMap<std::path::PathBuf, Vec<&NodeData>> = HashMap::new();
    for id in related {
        if let Some(node) = graph.get_node(id) {
            by_file.entry(node.file_path.clone()).or_default().push(node);
        }
    }
    let mut files: Vec<_> = by_file.into_iter().collect();
    files.sort_by_key(|(p, _)| p.clone());
    for (file, symbols) in files {
        let inline: Vec<String> = symbols
            .iter()
            .map(|n| format!("{}:{}", n.name, n.span.start_line))
            .collect();
        out.push_str(&format!("- {}: {}\n", file.display(), inline.join(", ")));
    }
    out.push('\n');
}

fn push_code_blocks(out: &mut String, graph: &CodeGraph, code_blocks: &[(NodeId, String)]) {
    if code_blocks.is_empty() {
        return;
    }
    out.push_str("### Code\n\n");
    for (id, body) in code_blocks {
        let Some(node) = graph.get_node(id) else { continue };
        out.push_str(&format!(
            "#### {} ({}:{})\n\n",
            node.name,
            node.file_path.display(),
            node.span.start_line,
        ));
        out.push_str("```rust\n");
        out.push_str(body);
        if !body.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("```\n\n");
    }
}

/// Render an explore-style payload: a relationships map plus per-file
/// source slices. The caller pre-builds `file_blocks` as
/// `(path, language, header_symbols, body_with_line_numbers)`.
pub fn render_explore(
    query: &str,
    total_symbols: usize,
    total_files: usize,
    relationships: &[(EdgeKind, Vec<(String, String)>)],
    file_blocks: &[(String, String, String, String)],
    additional_files: &[(String, String)],
    budget: &ExploreBudget,
) -> String {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("## Exploration: {query}"));
    lines.push(String::new());
    lines.push(format!(
        "Found {total_symbols} symbols across {total_files} files."
    ));
    lines.push(String::new());

    if budget.include_relationships && !relationships.is_empty() {
        lines.push("### Relationships".into());
        lines.push(String::new());
        for (kind, edges) in relationships {
            lines.push(format!("**{}:**", edge_kind_label(kind)));
            for (src, tgt) in edges.iter().take(budget.max_edges_per_relationship_kind) {
                lines.push(format!("- {src} → {tgt}"));
            }
            if edges.len() > budget.max_edges_per_relationship_kind {
                lines.push(format!(
                    "- ... and {} more",
                    edges.len() - budget.max_edges_per_relationship_kind
                ));
            }
            lines.push(String::new());
        }
    }

    if !file_blocks.is_empty() {
        lines.push("### Source Code".into());
        lines.push(String::new());
        for (path, lang, header, body) in file_blocks {
            lines.push(format!("#### {path} — {header}"));
            lines.push(String::new());
            lines.push(format!("```{lang}"));
            lines.push(body.trim_end().to_string());
            lines.push("```".into());
            lines.push(String::new());
        }
    }

    if budget.include_additional_files && !additional_files.is_empty() {
        lines.push("### Additional relevant files (not shown)".into());
        lines.push(String::new());
        for (path, symbols) in additional_files.iter().take(10) {
            lines.push(format!("- {path}: {symbols}"));
        }
        if additional_files.len() > 10 {
            lines.push(format!("- ... and {} more files", additional_files.len() - 10));
        }
    }

    if budget.include_completeness_signal && !file_blocks.is_empty() {
        lines.push(String::new());
        lines.push("---".into());
        lines.push(format!(
            "> **Complete source code is included above for {} files.** \
             Use Read only for files under 'Additional relevant files' if you need more detail.",
            file_blocks.len(),
        ));
    }

    if budget.include_budget_note {
        lines.push(String::new());
        lines.push(format!(
            "> **Explore budget: {} calls max for this project.** \
             Stop exploring and synthesise your answer once you've used the budget.",
            ExploreBudget::call_budget(usize::MAX), // caller embeds correct value via wrapper
        ));
    }

    let output = lines.join("\n");
    truncate_to_budget(&output, budget.max_output_chars)
}

/// Truncate `text` to `max_chars`, prefer cutting on a newline boundary
/// if one exists in the last 20 %.
fn truncate_to_budget(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        return text.to_string();
    }
    let cut = &text[..max_chars];
    let safe_end = cut
        .rfind('\n')
        .filter(|i| *i > (max_chars * 4 / 5))
        .unwrap_or(max_chars);
    format!(
        "{}\n\n... (output truncated to budget — drill in with a narrower query)",
        &text[..safe_end]
    )
}

pub fn kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Function => "function",
        NodeKind::Struct => "struct",
        NodeKind::Enum => "enum",
        NodeKind::Trait => "trait",
        NodeKind::Module => "module",
    }
}

pub fn edge_kind_label(kind: &EdgeKind) -> &'static str {
    match kind {
        EdgeKind::Calls => "calls",
        EdgeKind::UnresolvedCall(_) => "unresolved_call",
        EdgeKind::UsesType => "uses_type",
        EdgeKind::References => "references",
        EdgeKind::Contains => "contains",
        EdgeKind::Implements => "implements",
        EdgeKind::ExternalCall(_, _) => "external_call",
    }
}

pub fn visibility_label(v: &Visibility) -> &'static str {
    match v {
        Visibility::Public => "public",
        Visibility::Crate => "pub(crate)",
        Visibility::Super => "pub(super)",
        Visibility::Private => "private",
    }
}

fn line_suffix(node: &NodeData) -> String {
    if node.span.start_line > 0 {
        format!(":{}", node.span.start_line)
    } else {
        String::new()
    }
}

/// Best-effort signature reconstruction. Functions surface their
/// metadata `signature` if the adapter populated it; otherwise we
/// fall back to `name(...)`.
fn signature_for(node: &NodeData) -> String {
    if let Some(sig) = node.metadata.get("signature") {
        return sig.clone();
    }
    match node.kind {
        NodeKind::Function => format!("fn {}(...)", node.name),
        NodeKind::Struct => format!("struct {}", node.name),
        NodeKind::Enum => format!("enum {}", node.name),
        NodeKind::Trait => format!("trait {}", node.name),
        NodeKind::Module => format!("mod {}", node.name),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeId, NodeKind, Span, Visibility};

    fn span(start: u32) -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: start,
            start_col: 0,
            end_line: start + 1,
            end_col: 0,
            byte_range: 0..1,
        }
    }

    fn node(name: &str, kind: NodeKind, line: u32) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: span(line),
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn render_search_results_includes_kind_and_signature() {
        let mut g = CodeGraph::new();
        let id = g.add_node(node("foo", NodeKind::Function, 10));
        let out = render_search_results(&g, None, "foo", &[id], None);
        assert!(out.contains("## Search Results"));
        assert!(out.contains("foo (function)"));
        assert!(out.contains("`fn foo(...)`"));
        assert!(out.contains(":10"));
    }

    #[test]
    fn render_search_empty_states_no_results() {
        let g = CodeGraph::new();
        let out = render_search_results(&g, None, "missing", &[], None);
        assert!(out.contains("No results"));
    }

    #[test]
    fn render_impact_groups_by_file() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node("a", NodeKind::Function, 1));
        let b = g.add_node(node("b", NodeKind::Function, 5));
        let out = render_impact(&g, "ToolCall", &[a, b], None);
        assert!(out.contains("## Impact"));
        assert!(out.contains("affects 2 symbols"));
        assert!(out.contains("**src/lib.rs:**"));
        assert!(out.contains("a:1, b:5"));
    }

    #[test]
    fn render_context_includes_feature_reminder() {
        let mut g = CodeGraph::new();
        let id = g.add_node(node("foo", NodeKind::Function, 1));
        let budget = ExploreBudget::for_file_count(1000);
        let out = render_context(
            &g,
            "add a thing",
            &[id],
            &[],
            &[],
            TaskIntent::Feature,
            &budget,
        );
        assert!(out.contains("UX preferences"));
    }

    #[test]
    fn render_context_omits_reminder_for_bugs() {
        let mut g = CodeGraph::new();
        let id = g.add_node(node("foo", NodeKind::Function, 1));
        let budget = ExploreBudget::for_file_count(1000);
        let out = render_context(
            &g,
            "fix the foo crash",
            &[id],
            &[],
            &[],
            TaskIntent::Bug,
            &budget,
        );
        assert!(!out.contains("UX preferences"));
    }

    #[test]
    fn truncate_to_budget_keeps_under_cap() {
        let big = "line\n".repeat(1000);
        let truncated = truncate_to_budget(&big, 200);
        assert!(truncated.len() <= 400);
        assert!(truncated.contains("truncated"));
    }

    #[test]
    fn render_explore_emits_relationships() {
        let budget = ExploreBudget::for_file_count(1000);
        let rels = vec![(EdgeKind::Calls, vec![("a".to_string(), "b".to_string())])];
        let out = render_explore("test", 1, 1, &rels, &[], &[], &budget);
        assert!(out.contains("### Relationships"));
        assert!(out.contains("**calls:**"));
        assert!(out.contains("a → b"));
    }
}
