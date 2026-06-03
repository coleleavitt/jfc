//! Agent-friendly context builder, mirroring codegraph's
//! `codegraph_context` / `codegraph_explore` / `codegraph_search` /
//! `codegraph_callers` / `codegraph_callees` / `codegraph_impact`
//! shape on top of the jfc-graph code graph.
//!
//! The submodules split the work:
//! - [`budget`]: adaptive output budget tier table
//! - [`heuristics`]: feature/bug/exploration intent classifier
//! - [`resolver`]: qualified-name → NodeId matcher with `::` / `.` /
//!   `/` separators and Rust-prefix stripping
//! - [`expansion`]: type hierarchy walk, per-file diversity cap,
//!   test-path cap, edge recovery, co-location boost
//! - [`clustering`]: range merging + source slicing for explore
//! - [`render`]: markdown formatting (`##` headers, `**bold**`,
//!   fenced source blocks, file-grouped impact, handles footer)

pub mod budget;
pub mod clustering;
pub mod dataflow_seed;
pub mod expansion;
pub mod heuristics;
pub mod measure;
pub mod render;
pub mod resolver;
pub mod retrieval_gate;

use std::collections::HashSet;

use crate::edges::EdgeKind;
use crate::graph::CodeGraph;
use crate::nodes::NodeId;
use crate::symbols::SymbolTable;
use crate::traversal::{TraversalConfig, TraversalDirection, traverse};

pub use budget::ExploreBudget;
pub use dataflow_seed::{seed_from_dataflow, seed_from_nodes};
pub use expansion::ExpandedSubgraph;
pub use heuristics::{TaskIntent, classify_intent};
pub use resolver::{MatchQuality, matches_symbol, resolve_symbol};
pub use retrieval_gate::{RetrievalSignal, can_skip_retrieval, should_retrieve};

/// Options for `codegraph_context`-style queries.
#[derive(Debug, Clone)]
pub struct ContextOptions {
    /// Maximum entry-point + related-symbol nodes to surface.
    pub max_nodes: usize,
    /// Include source-code blocks for the entry points.
    pub include_code: bool,
    /// BFS expansion depth from entry points.
    pub traversal_depth: u8,
    /// Force the related-node expansion even when the Repoformer retrieval gate
    /// would abstain. Used to measure the gate's saved work (the pre-gate
    /// baseline); normal callers leave this `false`.
    pub force_expand: bool,
}

impl Default for ContextOptions {
    fn default() -> Self {
        Self {
            max_nodes: 20,
            include_code: true,
            traversal_depth: 1,
            force_expand: false,
        }
    }
}

/// Full context-builder result: the chosen entry points, the related
/// symbols discovered by BFS + hierarchy walk, the rendered markdown,
/// and the budget tier used.
#[derive(Debug, Clone)]
pub struct ContextResult {
    pub query: String,
    pub entry_points: Vec<NodeId>,
    pub related: Vec<NodeId>,
    pub markdown: String,
    pub intent: TaskIntent,
    pub budget: ExploreBudget,
}

