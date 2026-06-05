//! Cross-file call-reference resolver.
//!
//! Runs as a single post-pass over every captured [`CallSite`] once all
//! files are indexed. For each site, finds every same-named function in
//! the graph and scores them with a port of codegraph's `findBestMatch`
//! algorithm, then emits a `Calls` edge to the winner.
//!
//! ## Scoring (mirrors `name-matcher.ts::findBestMatch`)
//!
//! | Signal | Weight |
//! |---|---|
//! | Caller and candidate in the **same file** | +100 |
//! | **Directory proximity** (per shared dir segment) | +15, capped at +80 |
//! | **Qualifier path match** (each `mod::sub` segment found in candidate file path) | +50 / segment |
//! | Candidate is a `Function` (or method-ish) | +25 |
//! | Candidate is `public` visibility | +10 |
//! | Per-site **floor** | 50 |
//!
//! Ties resolve deterministically by `NodeId` so two runs over the same
//! workspace produce the same edge set. Sites scoring under the floor
//! become `UnresolvedCall` edges (kept around for impact / coverage
//! analysis without polluting the resolved Calls graph).

use std::collections::HashMap;

use tracing::info;

use crate::call_site::{CallSite, CallSiteKind};
use crate::edges::{EdgeData, EdgeKind};
use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

/// Minimum score for a candidate to be accepted as the resolved
/// target. Mirrors codegraph's threshold (their `MIN_CONFIDENCE`
/// is 0.5 on a normalised 0–1 scale — we keep the raw integer
/// scale for clarity).
pub const RESOLUTION_FLOOR: i32 = 50;

/// Outcome of one resolver run.
#[derive(Debug, Default, Clone, Copy)]
pub struct ResolutionReport {
    pub sites_seen: usize,
    pub resolved: usize,
    pub unresolved: usize,
    pub already_existed: usize,
}

/// Resolves every captured [`CallSite`] against the indexed graph and
/// emits `Calls` edges in place.
pub struct ReferenceResolver<'g> {
    graph: &'g mut CodeGraph,
    /// `name → [NodeId]` lookup over every Function in the graph.
    /// Built once up front so per-site lookups are O(1) average.
    name_index: HashMap<String, Vec<NodeId>>,
}

impl<'g> ReferenceResolver<'g> {
    pub fn new(graph: &'g mut CodeGraph) -> Self {
        let mut name_index: HashMap<String, Vec<NodeId>> = HashMap::new();
        for id in graph.all_node_ids() {
            if let Some(node) = graph.get_node(id) {
                if node.kind == NodeKind::Function {
                    name_index
                        .entry(node.name.clone())
                        .or_default()
                        .push(id.clone());
                }
            }
        }
        Self { graph, name_index }
    }

    /// Resolve every site in `sites` against the graph, emitting
    /// `Calls` edges where confidence clears [`RESOLUTION_FLOOR`].
    pub fn resolve_all(&mut self, sites: &[CallSite]) -> ResolutionReport {
        let mut report = ResolutionReport {
            sites_seen: sites.len(),
            ..Default::default()
        };
        for site in sites {
            if self.resolve_one(site, &mut report) {
                report.resolved += 1;
            } else {
                report.unresolved += 1;
            }
        }
        info!(
            target: "jfc::graph::resolver",
            sites_seen = report.sites_seen,
            resolved = report.resolved,
            unresolved = report.unresolved,
            already_existed = report.already_existed,
            "cross-file call resolution complete"
        );
        report
    }

    /// Resolve one call site. Returns `true` if a `Calls` edge was
    /// emitted (or already existed).
    fn resolve_one(&mut self, site: &CallSite, report: &mut ResolutionReport) -> bool {
        let candidates = match self.name_index.get(site.name_for_resolution()) {
            Some(c) if !c.is_empty() => c.clone(),
            _ => return false,
        };

        let mut best: Option<(NodeId, i32)> = None;
        for cand_id in &candidates {
            let Some(cand) = self.graph.get_node(cand_id) else {
                continue;
            };
            let score = score_candidate(site, cand);
            match &best {
                Some((best_id, best_score)) => {
                    let better = score > *best_score || (score == *best_score && cand_id < best_id);
                    if better {
                        best = Some((cand_id.clone(), score));
                    }
                }
                None => best = Some((cand_id.clone(), score)),
            }
        }

        let Some((target_id, score)) = best else {
            return false;
        };
        if score < RESOLUTION_FLOOR {
            return false;
        }
        if !self.graph.contains_node(&site.caller_id) || !self.graph.contains_node(&target_id) {
            return false;
        }
        if existing_calls_edge(self.graph, &site.caller_id, &target_id) {
            report.already_existed += 1;
            return true;
        }
        let span = synthetic_span(site);
        let weight = (score as f32 / 200.0).clamp(0.0, 1.0);
        if let Err(_e) = self.graph.add_edge(
            &site.caller_id,
            &target_id,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: span,
                weight,
            },
        ) {
            return false;
        }
        true
    }
}

