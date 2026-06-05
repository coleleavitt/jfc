//! Persistent content index for `graph_grep`.
//!
//! Two caches, both keyed by file path, both validated against the file's
//! modification time so a stale entry is silently refreshed:
//!
//! 1. **Line cache** — the file's lines, read once and reused across grep
//!    calls. Without it, every `graph_grep` re-read every indexed file from
//!    disk (the limitation flagged in the search→sed fix).
//!
//! 2. **Symbol-span index** — a per-file, start-line-sorted list of
//!    `(start, end, name)` spans for Function/Struct nodes. Enclosing-symbol
//!    lookup becomes a binary search instead of an O(N) scan over *every*
//!    graph node for *every* match (the old `enclosing_symbol` was O(M×N)).
//!
//! The index uses `DashMap` for interior mutability through `&self`, matching
//! the `QueryCache` pattern in [`crate::incremental`], so it slots into the
//! `Arc<GraphSession>` read path without a `&mut` borrow.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use dashmap::DashMap;

use crate::graph::CodeGraph;
use crate::nodes::NodeKind;

/// One file's cached lines plus the mtime they were read at.
struct LineEntry {
    mtime: Option<SystemTime>,
    lines: Arc<Vec<String>>,
}

/// A symbol span used for enclosing-symbol resolution. Sorted by `start`.
#[derive(Clone)]
struct SymbolSpan {
    start: u32,
    end: u32,
    name: String,
}

/// One file's symbol spans plus the graph revision they were built at.
struct SpanEntry {
    revision: u64,
    spans: Arc<Vec<SymbolSpan>>,
}

/// Caches file content + symbol spans for fast repeated content search.
#[derive(Default)]
pub struct ContentIndex {
    lines: DashMap<PathBuf, LineEntry>,
    spans: DashMap<PathBuf, SpanEntry>,
}

impl ContentIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Cached file lines, refreshed when the on-disk mtime changes.
    /// Returns `None` if the file can't be read.
    pub fn lines(&self, path: &Path) -> Option<Arc<Vec<String>>> {
        let disk_mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();

        if let Some(entry) = self.lines.get(path)
            && entry.mtime == disk_mtime
        {
            return Some(Arc::clone(&entry.lines));
        }

        // Miss or stale — read and cache.
        let content = std::fs::read_to_string(path).ok()?;
        let lines: Arc<Vec<String>> = Arc::new(content.lines().map(str::to_owned).collect());
        self.lines.insert(
            path.to_path_buf(),
            LineEntry {
                mtime: disk_mtime,
                lines: Arc::clone(&lines),
            },
        );
        Some(lines)
    }

    /// Cached lines `start..=end` (1-indexed, inclusive) of `file`, returned
    /// as owned strings. Used by the body-rendering read paths (`graph_node`,
    /// `graph_search include_code`, `graph_explore`) so they share the same
    /// mtime-validated cache as `graph_grep` instead of re-reading from disk.
    /// Returns `None` if the file is unreadable or the range is degenerate.
    pub fn span_lines(&self, file: &Path, start: u32, end: u32) -> Option<Vec<String>> {
        let lines = self.lines(file)?;
        let lo = start.saturating_sub(1) as usize;
        let hi = (end as usize).min(lines.len());
        if lo >= hi {
            return None;
        }
        Some(lines[lo..hi].to_vec())
    }

    /// Innermost enclosing Function/Struct symbol at `line` in `file`, using
    /// a cached, start-sorted span list (binary search). `graph` is consulted
    /// only on a cache miss or when the graph revision advanced.
    pub fn enclosing_symbol(&self, graph: &CodeGraph, file: &Path, line: u32) -> Option<String> {
        let spans = self.spans_for(graph, file);
        // `spans` is sorted by `start`. Among all spans whose
        // [start, end] contains `line`, the innermost is the one with the
        // largest `start` (equivalently the smallest span, since they nest).
        // Walk the prefix with `start <= line` from the back.
        let upper = spans.partition_point(|s| s.start <= line);
        spans[..upper]
            .iter()
            .rev()
            .filter(|s| s.end >= line)
            .min_by_key(|s| s.end.saturating_sub(s.start))
            .map(|s| s.name.clone())
    }

    /// Get (or build) the start-sorted span list for `file`.
    fn spans_for(&self, graph: &CodeGraph, file: &Path) -> Arc<Vec<SymbolSpan>> {
        let rev = graph.current_revision();
        if let Some(entry) = self.spans.get(file)
            && entry.revision == rev
        {
            return Arc::clone(&entry.spans);
        }

        let mut spans: Vec<SymbolSpan> = graph
            .all_node_ids()
            .iter()
            .filter_map(|id| graph.get_node(id))
            .filter(|n| {
                n.file_path == file && matches!(n.kind, NodeKind::Function | NodeKind::Struct)
            })
            .map(|n| SymbolSpan {
                start: n.span.start_line,
                end: n.span.end_line,
                name: n.name.clone(),
            })
            .collect();
        spans.sort_by_key(|s| s.start);
        let spans = Arc::new(spans);
        self.spans.insert(
            file.to_path_buf(),
            SpanEntry {
                revision: rev,
                spans: Arc::clone(&spans),
            },
        );
        spans
    }

    /// Drop cached state for one file (called when a file changes). The span
    /// cache is also revision-gated, so this is belt-and-suspenders for the
    /// line cache whose validity is mtime-based.
    pub fn invalidate(&self, file: &Path) {
        self.lines.remove(file);
        self.spans.remove(file);
    }

    /// Number of files with cached lines (diagnostics / tests).
    pub fn cached_file_count(&self) -> usize {
        self.lines.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lines_cache_hits_without_rereading() {
        let dir = std::env::temp_dir().join(format!("jfc-ci-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("a.txt");
        std::fs::write(&f, "one\ntwo\nthree\n").unwrap();

        let idx = ContentIndex::new();
        let first = idx.lines(&f).unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(idx.cached_file_count(), 1);

        // Second call returns the same Arc (cache hit).
        let second = idx.lines(&f).unwrap();
        assert!(
            Arc::ptr_eq(&first, &second),
            "cache should return the same Arc"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn lines_cache_refreshes_on_mtime_change() {
        let dir = std::env::temp_dir().join(format!("jfc-ci-mtime-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("b.txt");
        std::fs::write(&f, "v1\n").unwrap();

        let idx = ContentIndex::new();
        let first = idx.lines(&f).unwrap();
        assert_eq!(first.as_slice(), &["v1".to_string()]);

        // Rewrite with a guaranteed-later mtime (sleep covers coarse FS
        // timestamp resolution).
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&f, "v1\nv2\n").unwrap();

        let second = idx.lines(&f).unwrap();
        assert_eq!(
            second.len(),
            2,
            "stale entry should refresh after mtime change"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn invalidate_drops_entry() {
        let dir = std::env::temp_dir().join(format!("jfc-ci-inval-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("c.txt");
        std::fs::write(&f, "x\n").unwrap();

        let idx = ContentIndex::new();
        idx.lines(&f).unwrap();
        assert_eq!(idx.cached_file_count(), 1);
        idx.invalidate(&f);
        assert_eq!(idx.cached_file_count(), 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
