//! Coverage integration — parse LCOV output and annotate graph nodes.
//!
//! Parses `lcov.info` (or any LCOV-format file) and annotates every
//! [`NodeKind::Function`] node with:
//!
//! - `metadata["coverage_count"]` — total hit count across all lines in the
//!   function's span. `"0"` means the function was never executed.
//! - `metadata["coverage_tested"]` — `"true"` if `coverage_count > 0`,
//!   `"false"` otherwise.
//!
//! # LCOV format (subset we parse)
//!
//! ```text
//! SF:<source_file>
//! DA:<line_number>,<execution_count>
//! end_of_record
//! ```
//!
//! We only care about `SF` (source file), `DA` (line data), and
//! `end_of_record`. Everything else (`FN`, `FNDA`, `LF`, `LH`, `BRF`,
//! `BRH`, etc.) is ignored.

use std::collections::HashMap;
use std::io::BufRead;
use std::path::{Path, PathBuf};

use crate::graph::CodeGraph;
use crate::nodes::NodeKind;
use crate::pass::{GraphFlag, Pass, PassError};

/// Per-file line coverage: line_number → execution_count.
type LineCoverage = HashMap<u32, u64>;

/// Parsed LCOV data: source_file → line coverage.
#[derive(Debug, Default)]
pub struct LcovData {
    pub files: HashMap<PathBuf, LineCoverage>,
}

/// Parse an LCOV-format reader into structured coverage data.
///
/// Tolerant: unknown record types are silently skipped. Malformed `DA`
/// lines (wrong field count, non-numeric values) are skipped with a
/// warning count returned alongside the data.
pub fn parse_lcov<R: BufRead>(reader: R) -> (LcovData, usize) {
    let mut data = LcovData::default();
    let mut current_file: Option<PathBuf> = None;
    let mut warnings = 0;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => {
                warnings += 1;
                continue;
            }
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(sf) = line.strip_prefix("SF:") {
            current_file = Some(PathBuf::from(sf));
        } else if line == "end_of_record" {
            current_file = None;
        } else if let Some(da) = line.strip_prefix("DA:") {
            if let Some(ref file) = current_file {
                let parts: Vec<&str> = da.splitn(3, ',').collect();
                if parts.len() >= 2 {
                    if let (Ok(line_no), Ok(count)) =
                        (parts[0].parse::<u32>(), parts[1].parse::<u64>())
                    {
                        data.files
                            .entry(file.clone())
                            .or_default()
                            .insert(line_no, count);
                    } else {
                        warnings += 1;
                    }
                } else {
                    warnings += 1;
                }
            }
        }
        // All other record types (FN, FNDA, LF, LH, BRF, BRH, etc.) ignored.
    }

    (data, warnings)
}

/// Annotate every `Function` node in the graph with coverage data.
///
/// For each function, sums `DA` hit counts for every line in the
/// function's `[start_line, end_line]` span. Sets:
/// - `metadata["coverage_count"]` = total hits (string)
/// - `metadata["coverage_tested"]` = `"true"` / `"false"`
///
/// `project_root` is used to resolve LCOV source paths (which are
/// typically absolute or relative to the build dir) against the graph's
/// relative `file_path` entries.
///
/// Returns `(annotated_count, untested_count)`.
pub fn annotate_graph_from_lcov(
    graph: &mut CodeGraph,
    lcov: &LcovData,
    project_root: &Path,
) -> (usize, usize) {
    let function_ids: Vec<_> = graph
        .nodes_by_kind(NodeKind::Function)
        .iter()
        .map(|n| {
            (
                n.id.clone(),
                n.file_path.clone(),
                n.span.start_line,
                n.span.end_line,
            )
        })
        .collect();

    let mut annotated = 0;
    let mut untested = 0;

    for (id, file_path, start_line, end_line) in &function_ids {
        let total_hits = coverage_for_span(lcov, project_root, file_path, *start_line, *end_line);
        let tested = total_hits > 0;

        graph.update_node_metadata(&id, |meta| {
            meta.insert("coverage_count".into(), total_hits.to_string());
            meta.insert("coverage_tested".into(), tested.to_string());
        });

        annotated += 1;
        if !tested {
            untested += 1;
        }
    }

    (annotated, untested)
}

