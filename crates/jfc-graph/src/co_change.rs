//! Co-change analysis: temporal coupling from git history.
//!
//! Identifies functions/files that frequently change together across commits,
//! revealing hidden coupling not visible in the static call graph. The core
//! algorithm:
//!
//! 1. Parse `git log --name-only --format="%H"` output into commit records.
//! 2. Map file paths from each commit to NodeIds in the code graph.
//! 3. Build a co-occurrence matrix: for each pair of nodes appearing in the
//!    same commit, increment their shared counter.
//! 4. Compute confidence = times_together / max(total_a, total_b).
//! 5. Filter by minimum support threshold and sort by confidence descending.

use std::collections::HashMap;
use std::path::Path;

use crate::graph::CodeGraph;
use crate::nodes::NodeId;

/// Result of co-change analysis.
#[derive(Debug, Clone)]
pub struct CoChangeResult {
    pub pairs: Vec<CoChangePair>,
}

/// A pair of nodes that co-change with measured coupling strength.
#[derive(Debug, Clone)]
pub struct CoChangePair {
    pub node_a: NodeId,
    pub node_b: NodeId,
    /// Number of commits where both nodes' files changed together.
    pub times_changed_together: u32,
    /// Total commits where node_a's file changed.
    pub total_changes_a: u32,
    /// Total commits where node_b's file changed.
    pub total_changes_b: u32,
    /// Coupling confidence: times_together / max(total_a, total_b).
    /// Range [0.0, 1.0] — higher means stronger temporal coupling.
    pub confidence: f64,
}

/// A single commit's metadata extracted from git log.
#[derive(Debug, Clone)]
pub struct CommitInfo {
    pub hash: String,
    pub files: Vec<String>,
}

/// Parse git log output (from `git log --name-only --format="%H"`) into commits.
///
/// The expected format is blocks separated by empty lines, where each block
/// has the commit hash on the first line followed by file paths:
///
/// ```text
/// abc123def
/// src/foo.rs
/// src/bar.rs
///
/// def456abc
/// src/baz.rs
/// ```
pub fn parse_git_log(output: &str) -> Vec<CommitInfo> {
    let mut commits = Vec::new();
    let mut current_hash: Option<String> = None;
    let mut current_files: Vec<String> = Vec::new();

    for line in output.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            // End of a commit block — flush if we have accumulated data.
            if let Some(hash) = current_hash.take() {
                if !current_files.is_empty() {
                    commits.push(CommitInfo {
                        hash,
                        files: std::mem::take(&mut current_files),
                    });
                }
            }
            continue;
        }

        if current_hash.is_none() {
            // First non-empty line of a block is the commit hash.
            current_hash = Some(trimmed.to_string());
        } else {
            // Subsequent lines are file paths.
            current_files.push(trimmed.to_string());
        }
    }

    // Flush the final block (git log output may not end with a blank line).
    if let Some(hash) = current_hash.take() {
        if !current_files.is_empty() {
            commits.push(CommitInfo {
                hash,
                files: current_files,
            });
        }
    }

    commits
}

/// Compute co-change pairs from commit history and a code graph.
///
/// `min_support`: minimum number of co-occurrences to include a pair in the
/// result. Setting this to 2+ filters out one-off coincidences.
pub fn compute_co_changes(
    graph: &CodeGraph,
    commits: &[CommitInfo],
    min_support: u32,
) -> CoChangeResult {
    if commits.is_empty() {
        return CoChangeResult { pairs: Vec::new() };
    }

    // Build a lookup: file path (normalized string) → Vec<NodeId>.
    // Multiple nodes can live in the same file.
    let file_to_nodes = build_file_to_nodes_map(graph);

    // Track per-node total change count.
    let mut node_changes: HashMap<NodeId, u32> = HashMap::new();
    // Track co-occurrence: (node_a, node_b) → count, with a < b ordering
    // on NodeId to avoid double-counting.
    let mut co_occurrences: HashMap<(NodeId, NodeId), u32> = HashMap::new();

    for commit in commits {
        // Map commit file paths to node IDs.
        let mut commit_nodes: Vec<NodeId> = Vec::new();
        for file_path in &commit.files {
            if let Some(nodes) = file_to_nodes.get(file_path.as_str()) {
                commit_nodes.extend(nodes.iter().cloned());
            }
        }

        // Deduplicate nodes within this commit.
        commit_nodes.sort();
        commit_nodes.dedup();

        // Increment per-node change counter.
        for node in &commit_nodes {
            *node_changes.entry(node.clone()).or_insert(0) += 1;
        }

        // Increment co-occurrence for all pairs (ordered by NodeId).
        let n = commit_nodes.len();
        for i in 0..n {
            for j in (i + 1)..n {
                let key = (commit_nodes[i].clone(), commit_nodes[j].clone());
                *co_occurrences.entry(key).or_insert(0) += 1;
            }
        }
    }

    // Build result pairs, filtering by min_support.
    let mut pairs: Vec<CoChangePair> = co_occurrences
        .into_iter()
        .filter(|(_, count)| *count >= min_support)
        .map(|((node_a, node_b), times_together)| {
            let total_a = node_changes.get(&node_a).copied().unwrap_or(0);
            let total_b = node_changes.get(&node_b).copied().unwrap_or(0);
            let max_total = total_a.max(total_b);
            let confidence = if max_total > 0 {
                times_together as f64 / max_total as f64
            } else {
                0.0
            };
            CoChangePair {
                node_a,
                node_b,
                times_changed_together: times_together,
                total_changes_a: total_a,
                total_changes_b: total_b,
                confidence,
            }
        })
        .collect();

    // Sort by confidence descending, break ties by times_changed_together descending.
    pairs.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.times_changed_together.cmp(&a.times_changed_together))
    });

    CoChangeResult { pairs }
}