/// Build an agent-friendly context for `task`. Resolves entry points
/// by exact-name + qualified-name lookup, expands the type hierarchy
/// (1 hop), runs a shallow BFS, enforces per-file diversity, deprioritises
/// test files, and renders the result with intent-aware reminders.
pub fn build_context(
    graph: &CodeGraph,
    symbols: Option<&SymbolTable>,
    task: &str,
    opts: ContextOptions,
) -> ContextResult {
    let budget = ExploreBudget::for_file_count(distinct_file_count(graph));
    let intent = classify_intent(task);

    let entry_points = seed_entry_points(graph, task, opts.max_nodes);

    // Repoformer (arXiv:2403.10059) when-to-retrieve gating: if the entry points
    // are entirely self-contained (no cross-file / external edges), the
    // expensive BFS + type-hierarchy expansion is unlikely to add value, so we
    // abstain from it and return just the entry points. This cuts graph_context
    // latency on local-only queries. We still expand whenever any signal says
    // the local view is incomplete.
    let signal = compute_retrieval_signal(graph, &entry_points);
    // The gate decides whether expansion is worth it — unless a caller forces it
    // (used by the measurement harness to capture the pre-gate baseline).
    let do_expand = opts.force_expand || retrieval_gate::should_retrieve(&signal);
    let mut related = if do_expand {
        expand_related(graph, &entry_points, opts.traversal_depth, opts.max_nodes)
    } else {
        Vec::new()
    };

    // Type hierarchy fills in parent traits + sibling implementors that
    // BFS alone may miss when its budget gets eaten by Contains edges.
    if do_expand {
        let hierarchy_budget = (opts.max_nodes / 4).max(2);
        let hierarchy = expansion::expand_type_hierarchy(graph, &entry_points, hierarchy_budget);
        for id in hierarchy.nodes {
            if !related.contains(&id) && !entry_points.contains(&id) {
                related.push(id);
            }
        }
    }

    let roots: HashSet<NodeId> = entry_points.iter().cloned().collect();
    let per_file_cap = (opts.max_nodes / 5).max(2);
    let mut all = entry_points.clone();
    all.extend(related.iter().cloned());
    let diversified = expansion::enforce_file_diversity(graph, all, &roots, per_file_cap);

    let max_non_prod = (opts.max_nodes / 7).max(1);
    let capped = expansion::cap_test_files(graph, diversified, max_non_prod);

    let related_final: Vec<NodeId> = capped
        .iter()
        .filter(|id| !entry_points.contains(id))
        .cloned()
        .collect();

    let code_blocks: Vec<(NodeId, String)> = if opts.include_code {
        entry_points
            .iter()
            .filter_map(|id| read_node_source(graph, id).map(|src| (id.clone(), src)))
            .collect()
    } else {
        Vec::new()
    };

    let _ = symbols;
    let markdown = render::render_context(
        graph,
        task,
        &entry_points,
        &related_final,
        &code_blocks,
        intent,
        &budget,
    );

    ContextResult {
        query: task.to_string(),
        entry_points,
        related: related_final,
        markdown,
        intent,
        budget,
    }
}

/// Resolve the entry points for a task: name matches, then DRACO dataflow
/// seeding.
///
/// DRACO (arXiv:2405.17337): augment the name-matched entry points with the
/// cursor's *dataflow dependencies* — the type/return/call neighbours of the
/// resolved symbols — so the context is seeded by what the code actually
/// depends on, not just textual name matches. The DRACO seeds are bounded so
/// they can't crowd out the name entries.
fn seed_entry_points(graph: &CodeGraph, task: &str, max_nodes: usize) -> Vec<NodeId> {
    let name_entries = pick_entry_points(graph, task, max_nodes / 4);
    let mut entry_points = name_entries.clone();
    let draco_cap = (max_nodes / 4).max(2);
    for id in dataflow_seed::seed_from_nodes(graph, &name_entries, 1) {
        if entry_points.len() >= name_entries.len() + draco_cap {
            break;
        }
        if !entry_points.contains(&id) {
            entry_points.push(id);
        }
    }
    entry_points
}

/// Compute a [`RetrievalSignal`] for a set of entry points: count how many of
/// their outgoing edges cross a file boundary (cross-module) or point at an
/// unresolved/external symbol. Drives the Repoformer when-to-retrieve gate —
/// when nothing reaches outside the entry points' own files, the local view is
/// self-contained and expansion is skipped.
fn compute_retrieval_signal(graph: &CodeGraph, entry_points: &[NodeId]) -> RetrievalSignal {
    let mut cross_module_refs = 0u32;
    let mut references_external_symbol = false;
    for id in entry_points {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        for (target, edge) in graph.get_edges_from(id) {
            if matches!(
                edge.kind,
                EdgeKind::UnresolvedCall(_) | EdgeKind::ExternalCall(_, _)
            ) {
                references_external_symbol = true;
            }
            if let Some(target_node) = graph.get_node(target)
                && target_node.file_path != node.file_path
            {
                cross_module_refs += 1;
            }
        }
    }
    RetrievalSignal {
        cross_module_refs,
        unresolved_types: 0,
        references_external_symbol,
        local_self_contained: cross_module_refs == 0 && !references_external_symbol,
    }
}