/// Sum DA hit counts for lines within `[start_line, end_line]` of the
/// given file. Tries multiple path resolution strategies:
/// 1. Exact match on `file_path` as-is
/// 2. `project_root / file_path`
/// 3. Suffix match (LCOV path ends with the graph's relative path)
fn coverage_for_span(
    lcov: &LcovData,
    project_root: &Path,
    file_path: &Path,
    start_line: u32,
    end_line: u32,
) -> u64 {
    let line_cov = resolve_file_coverage(lcov, project_root, file_path);
    let Some(line_cov) = line_cov else {
        return 0;
    };

    let mut total = 0u64;
    for line in start_line..=end_line {
        if let Some(&count) = line_cov.get(&line) {
            total = total.saturating_add(count);
        }
    }
    total
}

/// Try to find the LCOV file entry matching a graph node's `file_path`.
fn resolve_file_coverage<'a>(
    lcov: &'a LcovData,
    project_root: &Path,
    file_path: &Path,
) -> Option<&'a LineCoverage> {
    // Strategy 1: exact match.
    if let Some(cov) = lcov.files.get(file_path) {
        return Some(cov);
    }

    // Strategy 2: project_root / file_path.
    let abs = project_root.join(file_path);
    if let Some(cov) = lcov.files.get(&abs) {
        return Some(cov);
    }

    // Strategy 3: suffix match — the LCOV path ends with the relative path.
    let file_str = file_path.to_string_lossy();
    for (lcov_path, cov) in &lcov.files {
        let lcov_str = lcov_path.to_string_lossy();
        if lcov_str.ends_with(file_str.as_ref()) {
            return Some(cov);
        }
    }

    None
}

/// [`Pass`] implementation for coverage annotation.
///
/// Construct with the path to an LCOV file. The pass parses it and
/// annotates all `Function` nodes in one shot.
pub struct CoveragePass {
    lcov_path: PathBuf,
    project_root: PathBuf,
}

impl CoveragePass {
    pub fn new(lcov_path: PathBuf, project_root: PathBuf) -> Self {
        Self {
            lcov_path,
            project_root,
        }
    }
}

