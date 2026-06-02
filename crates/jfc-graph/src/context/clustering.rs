//! Symbol clustering and source-snippet extraction.
//!
//! When the agent wants to *see* the code (explore / context with
//! `include_source=true`), we sort symbols by start line, merge ones
//! whose spans are close enough (within `gap_threshold` lines) into
//! contiguous clusters, then read the file once and slice each
//! cluster out with a few lines of context padding.
//!
//! The pathological case is a single huge container symbol (a 1 400-
//! line class) — keeping it would merge every method inside into one
//! cluster spanning the whole file, which then tail-trims down to just
//! the container's opening lines and buries the methods the query
//! actually asked about. We drop envelope nodes that cover more than
//! 50 % of a file before clustering.

use std::collections::HashSet;
use std::path::Path;

use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId, NodeKind};

/// A range of lines (1-indexed, inclusive) and a per-symbol importance
/// score. Higher importance = more likely to survive the per-file cap.
#[derive(Debug, Clone)]
pub struct Range {
    pub start: u32,
    pub end: u32,
    pub name: String,
    pub kind: NodeKind,
    pub importance: u8,
}

impl Range {
    pub fn edge_source(start: u32, name: String, importance: u8) -> Self {
        Self {
            start,
            end: start,
            name,
            kind: NodeKind::Function,
            importance,
        }
    }
}

/// A merged group of nearby ranges, ready to be rendered as a single
/// contiguous source slice (with line-number prefixes and a few lines
/// of context padding on each side).
#[derive(Debug, Clone)]
pub struct Cluster {
    pub start: u32,
    pub end: u32,
    pub symbols: Vec<String>,
    /// Sum of `importance` across member ranges.
    pub score: u32,
    /// Highest `importance` seen — used to outrank dense low-priority
    /// clusters with high-priority single members.
    pub max_importance: u8,
}

/// Build per-symbol ranges from a list of node IDs, skipping envelope
/// containers (struct/enum/trait/module spanning > 50 % of the file).
pub fn build_ranges(
    graph: &CodeGraph,
    nodes: &[NodeId],
    entry_points: &HashSet<NodeId>,
    file_line_count: u32,
) -> Vec<Range> {
    build_ranges_with_importance(graph, nodes, file_line_count, |id| {
        if entry_points.contains(id) { 10 } else { 1 }
    })
}

pub fn build_ranges_with_importance<F>(
    graph: &CodeGraph,
    nodes: &[NodeId],
    file_line_count: u32,
    importance_for: F,
) -> Vec<Range>
where
    F: Fn(&NodeId) -> u8,
{
    let mut out = Vec::with_capacity(nodes.len());
    for id in nodes {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        if node.span.start_line == 0 || node.span.end_line == 0 {
            continue;
        }
        let span_size = node.span.end_line.saturating_sub(node.span.start_line) + 1;
        if is_envelope_kind(node.kind) && file_line_count > 0 && span_size * 2 > file_line_count {
            continue;
        }
        let importance = importance_for(id);
        out.push(Range {
            start: node.span.start_line,
            end: node.span.end_line,
            name: node.name.clone(),
            kind: node.kind,
            importance,
        });
    }
    out.sort_by_key(|r| r.start);
    out
}

fn is_envelope_kind(kind: NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::Struct | NodeKind::Enum | NodeKind::Trait | NodeKind::Module
    )
}

/// Merge ranges into clusters by `gap_threshold`-line proximity.
pub fn build_clusters(ranges: &[Range], gap_threshold: u32) -> Vec<Cluster> {
    if ranges.is_empty() {
        return Vec::new();
    }
    let mut clusters: Vec<Cluster> = Vec::new();
    let mut current = Cluster {
        start: ranges[0].start,
        end: ranges[0].end,
        symbols: vec![format!(
            "{}({})",
            ranges[0].name,
            display_kind_label(ranges[0].kind)
        )],
        score: ranges[0].importance as u32,
        max_importance: ranges[0].importance,
    };
    for r in &ranges[1..] {
        if r.start <= current.end.saturating_add(gap_threshold) {
            current.end = current.end.max(r.end);
            current
                .symbols
                .push(format!("{}({})", r.name, display_kind_label(r.kind)));
            current.score = current.score.saturating_add(r.importance as u32);
            if r.importance > current.max_importance {
                current.max_importance = r.importance;
            }
        } else {
            clusters.push(current);
            current = Cluster {
                start: r.start,
                end: r.end,
                symbols: vec![format!("{}({})", r.name, display_kind_label(r.kind))],
                score: r.importance as u32,
                max_importance: r.importance,
            };
        }
    }
    clusters.push(current);
    clusters
}

