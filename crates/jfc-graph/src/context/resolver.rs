//! Qualified-name resolution with multi-language separator support.
//!
//! Accepts simple names (`foo`) plus three qualifier flavours:
//!   - dotted     `Session.request`         (TS/JS/Python)
//!   - colon-pair `stage_apply::run`        (Rust, C++, Ruby)
//!   - slash      `configurator/stage_apply` (path-ish)
//!
//! Multi-level qualifiers compose: `crate::configurator::stage_apply::run`
//! works. Rust path prefixes (`crate`, `super`, `self`) are stripped so
//! the canonical `crate::module::symbol` form resolves the same as the
//! bare `module::symbol`.
//!
//! Resolution order — last part must always equal `node.name`:
//!   1. Suffix-match against `qualified_name` (handles class-scoped methods)
//!   2. File-path containment (handles module-from-path lookups —
//!      `stage_apply::run` matches a `run` in `stage_apply.rs`)

use crate::graph::CodeGraph;
use crate::nodes::{NodeData, NodeId};

/// Rust path roots that have no file-system equivalent.
const RUST_PATH_PREFIXES: &[&str] = &["crate", "super", "self"];

/// Match outcome for one node against a (possibly qualified) symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchQuality {
    /// Last name matched but no qualifier given.
    Simple,
    /// `qualified_name` ends with the colon-joined query parts.
    QualifiedSuffix,
    /// File path contains every non-Rust-prefix container hint.
    FilePathContainment,
    /// Did not match.
    None,
}

impl MatchQuality {
    pub fn is_match(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// Check whether `node` matches `symbol`, supporting all qualifier flavours.
pub fn matches_symbol(node: &NodeData, symbol: &str) -> MatchQuality {
    if node.name == symbol {
        return MatchQuality::Simple;
    }
    if !has_qualifier(symbol) {
        return MatchQuality::None;
    }
    let parts = split_qualifier(symbol);
    if parts.len() < 2 {
        return MatchQuality::None;
    }
    let last = parts.last().copied().unwrap_or("");
    if node.name != last {
        return MatchQuality::None;
    }

    let colon_suffix = parts.join("::");
    if node.qualified_name.contains(&colon_suffix) {
        return MatchQuality::QualifiedSuffix;
    }

    let hints: Vec<&str> = parts[..parts.len() - 1]
        .iter()
        .copied()
        .filter(|p| !RUST_PATH_PREFIXES.contains(p))
        .collect();
    if hints.is_empty() {
        return MatchQuality::None;
    }

    let segments: Vec<&str> = node
        .file_path
        .to_str()
        .unwrap_or("")
        .split('/')
        .filter(|s| !s.is_empty())
        .collect();
    let all_hit = hints.iter().all(|hint| {
        segments
            .iter()
            .any(|seg| *seg == *hint || seg.rsplit_once('.').map(|(stem, _)| stem) == Some(*hint))
    });
    if all_hit {
        MatchQuality::FilePathContainment
    } else {
        MatchQuality::None
    }
}

/// Whether the query carries a qualifier (`::`, `.`, or `/`).
pub fn has_qualifier(symbol: &str) -> bool {
    symbol.contains("::") || symbol.contains('.') || symbol.contains('/')
}

/// Split the query on every supported separator, dropping empties.
pub fn split_qualifier(symbol: &str) -> Vec<&str> {
    symbol
        .split(|c: char| c == ':' || c == '.' || c == '/')
        .filter(|p| !p.is_empty())
        .collect()
}

/// The last `::` / `.` / `/`-separated segment of a qualified symbol.
pub fn last_part(symbol: &str) -> &str {
    let parts = split_qualifier(symbol);
    parts.last().copied().unwrap_or(symbol)
}

/// Resolve one symbol against the whole graph and return ranked matches.
///
/// Order:
///   1. `MatchQuality::Simple` (bare-name hit)
///   2. `MatchQuality::QualifiedSuffix` (qualified-name hit)
///   3. `MatchQuality::FilePathContainment` (path-derived module hit)
pub fn resolve_symbol(graph: &CodeGraph, symbol: &str) -> Vec<NodeId> {
    let qualified = has_qualifier(symbol);
    let mut hits: Vec<(NodeId, MatchQuality)> = Vec::new();

    let candidates: Vec<NodeId> = if qualified {
        graph
            .find_by_name(last_part(symbol))
            .into_iter()
            .map(|n| n.id.clone())
            .collect()
    } else {
        graph
            .find_by_name(symbol)
            .into_iter()
            .map(|n| n.id.clone())
            .collect()
    };

    for id in candidates {
        if let Some(node) = graph.get_node(&id) {
            let q = matches_symbol(node, symbol);
            if q.is_match() {
                hits.push((id, q));
            }
        }
    }

    hits.sort_by_key(|(_, q)| match q {
        MatchQuality::Simple => 0,
        MatchQuality::QualifiedSuffix => 1,
        MatchQuality::FilePathContainment => 2,
        MatchQuality::None => 3,
    });

    hits.into_iter().map(|(id, _)| id).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeId, NodeKind, Span, Visibility};

    fn make_node(name: &str, qualified: &str, file: &str) -> NodeData {
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
            metadata: HashMap::new(),
            birth_revision: 0,
            last_modified_revision: 0,
            complexity: None,
            cfg: None,
            dataflow: None,
        }
    }