/// Resolve symbol candidates from a free-form `task` description.
/// Extracts identifier-shaped tokens, then tries each via the qualified
/// resolver. Caps the returned entry-point list at `limit`.
fn pick_entry_points(graph: &CodeGraph, task: &str, limit: usize) -> Vec<NodeId> {
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut out: Vec<NodeId> = Vec::new();
    for tok in extract_identifiers(task) {
        if out.len() >= limit {
            break;
        }
        for id in resolve_symbol(graph, &tok) {
            if seen.insert(id.clone()) {
                out.push(id);
                if out.len() >= limit {
                    break;
                }
            }
        }
    }
    out
}

/// Pull plausible identifier tokens out of a natural-language task —
/// camelCase, PascalCase, snake_case, dotted, qualified. Drops short
/// English words and our internal stop-list to avoid resolving "the",
/// "from", "into", …
fn extract_identifiers(task: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for raw in
        task.split(|c: char| !c.is_alphanumeric() && c != '_' && c != ':' && c != '.' && c != '/')
    {
        if raw.len() < 3 || STOP_WORDS.contains(&raw.to_lowercase().as_str()) {
            continue;
        }
        if !raw.chars().any(|c| c.is_alphabetic()) {
            continue;
        }
        let key = raw.to_string();
        if seen.insert(key.clone()) {
            out.push(key);
        }
    }
    out
}

const STOP_WORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "this", "that", "have", "into", "but", "not", "are",
    "was", "were", "has", "had", "its", "can", "did", "may", "also", "than", "then", "them",
    "each", "some", "such", "only", "same", "about", "after", "before", "between", "through",
    "during", "without", "again", "further", "once", "here", "there", "both", "just", "more",
    "most", "very", "how", "what", "when", "where", "which", "who", "why", "does", "doing", "done",
    "use", "used", "using",
];

fn expand_related(graph: &CodeGraph, seeds: &[NodeId], depth: u8, budget: usize) -> Vec<NodeId> {
    let config = TraversalConfig {
        max_depth: depth as usize,
        max_nodes: budget,
        direction: TraversalDirection::Both,
        parallel: false,
    };
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut out: Vec<NodeId> = Vec::new();
    for seed in seeds {
        let result = traverse(graph, seed, &config);
        for id in result.nodes {
            if seeds.contains(&id) {
                continue;
            }
            if seen.insert(id.clone()) {
                out.push(id);
                if out.len() >= budget {
                    return out;
                }
            }
        }
    }
    out
}

fn distinct_file_count(graph: &CodeGraph) -> usize {
    let mut files: HashSet<&std::path::Path> = HashSet::new();
    for id in graph.all_node_ids() {
        if let Some(node) = graph.get_node(id) {
            files.insert(node.file_path.as_path());
        }
    }
    files.len()
}