pub fn display_kind_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Function => "fn",
        NodeKind::Struct => "struct",
        NodeKind::Enum => "enum",
        NodeKind::Trait => "trait",
        NodeKind::Module => "mod",
        NodeKind::EnumVariant => "variant",
        NodeKind::Field => "field",
        NodeKind::TypeAlias => "type",
        NodeKind::Constant => "const",
        NodeKind::Interface => "interface",
    }
}

/// Read `file` and slice out `[cluster.start - padding, cluster.end +
/// padding]`, returning the slice with 1-indexed line-number prefixes
/// (cat-n style). Returns `None` if the file can't be read.
pub fn read_cluster_source(file: &Path, cluster: &Cluster, context_padding: u32) -> Option<String> {
    let content = std::fs::read_to_string(file).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = (cluster.start.saturating_sub(1 + context_padding)) as usize;
    let end = ((cluster.end + context_padding) as usize).min(lines.len());
    if start >= end {
        return None;
    }
    let mut out = String::new();
    for (i, line) in lines[start..end].iter().enumerate() {
        let lineno = start + 1 + i;
        out.push_str(&format!("{lineno}\t{line}\n"));
    }
    Some(out)
}

/// Rank clusters for inclusion under a per-file character budget.
/// Returns indices into the input slice, ordered by descending priority.
///
/// Ordering keys (first wins):
///   1. `max_importance` — entry-point cluster outranks declaration block
///   2. density (`score / span`) — focused clusters over sprawling ones
///   3. raw score — total importance
///   4. ascending span — cheaper to include
pub fn rank_clusters_for_inclusion(clusters: &[Cluster]) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..clusters.len()).collect();
    idx.sort_by(|a, b| {
        let ca = &clusters[*a];
        let cb = &clusters[*b];
        cb.max_importance.cmp(&ca.max_importance).then_with(|| {
            let span_a = (ca.end.saturating_sub(ca.start) + 1) as f32;
            let span_b = (cb.end.saturating_sub(cb.start) + 1) as f32;
            let density_a = ca.score as f32 / span_a;
            let density_b = cb.score as f32 / span_b;
            density_b
                .partial_cmp(&density_a)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| cb.score.cmp(&ca.score))
                .then_with(|| {
                    span_a
                        .partial_cmp(&span_b)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
        })
    });
    idx
}

/// Build a compact "file header" line listing the symbols inside —
/// dedup by name, sort by frequency descending, cap at `max_in_header`
/// with a trailing `+N more` if truncated.
pub fn build_file_header(symbols: &[String], max_in_header: usize) -> String {
    use std::collections::HashMap as HMap;
    let mut counts: HMap<&String, u32> = HMap::new();
    for s in symbols {
        *counts.entry(s).or_insert(0) += 1;
    }
    let mut by_freq: Vec<(&String, u32)> = counts.into_iter().collect();
    by_freq.sort_by_key(|b| std::cmp::Reverse(b.1));
    let total = by_freq.len();
    let kept: Vec<String> = by_freq
        .iter()
        .take(max_in_header)
        .map(|(s, _)| (*s).clone())
        .collect();
    if total > max_in_header {
        format!("{}, +{} more", kept.join(", "), total - max_in_header)
    } else {
        kept.join(", ")
    }
}