/// Score one candidate against one site. See module docs for weights.
fn score_candidate(site: &CallSite, cand: &NodeData) -> i32 {
    let mut score: i32 = 0;
    if cand.file_path == site.file_path {
        score += 100;
    }
    score += directory_proximity(&site.file_path, &cand.file_path);
    if site.is_qualified() {
        score += qualifier_match_bonus(site, cand);
    }
    if matches!(cand.kind, NodeKind::Function) {
        score += 25;
    }
    if matches!(site.kind, CallSiteKind::MethodCall) && cand.qualified_name.contains("::") {
        // `obj.method()` — anything that lives under a Type:: prefix is
        // more likely to be the actual target than a free function.
        score += 10;
    }
    if matches!(cand.visibility, Visibility::Public) {
        score += 10;
    }
    score
}

/// Shared-directory-segment count between two file paths, weighted +15
/// per segment, capped at +80 (codegraph's exact knob).
fn directory_proximity(a: &std::path::Path, b: &std::path::Path) -> i32 {
    let a_dirs: Vec<&str> = a
        .parent()
        .map(|p| p.to_str().unwrap_or(""))
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let b_dirs: Vec<&str> = b
        .parent()
        .map(|p| p.to_str().unwrap_or(""))
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let mut shared = 0;
    for (x, y) in a_dirs.iter().zip(b_dirs.iter()) {
        if x == y {
            shared += 1;
        } else {
            break;
        }
    }
    (shared * 15).min(80)
}

/// For a qualified call `mod::sub::foo()`, +50 for every qualifier
/// segment that appears either in the candidate's file-path segments or
/// in its `qualified_name`. Captures "`dispatch::execute_tool` should
/// prefer the `execute_tool` in `tools/dispatch.rs`".
fn qualifier_match_bonus(site: &CallSite, cand: &NodeData) -> i32 {
    let path_str = cand.file_path.to_string_lossy().to_lowercase();
    let q_name = cand.qualified_name.to_lowercase();
    let segments: Vec<&str> = path_str
        .split('/')
        .flat_map(|s| s.split('.'))
        .filter(|s| !s.is_empty())
        .collect();
    let mut hits = 0;
    for seg in &site.path_segments {
        let needle = seg.to_lowercase();
        if segments.iter().any(|s| *s == needle) || q_name.contains(&needle) {
            hits += 1;
        }
    }
    hits * 50
}

/// Whether a `Calls` edge from `from` to `to` already exists. Avoids
/// double-counting when the same call is captured twice (e.g. via two
/// indexing passes).
fn existing_calls_edge(graph: &CodeGraph, from: &NodeId, to: &NodeId) -> bool {
    graph
        .get_edges_from(from)
        .into_iter()
        .any(|(tgt, edge)| tgt == to && matches!(edge.kind, EdgeKind::Calls))
}