/// For a given set of seed NodeIds, compute co-change pairs where at least
/// one member of the pair is in the seed set.
///
/// This is the query-facing function: "given these nodes, what else changes
/// with them?" Filters the full co-change result to pairs touching the seed.
pub fn co_changes_for_nodes(
    graph: &CodeGraph,
    commits: &[CommitInfo],
    seed_nodes: &[NodeId],
    min_support: u32,
) -> CoChangeResult {
    let full = compute_co_changes(graph, commits, min_support);

    let seed_set: std::collections::HashSet<&NodeId> = seed_nodes.iter().collect();
    let pairs: Vec<CoChangePair> = full
        .pairs
        .into_iter()
        .filter(|p| seed_set.contains(&p.node_a) || seed_set.contains(&p.node_b))
        .collect();

    CoChangeResult { pairs }
}

/// Build a mapping from normalized file path strings to node IDs in the graph.
fn build_file_to_nodes_map(graph: &CodeGraph) -> HashMap<&str, Vec<NodeId>> {
    let mut map: HashMap<&str, Vec<NodeId>> = HashMap::new();
    for id in graph.all_node_ids() {
        if let Some(node) = graph.get_node(id) {
            let file_str = node.file_path.to_str().unwrap_or("");
            if !file_str.is_empty() {
                map.entry(file_str).or_default().push(id.clone());
            }
        }
    }
    map
}