/// Outline a container node (struct/enum) as a compact member list,
/// avoiding the wall-of-source that a 1k-line struct's full body
/// produces. Returns `None` for non-container kinds.
pub fn outline_container(node: &NodeData) -> Option<String> {
    if !is_envelope_kind(node.kind) {
        return None;
    }
    let fields = node
        .metadata
        .get("fields")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|e| e.get("name").and_then(|n| n.as_str()).map(str::to_string))
                .collect::<Vec<_>>()
        });
    let variants = node
        .metadata
        .get("variants")
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|v| v.as_array().cloned())
        .map(|arr| {
            arr.into_iter()
                .filter_map(|e| e.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        });
    let mut lines = vec![format!("**{} ({:?})**", node.name, node.kind)];
    if let Some(fs) = fields
        && !fs.is_empty()
    {
        lines.push(format!("fields ({}): {}", fs.len(), fs.join(", ")));
    }
    if let Some(vs) = variants
        && !vs.is_empty()
    {
        lines.push(format!("variants ({}): {}", vs.len(), vs.join(", ")));
    }
    if lines.len() == 1 {
        return None;
    }
    Some(lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

    fn span_range(start: u32, end: u32) -> Span {
        Span {
            file: PathBuf::from("src/lib.rs"),
            start_line: start,
            start_col: 0,
            end_line: end,
            end_col: 0,
            byte_range: 0..1,
        }
    }

    fn node_with_span(name: &str, kind: NodeKind, start: u32, end: u32) -> NodeData {
        let id = NodeId::new("src/lib.rs", &format!("crate::{name}"), kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: format!("crate::{name}"),
            file_path: PathBuf::from("src/lib.rs"),
            span: span_range(start, end),
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
    fn clusters_merge_within_gap() {
        let ranges = vec![
            Range {
                start: 10,
                end: 20,
                name: "a".into(),
                kind: NodeKind::Function,
                importance: 1,
            },
            Range {
                start: 25,
                end: 30,
                name: "b".into(),
                kind: NodeKind::Function,
                importance: 1,
            },
            Range {
                start: 100,
                end: 110,
                name: "c".into(),
                kind: NodeKind::Function,
                importance: 1,
            },
        ];
        // gap=10: a+b merge (25 - 20 = 5), c stands alone (100 - 30 = 70).
        let clusters = build_clusters(&ranges, 10);
        assert_eq!(clusters.len(), 2);
        assert_eq!(clusters[0].start, 10);
        assert_eq!(clusters[0].end, 30);
        assert_eq!(clusters[1].start, 100);
    }

    #[test]
    fn ranking_prefers_entry_point_clusters() {
        let clusters = vec![
            Cluster {
                start: 1,
                end: 100,
                symbols: vec!["a(fn)".into(); 5],
                score: 5,
                max_importance: 1, // pile of low-priority
            },
            Cluster {
                start: 200,
                end: 210,
                symbols: vec!["b(fn)".into()],
                score: 10,
                max_importance: 10, // a single entry point
            },
        ];
        let ranked = rank_clusters_for_inclusion(&clusters);
        assert_eq!(ranked[0], 1); // entry-point cluster wins
    }

    #[test]
    fn build_ranges_drops_envelope_containers() {
        let mut g = CodeGraph::new();
        let big = g.add_node(node_with_span("Big", NodeKind::Struct, 1, 600));
        let small = g.add_node(node_with_span("small", NodeKind::Function, 100, 110));
        let roots = HashSet::new();
        let ranges = build_ranges(&g, &[big, small], &roots, 1000);
        // Big struct covers 600/1000 → dropped.
        assert_eq!(ranges.len(), 1);
        assert_eq!(ranges[0].name, "small");
    }

    #[test]
    fn build_ranges_keeps_small_struct() {
        let mut g = CodeGraph::new();
        let small_struct = g.add_node(node_with_span("S", NodeKind::Struct, 1, 30));
        let roots = HashSet::new();
        let ranges = build_ranges(&g, &[small_struct], &roots, 1000);
        assert_eq!(ranges.len(), 1);
    }

    #[test]
    fn file_header_caps_and_summarizes() {
        let symbols: Vec<String> = (0..20).map(|i| format!("sym{i}")).collect();
        let header = build_file_header(&symbols, 5);
        assert!(header.contains("+15 more"));
    }

    #[test]
    fn outline_container_lists_fields() {
        let mut node = node_with_span("Config", NodeKind::Struct, 1, 50);
        node.metadata.insert(
            "fields".to_string(),
            r#"[{"name":"alpha"},{"name":"beta"}]"#.to_string(),
        );
        let outline = outline_container(&node).expect("outline");
        assert!(outline.contains("alpha"));
        assert!(outline.contains("beta"));
    }

    #[test]
    fn outline_function_returns_none() {
        let node = node_with_span("f", NodeKind::Function, 1, 10);
        assert!(outline_container(&node).is_none());
    }
}