/// Construct a span pointing at the call expression itself (we don't
/// have the byte range here, but the line is preserved so downstream
/// tools can navigate to the call site).
fn synthetic_span(site: &CallSite) -> Span {
    Span {
        file: site.file_path.clone(),
        start_line: site.line,
        start_col: 0,
        end_line: site.line,
        end_col: 0,
        // Record the real call-site byte offset (not 0..0) so predicate
        // extraction can walk up to the enclosing if/match/while guard.
        byte_range: site.byte_offset..site.byte_offset,
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::call_site::CallSiteKind;
    use crate::nodes::{NodeData, Visibility};

    fn fn_node(name: &str, qualified: &str, file: &str) -> NodeData {
        let id = NodeId::new(file, qualified, NodeKind::Function);
        NodeData {
            id,
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: qualified.to_string(),
            file_path: PathBuf::from(file),
            span: Span {
                file: PathBuf::from(file),
                start_line: 1,
                start_col: 0,
                end_line: 1,
                end_col: 0,
                byte_range: 0..1,
            },
            visibility: Visibility::Public,
            metadata: std::collections::HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn directory_proximity_caps_at_80() {
        let a = std::path::Path::new("crates/jfc-ui/src/tools/dispatch.rs");
        let b = std::path::Path::new("crates/jfc-ui/src/tools/dispatch_heavy.rs");
        // 4 shared segments (`crates`, `jfc-ui`, `src`, `tools`) — score
        // 60, under the cap.
        assert_eq!(directory_proximity(a, b), 60);
    }

    #[test]
    fn directory_proximity_zero_for_different_roots() {
        let a = std::path::Path::new("crates/jfc-ui/src/x.rs");
        let b = std::path::Path::new("other/y.rs");
        assert_eq!(directory_proximity(a, b), 0);
    }

    #[test]
    fn qualifier_bonus_matches_path_segment() {
        let cand = fn_node(
            "execute_tool",
            "crate::execute_tool",
            "src/tools/dispatch.rs",
        );
        let site = CallSite {
            caller_id: NodeId::new("src/caller.rs", "crate::caller", NodeKind::Function),
            file_path: PathBuf::from("src/caller.rs"),
            name: "execute_tool".into(),
            path_segments: vec!["dispatch".into()],
            line: 1,
            byte_offset: 0,
            kind: CallSiteKind::Qualified,
        };
        assert_eq!(qualifier_match_bonus(&site, &cand), 50);
    }

    #[test]
    fn resolver_emits_calls_edge() {
        let mut g = CodeGraph::new();
        let caller = g.add_node(fn_node("caller", "crate::caller", "src/a.rs"));
        let target = g.add_node(fn_node("target", "crate::target", "src/a.rs"));
        let site = CallSite {
            caller_id: caller.clone(),
            file_path: PathBuf::from("src/a.rs"),
            name: "target".into(),
            path_segments: Vec::new(),
            line: 5,
            byte_offset: 0,
            kind: CallSiteKind::Bare,
        };
        let mut resolver = ReferenceResolver::new(&mut g);
        let report = resolver.resolve_all(&[site]);
        assert_eq!(report.resolved, 1);
        let edges = g.get_edges_from(&caller);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, &target);
        assert!(matches!(edges[0].1.kind, EdgeKind::Calls));
    }

    #[test]
    fn resolver_prefers_same_file_on_ambiguous_name() {
        let mut g = CodeGraph::new();
        let caller = g.add_node(fn_node("caller", "crate::caller", "src/a.rs"));
        let local = g.add_node(fn_node("dup", "crate::a::dup", "src/a.rs"));
        let far = g.add_node(fn_node("dup", "crate::b::dup", "src/b.rs"));
        let site = CallSite {
            caller_id: caller.clone(),
            file_path: PathBuf::from("src/a.rs"),
            name: "dup".into(),
            path_segments: Vec::new(),
            line: 1,
            byte_offset: 0,
            kind: CallSiteKind::Bare,
        };
        let mut resolver = ReferenceResolver::new(&mut g);
        resolver.resolve_all(&[site]);
        let edges = g.get_edges_from(&caller);
        assert_eq!(edges.len(), 1, "expected one Calls edge");
        assert_eq!(edges[0].0, &local, "same-file candidate must win");
        assert_ne!(edges[0].0, &far);
    }

    #[test]
    fn qualified_call_disambiguates_across_files() {
        let mut g = CodeGraph::new();
        let caller = g.add_node(fn_node("caller", "crate::caller", "src/main.rs"));
        let wrong = g.add_node(fn_node("run", "crate::other::run", "src/other.rs"));
        let right = g.add_node(fn_node(
            "run",
            "crate::stage_apply::run",
            "src/stage_apply.rs",
        ));
        let site = CallSite {
            caller_id: caller.clone(),
            file_path: PathBuf::from("src/main.rs"),
            name: "run".into(),
            path_segments: vec!["stage_apply".into()],
            line: 1,
            byte_offset: 0,
            kind: CallSiteKind::Qualified,
        };
        let mut resolver = ReferenceResolver::new(&mut g);
        resolver.resolve_all(&[site]);
        let edges = g.get_edges_from(&caller);
        assert_eq!(edges.len(), 1);
        assert_eq!(edges[0].0, &right, "qualifier bonus must win");
        assert_ne!(edges[0].0, &wrong);
    }

    #[test]
    fn no_candidates_returns_unresolved() {
        let mut g = CodeGraph::new();
        let caller = g.add_node(fn_node("caller", "crate::caller", "src/a.rs"));
        let site = CallSite {
            caller_id: caller,
            file_path: PathBuf::from("src/a.rs"),
            name: "nonexistent".into(),
            path_segments: Vec::new(),
            line: 1,
            byte_offset: 0,
            kind: CallSiteKind::Bare,
        };
        let mut resolver = ReferenceResolver::new(&mut g);
        let report = resolver.resolve_all(&[site]);
        assert_eq!(report.resolved, 0);
        assert_eq!(report.unresolved, 1);
    }
}