/// Read source text for a single node from disk. Returns `None` if the
/// file can't be read or the span is empty.
fn read_node_source(graph: &CodeGraph, id: &NodeId) -> Option<String> {
    let node = graph.get_node(id)?;
    if node.span.start_line == 0 {
        return None;
    }
    let content = std::fs::read_to_string(&node.file_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = node.span.start_line.saturating_sub(1) as usize;
    let end = (node.span.end_line as usize).min(lines.len());
    if start >= end {
        return None;
    }
    let mut out = String::new();
    for (offset, line) in lines[start..end].iter().enumerate() {
        let lineno = start + 1 + offset;
        out.push_str(&format!("{lineno}\t{line}\n"));
    }
    Some(out.trim_end().to_string())
}

/// Render a callers / callees result starting from one or more
/// resolved seed nodes.
pub fn callers_for(graph: &CodeGraph, symbol: &str, limit: usize) -> (Vec<NodeId>, Option<String>) {
    aggregate_neighbors(graph, symbol, limit, NeighborDirection::Incoming)
}

pub fn callees_for(graph: &CodeGraph, symbol: &str, limit: usize) -> (Vec<NodeId>, Option<String>) {
    aggregate_neighbors(graph, symbol, limit, NeighborDirection::Outgoing)
}

#[derive(Debug, Clone, Copy)]
enum NeighborDirection {
    Incoming,
    Outgoing,
}

fn aggregate_neighbors(
    graph: &CodeGraph,
    symbol: &str,
    limit: usize,
    direction: NeighborDirection,
) -> (Vec<NodeId>, Option<String>) {
    let matches = resolve_symbol(graph, symbol);
    if matches.is_empty() {
        return (Vec::new(), None);
    }
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut out: Vec<NodeId> = Vec::new();
    for id in &matches {
        let neighbors = match direction {
            NeighborDirection::Incoming => graph.get_edges_to(id),
            NeighborDirection::Outgoing => graph.get_edges_from(id),
        };
        for (nbr, edge) in neighbors {
            if !matches!(edge.kind, EdgeKind::Calls | EdgeKind::UnresolvedCall(_)) {
                continue;
            }
            if seen.insert(nbr.clone()) {
                out.push(nbr.clone());
                if out.len() >= limit {
                    break;
                }
            }
        }
        if out.len() >= limit {
            break;
        }
    }
    let note = if matches.len() > 1 {
        Some(format!(
            "Aggregated across {} symbols named `{}`",
            matches.len(),
            symbol
        ))
    } else {
        None
    };
    (out, note)
}

/// Build an impact set: walk *callers* outward N hops to surface every
/// symbol whose behaviour might shift if the seed changes.
pub fn impact_for(graph: &CodeGraph, symbol: &str, depth: u8) -> (Vec<NodeId>, Option<String>) {
    let matches = resolve_symbol(graph, symbol);
    if matches.is_empty() {
        return (Vec::new(), None);
    }
    let config = TraversalConfig {
        max_depth: depth as usize,
        max_nodes: 500,
        direction: TraversalDirection::Incoming,
        parallel: false,
    };
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut out: Vec<NodeId> = Vec::new();
    for seed in &matches {
        let result = traverse(graph, seed, &config);
        for id in result.nodes {
            if seen.insert(id.clone()) {
                out.push(id);
            }
        }
    }
    let note = if matches.len() > 1 {
        Some(format!(
            "Aggregated impact across {} symbols named `{}`",
            matches.len(),
            symbol
        ))
    } else {
        None
    };
    (out, note)
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::edges::EdgeData;
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};

    fn span_at(start: u32, end: u32) -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: start,
            start_col: 0,
            end_line: end,
            end_col: 0,
            byte_range: 0..1,
        }
    }

    fn node(name: &str, kind: NodeKind) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: span_at(1, 10),
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
    fn extract_identifiers_drops_stop_words() {
        let toks = extract_identifiers("how does the foo_bar work with Baz");
        assert!(toks.contains(&"foo_bar".to_string()));
        assert!(toks.contains(&"Baz".to_string()));
        assert!(!toks.contains(&"the".to_string()));
        assert!(!toks.contains(&"how".to_string()));
    }

    #[test]
    fn extract_identifiers_keeps_qualified_paths() {
        let toks = extract_identifiers("look at crate::stage_apply::run");
        assert!(toks.iter().any(|t| t.contains("stage_apply")));
    }

    #[test]
    fn build_context_finds_entry_points_by_name() {
        let mut g = CodeGraph::new();
        let _id = g.add_node(node("alpha", NodeKind::Function));
        let result = build_context(&g, None, "explore alpha", ContextOptions::default());
        assert_eq!(result.entry_points.len(), 1);
        assert_eq!(result.intent, TaskIntent::Exploration);
        assert!(result.markdown.contains("## Code Context"));
    }

    #[test]
    fn build_context_marks_feature_intent() {
        let mut g = CodeGraph::new();
        let _id = g.add_node(node("widget", NodeKind::Function));
        let result = build_context(&g, None, "add a widget", ContextOptions::default());
        assert_eq!(result.intent, TaskIntent::Feature);
        assert!(result.markdown.contains("UX preferences"));
    }

    /// Node in an explicit file (the default `node` helper hardcodes src/lib.rs).
    fn node_in(name: &str, kind: NodeKind, file: &str) -> NodeData {
        let id = NodeId::new(file, &format!("crate::{name}"), kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from(file),
            span: Span {
                file: PathBuf::from(file),
                start_line: 1,
                start_col: 0,
                end_line: 10,
                end_col: 0,
                byte_range: 0..1,
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

    // DRACO: build_context entry points include the resolved symbol's dataflow
    // dependency (a type it uses), not just name matches.
    #[test]
    fn build_context_draco_seeds_dataflow_deps_normal() {
        let mut g = CodeGraph::new();
        let handler = g.add_node(node("handler", NodeKind::Function));
        let req = g.add_node(node("Request", NodeKind::Struct));
        g.add_edge(
            &handler,
            &req,
            EdgeData { kind: EdgeKind::UsesType, source_span: span_at(1, 1), weight: 1.0 },
        )
        .unwrap();

        let result = build_context(&g, None, "look at handler", ContextOptions::default());
        // `Request` is never named in the task, but DRACO seeds it via the
        // UsesType dataflow edge out of `handler`.
        assert!(
            result.entry_points.contains(&req),
            "DRACO should seed the dataflow dependency Request"
        );
    }

    // Repoformer gate: a fully self-contained entry point (no cross-file or
    // external edges) skips the related-expansion BFS.
    #[test]
    fn build_context_gate_abstains_when_self_contained_normal() {
        let mut g = CodeGraph::new();
        // Two functions in the SAME file, no edges at all -> self-contained.
        g.add_node(node_in("solo", NodeKind::Function, "src/solo.rs"));
        let result = build_context(&g, None, "look at solo", ContextOptions::default());
        // Entry point resolves, but with no outward edges the gate abstains, so
        // there are no related nodes.
        assert!(!result.entry_points.is_empty());
        assert!(
            result.related.is_empty(),
            "self-contained query should skip expansion, got {:?}",
            result.related
        );
    }

    // Repoformer gate: a cross-file edge makes the signal non-self-contained, so
    // expansion runs and surfaces the related node.
    #[test]
    fn build_context_gate_expands_when_cross_file_robust() {
        let mut g = CodeGraph::new();
        let a = g.add_node(node_in("alpha", NodeKind::Function, "src/a.rs"));
        let b = g.add_node(node_in("beta", NodeKind::Function, "src/b.rs"));
        g.add_edge(
            &a,
            &b,
            EdgeData { kind: EdgeKind::Calls, source_span: span_at(1, 1), weight: 1.0 },
        )
        .unwrap();

        let result = build_context(&g, None, "look at alpha", ContextOptions::default());
        // alpha -> beta crosses a file boundary, so the gate retrieves and the
        // BFS surfaces beta as related (or as a DRACO seed entry).
        let found = result.related.contains(&b) || result.entry_points.contains(&b);
        assert!(found, "cross-file query should expand to beta");
    }

    #[test]
    fn callers_for_aggregates_across_matches() {
        let mut g = CodeGraph::new();
        let target = g.add_node(node("target", NodeKind::Function));
        let caller = g.add_node(node("caller", NodeKind::Function));
        g.add_edge(
            &caller,
            &target,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span_at(1, 1),
                weight: 1.0,
            },
        )
        .unwrap();
        let (nodes, _) = callers_for(&g, "target", 10);
        assert_eq!(nodes.len(), 1);
    }

    #[test]
    fn impact_for_walks_callers_outward() {
        let mut g = CodeGraph::new();
        let target = g.add_node(node("target", NodeKind::Function));
        let mid = g.add_node(node("mid", NodeKind::Function));
        let outer = g.add_node(node("outer", NodeKind::Function));
        g.add_edge(
            &mid,
            &target,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span_at(1, 1),
                weight: 1.0,
            },
        )
        .unwrap();
        g.add_edge(
            &outer,
            &mid,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span_at(1, 1),
                weight: 1.0,
            },
        )
        .unwrap();
        let (nodes, _) = impact_for(&g, "target", 3);
        assert!(nodes.contains(&mid));
        assert!(nodes.contains(&outer));
    }

    #[test]
    fn distinct_file_count_matches_unique_paths() {
        let mut g = CodeGraph::new();
        let _a = g.add_node(node("a", NodeKind::Function));
        let _b = g.add_node(node("b", NodeKind::Function));
        // Both nodes share `src/lib.rs` → 1 distinct file.
        assert_eq!(distinct_file_count(&g), 1);
    }
}