/// Shell out to `git log` and parse the result. Returns an empty vec if
/// git is unavailable or the directory is not a git repository.
///
/// `workspace_root`: the directory from which to run `git log`.
/// `max_commits`: cap on how many commits to fetch (performance guard).
pub fn fetch_git_history(workspace_root: &Path, max_commits: usize) -> Vec<CommitInfo> {
    let output = std::process::Command::new("git")
        .args([
            "log",
            "--name-only",
            "--format=%H",
            &format!("-n{}", max_commits),
        ])
        .current_dir(workspace_root)
        .output();

    match output {
        Ok(o) if o.status.success() => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            parse_git_log(&stdout)
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::edges::{EdgeData, EdgeKind};
    use crate::nodes::{NodeData, NodeKind, Span, Visibility};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_node(name: &str, file: &str) -> NodeData {
        NodeData {
            id: NodeId::new(file, name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from(file),
            span: Span {
                file: PathBuf::from(file),
                start_line: 1,
                start_col: 0,
                end_line: 10,
                end_col: 1,
                byte_range: 0..100,
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

    fn build_test_graph() -> CodeGraph {
        let mut graph = CodeGraph::new();
        let node_a = make_node("foo", "src/foo.rs");
        let node_b = make_node("bar", "src/bar.rs");
        let node_c = make_node("baz", "src/baz.rs");
        graph.add_node(node_a);
        graph.add_node(node_b);
        graph.add_node(node_c);
        // Add a call edge so the graph isn't trivially empty.
        let id_a = NodeId::new("src/foo.rs", "foo", NodeKind::Function);
        let id_b = NodeId::new("src/bar.rs", "bar", NodeKind::Function);
        let _ = graph.add_edge(
            &id_a,
            &id_b,
            EdgeData {
                kind: EdgeKind::Calls,
                source_span: Span {
                    file: PathBuf::from("src/foo.rs"),
                    start_line: 1,
                    start_col: 0,
                    end_line: 1,
                    end_col: 10,
                    byte_range: 0..10,
                },
                weight: 1.0,
            },
        );
        graph
    }

    #[test]
    fn test_parse_git_log_basic() {
        let input = "\
abc123
src/foo.rs
src/bar.rs

def456
src/baz.rs
";
        let commits = parse_git_log(input);
        assert_eq!(commits.len(), 2);
        assert_eq!(commits[0].hash, "abc123");
        assert_eq!(commits[0].files, vec!["src/foo.rs", "src/bar.rs"]);
        assert_eq!(commits[1].hash, "def456");
        assert_eq!(commits[1].files, vec!["src/baz.rs"]);
    }

    #[test]
    fn test_parse_git_log_no_trailing_newline() {
        let input = "abc123\nsrc/main.rs";
        let commits = parse_git_log(input);
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].hash, "abc123");
        assert_eq!(commits[0].files, vec!["src/main.rs"]);
    }

    #[test]
    fn test_parse_git_log_empty() {
        let commits = parse_git_log("");
        assert!(commits.is_empty());
    }

    #[test]
    fn test_parse_git_log_hash_only_no_files() {
        let input = "abc123\n\ndef456\nsrc/a.rs\n";
        let commits = parse_git_log(input);
        // First block has no files, so it's skipped.
        assert_eq!(commits.len(), 1);
        assert_eq!(commits[0].hash, "def456");
    }

    #[test]
    fn test_compute_co_changes_basic() {
        let graph = build_test_graph();
        let commits = vec![
            CommitInfo {
                hash: "c1".into(),
                files: vec!["src/foo.rs".into(), "src/bar.rs".into()],
            },
            CommitInfo {
                hash: "c2".into(),
                files: vec!["src/foo.rs".into(), "src/bar.rs".into()],
            },
            CommitInfo {
                hash: "c3".into(),
                files: vec!["src/foo.rs".into(), "src/baz.rs".into()],
            },
        ];

        let result = compute_co_changes(&graph, &commits, 1);
        assert!(!result.pairs.is_empty());

        // foo and bar co-change 2 times.
        let id_foo = NodeId::new("src/foo.rs", "foo", NodeKind::Function);
        let id_bar = NodeId::new("src/bar.rs", "bar", NodeKind::Function);
        let foo_bar = result.pairs.iter().find(|p| {
            (p.node_a == id_foo && p.node_b == id_bar) || (p.node_a == id_bar && p.node_b == id_foo)
        });
        assert!(foo_bar.is_some(), "foo-bar pair should exist");
        let fb = foo_bar.unwrap();
        assert_eq!(fb.times_changed_together, 2);
    }

    #[test]
    fn test_confidence_calculation() {
        let graph = build_test_graph();
        let commits = vec![
            CommitInfo {
                hash: "c1".into(),
                files: vec!["src/foo.rs".into(), "src/bar.rs".into()],
            },
            CommitInfo {
                hash: "c2".into(),
                files: vec!["src/foo.rs".into(), "src/bar.rs".into()],
            },
            CommitInfo {
                hash: "c3".into(),
                files: vec!["src/foo.rs".into()],
            },
        ];

        let result = compute_co_changes(&graph, &commits, 1);
        let id_foo = NodeId::new("src/foo.rs", "foo", NodeKind::Function);
        let id_bar = NodeId::new("src/bar.rs", "bar", NodeKind::Function);
        let pair = result
            .pairs
            .iter()
            .find(|p| {
                (p.node_a == id_foo && p.node_b == id_bar)
                    || (p.node_a == id_bar && p.node_b == id_foo)
            })
            .expect("pair should exist");

        // foo changes 3 times, bar changes 2 times, together 2 times.
        // confidence = 2 / max(3, 2) = 2/3 ≈ 0.667
        assert_eq!(pair.times_changed_together, 2);
        assert!((pair.confidence - 2.0 / 3.0).abs() < 1e-10);
    }

    #[test]
    fn test_min_support_filtering() {
        let graph = build_test_graph();
        let commits = vec![
            CommitInfo {
                hash: "c1".into(),
                files: vec!["src/foo.rs".into(), "src/bar.rs".into()],
            },
            CommitInfo {
                hash: "c2".into(),
                files: vec!["src/foo.rs".into(), "src/baz.rs".into()],
            },
        ];

        // With min_support=2, only pairs appearing together >= 2 times survive.
        let result = compute_co_changes(&graph, &commits, 2);
        assert!(
            result.pairs.is_empty(),
            "no pair co-occurs twice in this history"
        );

        // With min_support=1, both pairs appear.
        let result = compute_co_changes(&graph, &commits, 1);
        assert_eq!(result.pairs.len(), 2);
    }

    #[test]
    fn test_empty_history() {
        let graph = build_test_graph();
        let result = compute_co_changes(&graph, &[], 1);
        assert!(result.pairs.is_empty());
    }

    #[test]
    fn test_co_changes_for_nodes_filters_to_seed() {
        let graph = build_test_graph();
        let id_foo = NodeId::new("src/foo.rs", "foo", NodeKind::Function);
        let id_bar = NodeId::new("src/bar.rs", "bar", NodeKind::Function);
        let id_baz = NodeId::new("src/baz.rs", "baz", NodeKind::Function);

        let commits = vec![
            CommitInfo {
                hash: "c1".into(),
                files: vec!["src/foo.rs".into(), "src/bar.rs".into()],
            },
            CommitInfo {
                hash: "c2".into(),
                files: vec![
                    "src/foo.rs".into(),
                    "src/bar.rs".into(),
                    "src/baz.rs".into(),
                ],
            },
            CommitInfo {
                hash: "c3".into(),
                files: vec!["src/bar.rs".into(), "src/baz.rs".into()],
            },
        ];

        // Only ask for co-changes relative to foo.
        let result = co_changes_for_nodes(&graph, &commits, std::slice::from_ref(&id_foo), 1);
        // Every returned pair must include foo.
        for pair in &result.pairs {
            assert!(
                pair.node_a == id_foo || pair.node_b == id_foo,
                "pair should include the seed node"
            );
        }
        // The bar-baz pair (which doesn't include foo) should NOT appear.
        let bar_baz = result.pairs.iter().find(|p| {
            (p.node_a == id_bar && p.node_b == id_baz) || (p.node_a == id_baz && p.node_b == id_bar)
        });
        assert!(bar_baz.is_none(), "bar-baz pair should not appear");
    }

    #[test]
    fn test_sort_by_confidence_descending() {
        let graph = build_test_graph();
        let commits = vec![
            CommitInfo {
                hash: "c1".into(),
                files: vec![
                    "src/foo.rs".into(),
                    "src/bar.rs".into(),
                    "src/baz.rs".into(),
                ],
            },
            CommitInfo {
                hash: "c2".into(),
                files: vec!["src/foo.rs".into(), "src/bar.rs".into()],
            },
            CommitInfo {
                hash: "c3".into(),
                files: vec!["src/foo.rs".into()],
            },
        ];

        let result = compute_co_changes(&graph, &commits, 1);
        // Verify confidence is in descending order.
        for w in result.pairs.windows(2) {
            assert!(
                w[0].confidence >= w[1].confidence,
                "pairs should be sorted by confidence descending"
            );
        }
    }

    #[test]
    fn test_files_not_in_graph_are_ignored() {
        let graph = build_test_graph();
        let commits = vec![CommitInfo {
            hash: "c1".into(),
            files: vec!["src/unknown.rs".into(), "src/also_unknown.rs".into()],
        }];

        let result = compute_co_changes(&graph, &commits, 1);
        assert!(
            result.pairs.is_empty(),
            "files not in graph produce no pairs"
        );
    }

    #[test]
    fn test_multiple_nodes_same_file() {
        // Two nodes in the same file should co-change when that file changes.
        let mut graph = CodeGraph::new();
        let node_a = make_node("alpha", "src/shared.rs");
        let node_b = make_node("beta", "src/shared.rs");
        graph.add_node(node_a);
        graph.add_node(node_b);

        let commits = vec![
            CommitInfo {
                hash: "c1".into(),
                files: vec!["src/shared.rs".into()],
            },
            CommitInfo {
                hash: "c2".into(),
                files: vec!["src/shared.rs".into()],
            },
        ];

        let result = compute_co_changes(&graph, &commits, 1);
        assert_eq!(result.pairs.len(), 1);
        assert_eq!(result.pairs[0].times_changed_together, 2);
        // Both changed 2 times total, confidence = 2/2 = 1.0
        assert!((result.pairs[0].confidence - 1.0).abs() < 1e-10);
    }
}
