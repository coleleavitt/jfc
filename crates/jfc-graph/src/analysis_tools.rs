//! Agent-facing CPG analysis façade — program slicing, data dependencies, and
//! taint flow rendered as compact, source-annotated, path-capped reports.
//!
//! From *Bridging Code Property Graphs and Language Models*: a coding agent
//! gets far more leverage from a **backward slice** ("everything that can
//! affect this value", ≈ −90% of the code it would otherwise read) or a
//! **taint path** ("does untrusted input reach this sink?") than from raw
//! file reads. jfc already has the analyses ([`crate::slicing`],
//! [`crate::taint_v2`], [`crate::points_to`]) but no production driver wired
//! them to the live graph — they ran only over test-fixture IR maps.
//!
//! This module is that driver. Each entry point:
//! 1. builds the interprocedural IR map for the graph ([`crate::ir_map`]),
//! 2. constructs the real [`PointsToOracle`] from it,
//! 3. runs the requested analysis,
//! 4. caps the result (`max_paths` / node budget) so the agent isn't flooded,
//! 5. renders each node as `name (file:line)` so the model can jump to source.
//!
//! Source-annotation is "serialize-slice-to-source": the agent sees *where*
//! each slice/flow node lives without a follow-up `Read`.

use crate::graph::CodeGraph;
use crate::ir_map::build_ir_map;
use crate::nodes::NodeId;
use crate::slicing::{PointsToOracle, backward_slice, forward_slice};
use crate::taint_v2::{TaintConfig, analyze as taint_analyze};

/// Default hop cap for slice BFS — deep enough for real chains, bounded so a
/// dense graph can't explode the output.
const DEFAULT_SLICE_DEPTH: usize = 6;

/// Resolve a symbol name to its function NodeIds (qualified-name aware), via
/// the same matcher the context tools use.
fn resolve(graph: &CodeGraph, symbol: &str) -> Vec<NodeId> {
    crate::context::resolver::resolve_symbol(graph, symbol)
}

/// Render a node as `qualified_name (file:line)`, or its raw id when the node
/// is missing (it was pruned since analysis) — never panics.
fn render_node(graph: &CodeGraph, id: &NodeId) -> String {
    match graph.get_node(id) {
        Some(n) => format!(
            "{} ({}:{})",
            n.qualified_name,
            n.file_path.display(),
            n.span.start_line
        ),
        None => format!("<node {id:?}>"),
    }
}

/// Backward program slice: every function whose behaviour can affect the value
/// computed by `symbol`. Returns a source-annotated report, capped at
/// `max_nodes` entries.
pub fn program_slice(
    graph: &CodeGraph,
    symbol: &str,
    backward: bool,
    max_nodes: usize,
) -> String {
    let seeds = resolve(graph, symbol);
    if seeds.is_empty() {
        return format!("No symbol matching `{symbol}` found in the code graph.");
    }
    let ir_map = build_ir_map(graph);
    let oracle = PointsToOracle::build(graph, &ir_map);

    let direction = if backward { "backward" } else { "forward" };
    let mut all: Vec<NodeId> = Vec::new();
    for seed in &seeds {
        let slice = if backward {
            backward_slice(graph, &oracle, seed, DEFAULT_SLICE_DEPTH)
        } else {
            forward_slice(graph, &oracle, seed, DEFAULT_SLICE_DEPTH)
        };
        for n in slice {
            if !all.contains(&n) {
                all.push(n);
            }
        }
    }

    let total = all.len();
    let mut out = format!(
        "{direction} slice of `{symbol}` — {total} node{} (depth {DEFAULT_SLICE_DEPTH}):\n",
        if total == 1 { "" } else { "s" }
    );
    for id in all.iter().take(max_nodes) {
        out.push_str(&format!("  - {}\n", render_node(graph, id)));
    }
    if total > max_nodes {
        out.push_str(&format!("  ... and {} more (raise max_nodes)\n", total - max_nodes));
    }
    out
}

/// Data dependencies of `symbol`: the functions whose values flow *into* it
/// (its one-hop backward data neighbours via the points-to oracle). Distinct
/// from a full slice — this is the immediate dependency set.
pub fn data_dependencies(graph: &CodeGraph, symbol: &str, max_nodes: usize) -> String {
    let seeds = resolve(graph, symbol);
    if seeds.is_empty() {
        return format!("No symbol matching `{symbol}` found in the code graph.");
    }
    let ir_map = build_ir_map(graph);
    let oracle = PointsToOracle::build(graph, &ir_map);
    use crate::slicing::DataflowOracle;

    let mut deps: Vec<NodeId> = Vec::new();
    for seed in &seeds {
        for d in oracle.def_uses(seed) {
            if !deps.contains(&d) {
                deps.push(d);
            }
        }
    }
    let total = deps.len();
    let mut out = format!(
        "data dependencies of `{symbol}` — {total} direct dependenc{}:\n",
        if total == 1 { "y" } else { "ies" }
    );
    if total == 0 {
        out.push_str(
            "  (none found — the points-to oracle sees no incoming dataflow; \
             the function may take no tainted args or its language lacks IR lowering)\n",
        );
    }
    for id in deps.iter().take(max_nodes) {
        out.push_str(&format!("  - {}\n", render_node(graph, id)));
    }
    if total > max_nodes {
        out.push_str(&format!("  ... and {} more (raise max_nodes)\n", total - max_nodes));
    }
    out
}