    #[test]
    fn bare_name_simple_match() {
        let n = make_node("foo", "crate::m::foo", "src/m.rs");
        assert_eq!(matches_symbol(&n, "foo"), MatchQuality::Simple);
    }

    #[test]
    fn no_qualifier_no_match_when_name_differs() {
        let n = make_node("bar", "crate::m::bar", "src/m.rs");
        assert_eq!(matches_symbol(&n, "foo"), MatchQuality::None);
    }

    #[test]
    fn colon_qualified_suffix_match() {
        let n = make_node("run", "crate::stage_apply::run", "src/runner.rs");
        assert_eq!(
            matches_symbol(&n, "stage_apply::run"),
            MatchQuality::QualifiedSuffix
        );
    }

    #[test]
    fn dotted_qualifier_matches_via_qualified_name() {
        let n = make_node("request", "Session::request", "src/session.rs");
        assert_eq!(
            matches_symbol(&n, "Session.request"),
            MatchQuality::QualifiedSuffix
        );
    }

    #[test]
    fn slash_qualifier_matches_by_path() {
        let n = make_node("run", "crate::run", "src/stage_apply/runner.rs");
        assert_eq!(
            matches_symbol(&n, "stage_apply/run"),
            MatchQuality::FilePathContainment
        );
    }

    #[test]
    fn rust_path_prefix_is_stripped() {
        let n = make_node("run", "crate::stage_apply::run", "src/stage_apply.rs");
        assert_eq!(
            matches_symbol(&n, "crate::stage_apply::run"),
            MatchQuality::QualifiedSuffix
        );
    }

    #[test]
    fn last_part_must_equal_node_name() {
        let n = make_node("run", "crate::stage_apply::run", "src/stage_apply.rs");
        assert_eq!(matches_symbol(&n, "stage_apply::other"), MatchQuality::None);
    }

    #[test]
    fn unrelated_path_does_not_match() {
        let n = make_node("run", "crate::run", "src/other.rs");
        assert_eq!(matches_symbol(&n, "stage_apply::run"), MatchQuality::None);
    }

    #[test]
    fn file_path_stem_match_strips_extension() {
        let n = make_node("run", "crate::run", "src/stage_apply.rs");
        assert_eq!(
            matches_symbol(&n, "stage_apply::run"),
            MatchQuality::FilePathContainment
        );
    }

    #[test]
    fn last_part_helper_returns_tail() {
        assert_eq!(last_part("crate::module::sym"), "sym");
        assert_eq!(last_part("Session.request"), "request");
        assert_eq!(last_part("bare"), "bare");
    }

    #[test]
    fn has_qualifier_detects_separators() {
        assert!(has_qualifier("a::b"));
        assert!(has_qualifier("a.b"));
        assert!(has_qualifier("a/b"));
        assert!(!has_qualifier("plain"));
    }
}