impl Pass for CoveragePass {
    fn name(&self) -> &'static str {
        "coverage-annotate"
    }

    fn requires(&self) -> &'static [GraphFlag] {
        &[GraphFlag::TreeParsed]
    }

    fn establishes(&self) -> &'static [GraphFlag] {
        &[GraphFlag::CoverageAnnotated]
    }

    fn run(&self, graph: &mut CodeGraph) -> Result<(), PassError> {
        let file = std::fs::File::open(&self.lcov_path).map_err(|e| PassError::Failed {
            name: self.name(),
            reason: format!("failed to open lcov file {:?}: {}", self.lcov_path, e),
        })?;
        let reader = std::io::BufReader::new(file);
        let (lcov, _warnings) = parse_lcov(reader);

        let (annotated, untested) = annotate_graph_from_lcov(graph, &lcov, &self.project_root);

        // Store summary in graph metadata for downstream consumers.
        // (We use a sentinel node-less approach — just log it.)
        tracing::info!(
            annotated,
            untested,
            tested = annotated - untested,
            "coverage pass complete"
        );

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{NodeData, NodeId, Span, Visibility};
    use std::collections::HashMap;
    use std::io::Cursor;

    fn mk_function(name: &str, file: &str, start: u32, end: u32) -> NodeData {
        NodeData {
            id: NodeId::new(file, name, NodeKind::Function),
            kind: NodeKind::Function,
            name: name.to_string(),
            qualified_name: name.to_string(),
            file_path: PathBuf::from(file),
            span: Span {
                file: PathBuf::from(file),
                start_line: start,
                start_col: 0,
                end_line: end,
                end_col: 0,
                byte_range: 0..0,
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

    const SAMPLE_LCOV: &str = "\
SF:src/main.rs
DA:1,5
DA:2,5
DA:3,0
DA:10,3
DA:11,3
DA:12,3
end_of_record
SF:src/lib.rs
DA:1,0
DA:2,0
end_of_record
";

    #[test]
    fn parse_lcov_extracts_da_records_normal() {
        let (data, warnings) = parse_lcov(Cursor::new(SAMPLE_LCOV));
        assert_eq!(warnings, 0);
        assert_eq!(data.files.len(), 2);

        let main_cov = data.files.get(Path::new("src/main.rs")).unwrap();
        assert_eq!(main_cov.get(&1), Some(&5));
        assert_eq!(main_cov.get(&3), Some(&0));
        assert_eq!(main_cov.get(&10), Some(&3));

        let lib_cov = data.files.get(Path::new("src/lib.rs")).unwrap();
        assert_eq!(lib_cov.get(&1), Some(&0));
    }

    #[test]
    fn annotate_marks_tested_and_untested_normal() {
        let mut graph = CodeGraph::new();
        // Function spanning lines 1-3 in main.rs — lines 1,2 have hits.
        graph.add_node(mk_function("tested_fn", "src/main.rs", 1, 3));
        // Function spanning lines 1-2 in lib.rs — all zeros.
        graph.add_node(mk_function("untested_fn", "src/lib.rs", 1, 2));

        let (data, _) = parse_lcov(Cursor::new(SAMPLE_LCOV));
        let (annotated, untested) =
            annotate_graph_from_lcov(&mut graph, &data, Path::new("/project"));

        assert_eq!(annotated, 2);
        assert_eq!(untested, 1);

        let tested_id = NodeId::new("src/main.rs", "tested_fn", NodeKind::Function);
        let tested_node = graph.get_node(&tested_id).unwrap();
        assert_eq!(tested_node.metadata.get("coverage_tested").unwrap(), "true");
        assert_eq!(tested_node.metadata.get("coverage_count").unwrap(), "10"); // 5+5+0

        let untested_id = NodeId::new("src/lib.rs", "untested_fn", NodeKind::Function);
        let untested_node = graph.get_node(&untested_id).unwrap();
        assert_eq!(
            untested_node.metadata.get("coverage_tested").unwrap(),
            "false"
        );
        assert_eq!(untested_node.metadata.get("coverage_count").unwrap(), "0");
    }

    #[test]
    fn annotate_suffix_match_resolves_paths_normal() {
        let mut graph = CodeGraph::new();
        graph.add_node(mk_function("f", "src/main.rs", 10, 12));

        // LCOV uses absolute paths.
        let lcov_text = "\
SF:/home/user/project/src/main.rs
DA:10,1
DA:11,2
DA:12,3
end_of_record
";
        let (data, _) = parse_lcov(Cursor::new(lcov_text));
        let (annotated, untested) =
            annotate_graph_from_lcov(&mut graph, &data, Path::new("/home/user/project"));

        assert_eq!(annotated, 1);
        assert_eq!(untested, 0);

        let id = NodeId::new("src/main.rs", "f", NodeKind::Function);
        let node = graph.get_node(&id).unwrap();
        assert_eq!(node.metadata.get("coverage_count").unwrap(), "6"); // 1+2+3
    }

    #[test]
    fn parse_lcov_tolerates_malformed_lines_robust() {
        let bad_lcov = "\
SF:src/main.rs
DA:not_a_number,5
DA:1,also_bad
DA:2,10
end_of_record
";
        let (data, warnings) = parse_lcov(Cursor::new(bad_lcov));
        assert_eq!(warnings, 2);
        let cov = data.files.get(Path::new("src/main.rs")).unwrap();
        assert_eq!(cov.len(), 1);
        assert_eq!(cov.get(&2), Some(&10));
    }

    #[test]
    fn no_coverage_data_marks_all_untested_boundary() {
        let mut graph = CodeGraph::new();
        graph.add_node(mk_function("orphan", "src/orphan.rs", 1, 10));

        let data = LcovData::default();
        let (annotated, untested) =
            annotate_graph_from_lcov(&mut graph, &data, Path::new("/project"));

        assert_eq!(annotated, 1);
        assert_eq!(untested, 1);
    }
}