/// Taint flow: find source→sink flows where `sources` / `sinks` / `sanitizers`
/// are symbol names resolved against the graph. Caps the rendered flows at
/// `max_paths`. Each flow renders its full path source→…→sink with the
/// sanitizer noted when present.
pub fn taint_flow(
    graph: &CodeGraph,
    sources: &[String],
    sinks: &[String],
    sanitizers: &[String],
    max_paths: usize,
) -> String {
    let resolve_all = |names: &[String]| -> Vec<NodeId> {
        let mut ids = Vec::new();
        for name in names {
            for id in resolve(graph, name) {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
        }
        ids
    };
    let source_ids = resolve_all(sources);
    let sink_ids = resolve_all(sinks);
    let sanitizer_ids = resolve_all(sanitizers);

    if source_ids.is_empty() || sink_ids.is_empty() {
        return format!(
            "taint_flow needs at least one resolvable source and sink \
             (resolved {} source(s), {} sink(s)).",
            source_ids.len(),
            sink_ids.len()
        );
    }

    let ir_map = build_ir_map(graph);
    let oracle = PointsToOracle::build(graph, &ir_map);
    let config = TaintConfig {
        sources: &source_ids,
        sinks: &sink_ids,
        sanitizers: &sanitizer_ids,
    };
    let flows = taint_analyze(graph, &oracle, &config);

    let total = flows.len();
    if total == 0 {
        return "No source→sink taint flows found.".to_string();
    }
    let mut out = format!(
        "{total} taint flow{} found:\n",
        if total == 1 { "" } else { "s" }
    );
    for flow in flows.iter().take(max_paths) {
        let path = flow
            .path
            .iter()
            .map(|id| render_node(graph, id))
            .collect::<Vec<_>>()
            .join("\n      → ");
        out.push_str(&format!("\n  • flow:\n      {path}\n"));
        match &flow.passed_through_sanitizer {
            Some(s) => out.push_str(&format!(
                "    (sanitized at {}) — informational\n",
                render_node(graph, s)
            )),
            None => out.push_str("    ⚠ UNSANITIZED — reaches the sink directly\n"),
        }
    }
    if total > max_paths {
        out.push_str(&format!("\n  ... and {} more flow(s) (raise max_paths)\n", total - max_paths));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::{Path, PathBuf};

    fn write_temp(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("jfc_analysistools_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(name);
        let mut f = std::fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    fn fn_node(path: &Path, name: &str, src: &str, decl: &str) -> NodeData {
        let start = src.find(decl).unwrap_or(0);
        NodeData {
            id: NodeId::new(&path.display().to_string(), name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.into(),
            qualified_name: name.into(),
            file_path: path.to_path_buf(),
            span: Span {
                file: path.to_path_buf(),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: start..start + decl.len(),
            },
            visibility: Visibility::Public,
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    // Robust: an unresolvable symbol returns a clear message, not a panic.
    #[test]
    fn slice_unknown_symbol_reports_clearly_robust() {
        let g = CodeGraph::new();
        let out = program_slice(&g, "does_not_exist", true, 20);
        assert!(out.contains("No symbol matching"));
    }

    // Normal: a backward slice over a real two-function file resolves the seed
    // and renders source-annotated output (file:line present).
    #[test]
    fn backward_slice_renders_source_annotated_normal() {
        let src = "pub fn helper(x: i32) -> i32 { x + 1 }\npub fn caller() -> i32 { helper(2) }\n";
        let path = write_temp("slice.rs", src);
        let mut g = CodeGraph::new();
        let helper = g.add_node(fn_node(&path, "helper", src, "fn helper"));
        let caller = g.add_node(fn_node(&path, "caller", src, "fn caller"));
        g.add_edge(
            &caller,
            &helper,
            EdgeData { kind: EdgeKind::Calls, source_span: g.get_node(&caller).unwrap().span.clone(), weight: 1.0 },
        )
        .unwrap();

        let out = program_slice(&g, "helper", true, 20);
        // The seed is always in its own slice (Weiser inclusive), so the report
        // names it with a file:line annotation.
        assert!(out.contains("helper"));
        assert!(out.contains("slice.rs:"));
        let _ = std::fs::remove_file(&path);
    }

    // Robust: taint_flow with no resolvable source/sink explains itself.
    #[test]
    fn taint_flow_requires_source_and_sink_robust() {
        let g = CodeGraph::new();
        let out = taint_flow(&g, &["nope".into()], &["nada".into()], &[], 5);
        assert!(out.contains("needs at least one resolvable source and sink"));
    }

    // Robust: data_dependencies on an unknown symbol reports clearly.
    #[test]
    fn data_deps_unknown_symbol_robust() {
        let g = CodeGraph::new();
        let out = data_dependencies(&g, "ghost", 20);
        assert!(out.contains("No symbol matching"));
    }
}
