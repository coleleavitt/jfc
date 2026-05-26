//! Symbol table: maps human-readable handles to node locations for semantic editing.
//!
//! # Cycle detection for recursive resolution
//!
//! The current [`SymbolTable::resolve`] is a flat `HashMap` lookup and cannot
//! recurse. If a future resolver grows cross-symbol dependencies (e.g.,
//! resolving symbol `A` requires resolving symbol `B` which requires `A`), use
//! the [`ResolutionJob`] linked-list pattern below to detect cycles in
//! `O(depth)` without a global "currently-resolving" hashtable.
//!
//! Pattern (from rustc's query system, per Daria Sukhonina, t-compiler):
//! each in-flight resolution holds a borrow of its parent resolution job on the
//! stack. Walking the parent chain detects ancestors that match the current
//! target — that is a cycle. A new resolver should:
//!
//! 1. Take `parent: Option<&ResolutionJob>` as a parameter.
//! 2. On entry, check `if parent.map_or(false, |p| p.is_cycle(self))` and
//!    return [`SymbolError::Cycle`] with [`ResolutionJob::cycle_path`].
//! 3. Build a fresh `ResolutionJob { handle: self, parent }` and pass
//!    `Some(&job)` into recursive calls.
//!
//! The infrastructure here ([`ResolutionJob`], [`SymbolError`]) is exported and
//! ready to use; the non-recursive `resolve()` is intentionally unchanged.
//!
//! # Red-node deferred cycle recovery
//!
//! In addition to *erroring* on cycles via [`SymbolError::Cycle`], the
//! [`try_resolve_recursive`] function provides a *graceful-degradation* path
//! that returns a [`ResolutionResult::Red`] sentinel instead. Sibling to the
//! cycle-detection scaffold above, this mirrors rustc's red-green algorithm:
//! a "red" dep-node is recomputed on the next query but does **not** poison
//! its consumers, letting downstream passes choose policy (emit a "circular
//! reference" diagnostic, retry after dependents stabilize, etc.).
//!
//! Idiom (Zulip, t-compiler/query-system, Zoxc):
//!
//! > "I kind of like the idea of adding explicit query calls like
//! > `tcx.try_query().$query()` which can return `None` due to a query
//! > cycle. This gives a bit more control over where and how query cycles
//! > are recovered to the query providers themselves... I think we can get
//! > away with just making `try_query()` add a dependency a red dep node."
//!
//! [`try_resolve_recursive`] is a reusable, dependency-injected resolver: it
//! takes its dependency-lookup function as a parameter so the same cycle-safe
//! traversal serves both the production resolver and synthetic-adjacency unit
//! tests. The live cross-file resolution path runs through
//! [`crate::resolver::ReferenceResolver`] in `builder.rs`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::graph::CodeGraph;
use crate::nodes::{NodeId, NodeKind, Span};

/// Human-readable symbol handle (e.g., "fn:sample::foo", "struct:Config").
pub type SymbolHandle = String;

/// Errors produced by symbol-table operations.
#[derive(Debug, thiserror::Error)]
pub enum SymbolError {
    /// Recursive resolution detected a cycle. `path` is the chain of symbols
    /// from the outermost ancestor that matched, ending with the symbol whose
    /// resolution closed the loop.
    #[error("symbol resolution cycle: {}", format_cycle(.path))]
    Cycle { path: Vec<SymbolHandle> },
}

fn format_cycle(path: &[SymbolHandle]) -> String {
    if path.is_empty() {
        return "<empty>".to_string();
    }
    path.join(" -> ")
}

/// In-flight symbol-resolution job — a node in a stack-allocated linked list.
///
/// Each recursive resolution call constructs a `ResolutionJob` whose `parent`
/// borrows the caller's job. Walking the parent chain reveals every ancestor
/// currently being resolved, enabling `O(depth)` cycle detection.
///
/// Lifetimes:
/// - `'h` borrows the handle string from the resolver's caller.
/// - The (anonymous) lifetime on `parent` borrows the parent frame on the
///   stack; the borrow checker enforces that children cannot outlive the
///   parent job, which matches the call-stack discipline exactly.
///
/// See the module-level docs for the recommended call pattern.
#[derive(Debug, Clone, Copy)]
pub struct ResolutionJob<'h, 'p> {
    /// The symbol handle currently being resolved at this stack frame.
    pub handle: &'h str,
    /// Parent frame, or `None` for the root resolution.
    pub parent: Option<&'p ResolutionJob<'h, 'p>>,
}

impl<'h, 'p> ResolutionJob<'h, 'p> {
    /// Construct a root resolution job with no parent.
    pub fn root(handle: &'h str) -> Self {
        Self {
            handle,
            parent: None,
        }
    }

    /// Construct a child resolution job whose parent is `self`.
    ///
    /// Borrow on `self` ties the child's lifetime to the parent's stack frame.
    pub fn child<'c>(&'c self, handle: &'h str) -> ResolutionJob<'h, 'c>
    where
        'p: 'c,
    {
        ResolutionJob {
            handle,
            parent: Some(self),
        }
    }

    /// Walk the parent chain (including `self`) looking for `target`.
    /// Returns `true` if `target` appears as `self` or any ancestor — that is
    /// a cycle.
    pub fn is_cycle(&self, target: &str) -> bool {
        let mut cur: Option<&ResolutionJob<'h, '_>> = Some(self);
        while let Some(job) = cur {
            if job.handle == target {
                return true;
            }
            cur = job.parent;
        }
        false
    }

    /// Build a cycle path for diagnostic output. The returned `Vec` starts at
    /// the matching ancestor (inclusive) and ends with `target`, so the slice
    /// reads as the cycle itself: `A -> B -> ... -> A`.
    ///
    /// If `target` is not an ancestor, returns an empty `Vec`.
    pub fn cycle_path(&self, target: &str) -> Vec<SymbolHandle> {
        // Collect the parent chain, root-most first, including self.
        let mut chain: Vec<&str> = Vec::new();
        let mut cur: Option<&ResolutionJob<'h, '_>> = Some(self);
        while let Some(job) = cur {
            chain.push(job.handle);
            cur = job.parent;
        }
        chain.reverse();

        // Find the first occurrence of `target` and emit [target..end, target].
        let Some(start) = chain.iter().position(|h| *h == target) else {
            return Vec::new();
        };
        let mut path: Vec<SymbolHandle> = chain[start..].iter().map(|s| (*s).to_string()).collect();
        path.push(target.to_string());
        path
    }
}

/// Outcome of a recursive resolution attempt with red-node cycle recovery.
///
/// Marks a resolved symbol whose resolution was DEFERRED due to a detected
/// cycle. Subsequent passes can choose to:
///   - degrade gracefully (e.g., emit a "circular reference" diagnostic
///     instead of crashing)
///   - re-run after dependent symbols stabilize
///
/// This mirrors rustc's red-green algorithm: a red dep-node is recomputed on
/// the next query but doesn't poison its consumers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResolutionResult {
    /// Resolution succeeded with a stable value.
    Resolved(SymbolHandle),
    /// Resolution detected a cycle; the result is provisional.
    /// Consumers should check [`ResolutionResult::is_red`] and decide policy.
    ///
    /// `path` is the union of every cycle path produced by this subtree's
    /// dependencies (deduplicated, capped — see [`MAX_RED_PATH_LEN`]). The
    /// shape is *diagnostic*: enough to point a developer at the offending
    /// SCC without growing pathologically on dense graphs.
    Red { path: Vec<SymbolHandle> },
    /// Symbol genuinely doesn't exist (not a cycle, just unknown).
    Unknown,
}

/// Soft upper bound on the size of a [`ResolutionResult::Red`] cycle-path
/// vector. Aggregation appends new ancestors until it hits this limit;
/// further entries are silently dropped. Tuned for diagnostic legibility,
/// not for completeness — a 64-entry cycle is already pathological.
pub const MAX_RED_PATH_LEN: usize = 64;

impl ResolutionResult {
    /// `true` iff this result was produced by a detected cycle.
    pub fn is_red(&self) -> bool {
        matches!(self, Self::Red { .. })
    }

    /// Returns the resolved handle, or `None` for [`Self::Red`]/[`Self::Unknown`].
    pub fn resolved(&self) -> Option<&SymbolHandle> {
        match self {
            Self::Resolved(h) => Some(h),
            _ => None,
        }
    }
}

/// Recursive resolution with red-node deferred cycle recovery.
///
/// On detected cycle, returns [`ResolutionResult::Red`] carrying the cycle
/// path so the caller can degrade gracefully. The non-recursive
/// [`SymbolTable::resolve`] path is unaffected.
///
/// # Aggregation policy
///
/// When multiple dependencies of the current handle return
/// [`ResolutionResult::Red`], the returned `Red` *unions* their `path`
/// vectors (deduplicated, preserving first-seen order). This shows the full
/// cycle topology of the offending SCC instead of arbitrarily reporting one
/// branch. The aggregate is capped at [`MAX_RED_PATH_LEN`] to bound
/// pathological growth on dense graphs. An [`ResolutionResult::Unknown`]
/// dependency short-circuits and propagates up — a missing symbol is
/// strictly more informative than a cycle through one.
///
/// # Dependency injection
///
/// The `resolver` parameter abstracts the "look up dependencies of this
/// handle" logic so the same cycle-safe traversal serves both the production
/// resolver and unit tests over synthetic adjacency maps.
///
/// Idiom: rustc query-system "red dep nodes" (Zulip t-compiler/query-system,
/// Zoxc on explicit cycle recovery — see module docs).
pub fn try_resolve_recursive<'h, F>(
    table: &SymbolTable,
    handle: &'h str,
    parent: Option<&ResolutionJob<'h, '_>>,
    resolver: &F,
) -> ResolutionResult
where
    F: for<'a> Fn(&'h str, &'a ResolutionJob<'h, 'a>) -> Vec<&'h str>,
{
    // 1. Cycle check: if `handle` is already an ancestor on the stack, this
    //    edge would close a loop. Emit Red with the diagnostic path.
    if let Some(p) = parent {
        if p.is_cycle(handle) {
            return ResolutionResult::Red {
                path: p.cycle_path(handle),
            };
        }
    }

    // 2. Push our frame onto the linked-list stack.
    let job = match parent {
        Some(p) => p.child(handle),
        None => ResolutionJob::root(handle),
    };

    // 3. Ask the resolver for this handle's dependencies. An empty list
    //    means "leaf"; we still need to confirm the symbol *exists* for the
    //    Unknown branch — defer that to the table lookup at the bottom.
    let deps = resolver(handle, &job);

    // 4. Recurse over each dep, aggregating Red paths into a union and
    //    short-circuiting on Unknown.
    let mut red_paths: Vec<SymbolHandle> = Vec::new();
    for dep in deps {
        match try_resolve_recursive(table, dep, Some(&job), resolver) {
            ResolutionResult::Resolved(_) => {}
            ResolutionResult::Unknown => return ResolutionResult::Unknown,
            ResolutionResult::Red { path } => {
                for entry in path {
                    if red_paths.len() >= MAX_RED_PATH_LEN {
                        break;
                    }
                    if !red_paths.contains(&entry) {
                        red_paths.push(entry);
                    }
                }
            }
        }
    }

    if !red_paths.is_empty() {
        return ResolutionResult::Red { path: red_paths };
    }

    // 5. All deps clean. The handle resolves iff the table knows about it.
    //    Note: a real resolver might surface the dep-list itself from the
    //    table; here we only check existence, since dependency lookup is
    //    delegated to `resolver`.
    match table.resolve(handle) {
        Some(entry) => ResolutionResult::Resolved(entry.handle.clone()),
        None => ResolutionResult::Unknown,
    }
}

/// Entry in the symbol table mapping a handle to its location.
#[derive(Debug, Clone)]
pub struct SymbolEntry {
    pub node_id: NodeId,
    pub handle: SymbolHandle,
    pub file_path: PathBuf,
    pub span: Span,
    pub qualified_name: String,
    pub kind: NodeKind,
}

/// Symbol table: bidirectional mapping between handles and code locations.
pub struct SymbolTable {
    by_handle: HashMap<SymbolHandle, SymbolEntry>,
    by_node_id: HashMap<NodeId, SymbolHandle>,
}

impl SymbolTable {
    pub fn new() -> Self {
        Self {
            by_handle: HashMap::new(),
            by_node_id: HashMap::new(),
        }
    }

    /// Build symbol table from a CodeGraph — generates handles for all nodes.
    pub fn build_from_graph(graph: &CodeGraph) -> Self {
        let mut table = Self::new();

        for node_id in graph.all_node_ids() {
            let Some(node) = graph.get_node(node_id) else {
                continue;
            };
            table.insert_node(node_id, node);
        }

        table
    }

    /// Resolve exact handle to entry.
    pub fn resolve(&self, handle: &str) -> Option<&SymbolEntry> {
        self.by_handle.get(handle)
    }

    /// Fuzzy match: find entries where handle contains the partial string (case-insensitive).
    pub fn resolve_fuzzy(&self, partial: &str) -> Vec<&SymbolEntry> {
        let lower = partial.to_lowercase();
        self.by_handle
            .values()
            .filter(|entry| entry.handle.to_lowercase().contains(&lower))
            .collect()
    }

    /// Remove all entries for a given file (for incremental updates).
    pub fn invalidate_file(&mut self, path: &Path) {
        let handles_to_remove: Vec<SymbolHandle> = self
            .by_handle
            .values()
            .filter(|entry| entry.file_path == path)
            .map(|entry| entry.handle.clone())
            .collect();

        for handle in handles_to_remove {
            if let Some(entry) = self.by_handle.remove(&handle) {
                self.by_node_id.remove(&entry.node_id);
            }
        }
    }

    /// Rebuild entries for a single file from the graph.
    pub fn update_from_graph(&mut self, graph: &CodeGraph, changed_file: &Path) {
        self.invalidate_file(changed_file);

        for node_id in graph.all_node_ids() {
            let Some(node) = graph.get_node(node_id) else {
                continue;
            };
            if node.file_path == changed_file {
                self.insert_node(node_id, node);
            }
        }
    }

    /// Get all handles (for listing/completion).
    pub fn all_handles(&self) -> Vec<&str> {
        self.by_handle.keys().map(String::as_str).collect()
    }

    /// Get handle for a node ID.
    pub fn handle_for_node(&self, node_id: &NodeId) -> Option<&str> {
        self.by_node_id.get(node_id).map(String::as_str)
    }

    /// Total entry count.
    pub fn len(&self) -> usize {
        self.by_handle.len()
    }

    /// Returns true if the table has no entries.
    pub fn is_empty(&self) -> bool {
        self.by_handle.is_empty()
    }

    /// Insert a single node into the symbol table.
    fn insert_node(&mut self, node_id: &NodeId, node: &crate::nodes::NodeData) {
        let handle = format!("{}:{}", kind_prefix(node.kind), node.qualified_name);

        let entry = SymbolEntry {
            node_id: node_id.clone(),
            handle: handle.clone(),
            file_path: node.file_path.clone(),
            span: node.span.clone(),
            qualified_name: node.qualified_name.clone(),
            kind: node.kind,
        };

        self.by_handle.insert(handle.clone(), entry);
        self.by_node_id.insert(node_id.clone(), handle);
    }
}

impl Default for SymbolTable {
    fn default() -> Self {
        Self::new()
    }
}

/// Map NodeKind to its handle prefix.
fn kind_prefix(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Function => "fn",
        NodeKind::Struct => "struct",
        NodeKind::Enum => "enum",
        NodeKind::Module => "mod",
        NodeKind::Trait => "trait",
        NodeKind::EnumVariant => "variant",
        NodeKind::Field => "field",
        NodeKind::TypeAlias => "type",
        NodeKind::Constant => "const",
        NodeKind::Interface => "interface",
    }
}

/// Bounded Levenshtein edit distance. Returns `max_dist + 1` early when
/// the distance is known to exceed `max_dist`. O(min(len(a),len(b))) memory.
/// Compares case-folded inputs.
pub fn bounded_edit_distance(a: &str, b: &str, max_dist: usize) -> usize {
    let a = a.to_ascii_lowercase();
    let b = b.to_ascii_lowercase();
    let al = a.len();
    let bl = b.len();
    if a == b {
        return 0;
    }
    if al.abs_diff(bl) > max_dist {
        return max_dist + 1;
    }
    if al == 0 {
        return bl;
    }
    if bl == 0 {
        return al;
    }

    let mut prev: Vec<usize> = (0..=bl).collect();
    let mut cur = vec![0; bl + 1];

    for (i, ca) in a.bytes().enumerate() {
        cur[0] = i + 1;
        let mut row_min = cur[0];
        for (j, cb) in b.bytes().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            let ins = cur[j] + 1;
            let del = prev[j + 1] + 1;
            let sub = prev[j] + cost;
            cur[j + 1] = ins.min(del).min(sub);
            if cur[j + 1] < row_min {
                row_min = cur[j + 1];
            }
        }
        if row_min > max_dist {
            return max_dist + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[bl]
}

/// Fuzzy search over all node names in the graph. Returns (NodeId, distance)
/// pairs sorted by distance then name length (shorter = better).
pub fn fuzzy_search(
    graph: &crate::graph::CodeGraph,
    query: &str,
    max_dist: usize,
    limit: usize,
) -> Vec<(crate::nodes::NodeId, usize)> {
    let mut results = Vec::new();
    let lower_query = query.to_ascii_lowercase();

    for id in graph.all_node_ids() {
        let Some(node) = graph.get_node(id) else {
            continue;
        };
        let name_lower = node.name.to_ascii_lowercase();
        // Try substring first (distance 0 equivalent)
        if name_lower.contains(&lower_query) {
            results.push((id.clone(), 0));
            continue;
        }
        // Only fuzzy-match if names are similar length
        if name_lower.len().abs_diff(lower_query.len()) > max_dist {
            continue;
        }
        let dist = bounded_edit_distance(&node.name, query, max_dist);
        if dist <= max_dist {
            results.push((id.clone(), dist));
        }
    }

    results.sort_by(|a, b| {
        a.1.cmp(&b.1).then_with(|| {
            let na = graph.get_node(&a.0).map(|n| n.name.len()).unwrap_or(0);
            let nb = graph.get_node(&b.0).map(|n| n.name.len()).unwrap_or(0);
            na.cmp(&nb)
        })
    });
    results.truncate(limit);
    results
}

/// A parsed search query, separated into free-form `text` plus structured
/// filters extracted from `kind:`, `path:`, and `name:` field prefixes.
///
/// Constructed by [`parse_query`]; consumed by [`filtered_search`]. Empty
/// filter vectors mean "no constraint on this axis".
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ParsedQuery {
    /// Whitespace-joined remainder after pulling out recognized prefixes.
    pub text: String,
    /// Allowed [`NodeKind`]s from `kind:` tokens. Empty = any kind.
    pub kinds: Vec<crate::nodes::NodeKind>,
    /// `path:` substrings; a node matches if its `file_path` contains
    /// at least one (case-insensitive).
    pub path_filters: Vec<String>,
    /// `name:` substrings; a node matches if its `name` contains at
    /// least one (case-insensitive).
    pub name_filters: Vec<String>,
}

impl ParsedQuery {
    /// `true` iff any structured filter (kind/path/name) is populated.
    /// Used by callers to decide between filtered vs. legacy search paths.
    pub fn has_filters(&self) -> bool {
        !self.kinds.is_empty() || !self.path_filters.is_empty() || !self.name_filters.is_empty()
    }
}

/// Map a `kind:foo` token's payload to a [`NodeKind`].
///
/// Recognises the same prefixes [`kind_prefix`] emits, plus a few
/// human-friendly aliases (`function`, `variant`, `type`, `const`).
/// Returns `None` for unknown payloads so the caller can pass the
/// original `kind:foo` token through to `text` untouched.
fn parse_kind_token(payload: &str) -> Option<crate::nodes::NodeKind> {
    use crate::nodes::NodeKind;
    match payload.to_ascii_lowercase().as_str() {
        "function" | "fn" => Some(NodeKind::Function),
        "struct" => Some(NodeKind::Struct),
        "enum" => Some(NodeKind::Enum),
        "trait" => Some(NodeKind::Trait),
        "module" | "mod" => Some(NodeKind::Module),
        "variant" | "enumvariant" => Some(NodeKind::EnumVariant),
        "field" => Some(NodeKind::Field),
        "type" | "typealias" => Some(NodeKind::TypeAlias),
        "const" | "constant" => Some(NodeKind::Constant),
        "interface" => Some(NodeKind::Interface),
        _ => None,
    }
}

/// Parse a raw user query into a [`ParsedQuery`].
///
/// Whitespace-splits `raw`. Tokens matching `kind:<v>`, `path:<v>`,
/// `name:<v>` are pulled into the corresponding filter vectors;
/// unknown `foo:bar` prefixes and bare words fall through to `text`
/// (joined with single spaces, original order preserved).
///
/// An unknown `kind:` payload (e.g. `kind:thingamajig`) also falls
/// through to `text` so the user sees their query echoed verbatim
/// rather than silently dropped.
pub fn parse_query(raw: &str) -> ParsedQuery {
    let mut parsed = ParsedQuery::default();
    let mut text_parts: Vec<&str> = Vec::new();

    for token in raw.split_whitespace() {
        if let Some(rest) = token.strip_prefix("kind:") {
            if let Some(kind) = parse_kind_token(rest) {
                parsed.kinds.push(kind);
                continue;
            }
        } else if let Some(rest) = token.strip_prefix("path:") {
            if !rest.is_empty() {
                parsed.path_filters.push(rest.to_string());
                continue;
            }
        } else if let Some(rest) = token.strip_prefix("name:") {
            if !rest.is_empty() {
                parsed.name_filters.push(rest.to_string());
                continue;
            }
        }
        text_parts.push(token);
    }

    parsed.text = text_parts.join(" ");
    parsed
}

/// Resolve `query.text` (if any) then apply the structured filters.
///
/// - If `text` is empty, the candidate set is *every* node in the graph.
/// - If `text` is non-empty, candidates come from
///   [`crate::context::resolve_symbol`].
/// - Each populated filter list narrows the candidates: a node passes a
///   filter list iff at least one entry matches (logical OR within a
///   list; AND across lists). Path and name comparisons are
///   case-insensitive substring checks.
/// - Output is truncated to `limit` while preserving resolver order.
pub fn filtered_search(
    graph: &CodeGraph,
    query: &ParsedQuery,
    limit: usize,
) -> Vec<crate::nodes::NodeId> {
    let candidates: Vec<crate::nodes::NodeId> = if query.text.is_empty() {
        graph.all_node_ids().into_iter().cloned().collect()
    } else {
        crate::context::resolve_symbol(graph, &query.text)
    };

    let lower_paths: Vec<String> = query
        .path_filters
        .iter()
        .map(|p| p.to_ascii_lowercase())
        .collect();
    let lower_names: Vec<String> = query
        .name_filters
        .iter()
        .map(|n| n.to_ascii_lowercase())
        .collect();

    let mut out: Vec<crate::nodes::NodeId> = Vec::new();
    for id in candidates {
        let Some(node) = graph.get_node(&id) else {
            continue;
        };

        if !query.kinds.is_empty() && !query.kinds.contains(&node.kind) {
            continue;
        }

        if !lower_paths.is_empty() {
            let path_lower = node.file_path.to_string_lossy().to_ascii_lowercase();
            if !lower_paths.iter().any(|p| path_lower.contains(p)) {
                continue;
            }
        }

        if !lower_names.is_empty() {
            let name_lower = node.name.to_ascii_lowercase();
            if !lower_names.iter().any(|n| name_lower.contains(n)) {
                continue;
            }
        }

        out.push(id);
        if out.len() >= limit {
            break;
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::path::PathBuf;

    use super::*;
    use crate::nodes::{NodeData, NodeId, NodeKind, Span, Visibility};

    fn make_span(file: &str) -> Span {
        Span {
            file: PathBuf::from(file),
            start_line: 1,
            start_col: 0,
            end_line: 10,
            end_col: 1,
            byte_range: 0..100,
        }
    }

    fn make_node(file: &str, name: &str, qualified: &str, kind: NodeKind) -> NodeData {
        let id = NodeId::new(file, qualified, kind);
        NodeData {
            id,
            kind,
            name: name.to_string(),
            qualified_name: qualified.to_string(),
            file_path: PathBuf::from(file),
            span: make_span(file),
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
        graph.add_node(make_node(
            "src/sample.rs",
            "foo",
            "sample::foo",
            NodeKind::Function,
        ));
        graph.add_node(make_node(
            "src/sample.rs",
            "bar",
            "sample::bar",
            NodeKind::Function,
        ));
        graph.add_node(make_node(
            "src/lib.rs",
            "Config",
            "Config",
            NodeKind::Struct,
        ));
        graph.add_node(make_node("src/lib.rs", "Status", "Status", NodeKind::Enum));
        graph.add_node(make_node(
            "src/helpers.rs",
            "helpers",
            "helpers",
            NodeKind::Module,
        ));
        graph
    }

    #[test]
    fn test_symbol_table_build() {
        let graph = build_test_graph();
        let table = SymbolTable::build_from_graph(&graph);
        assert_eq!(table.len(), 5);
        assert!(!table.is_empty());
    }

    #[test]
    fn test_symbol_resolve_exact() {
        let graph = build_test_graph();
        let table = SymbolTable::build_from_graph(&graph);

        let entry = table.resolve("fn:sample::foo").expect("should resolve");
        assert_eq!(entry.kind, NodeKind::Function);
        assert_eq!(entry.qualified_name, "sample::foo");
        assert_eq!(entry.file_path, PathBuf::from("src/sample.rs"));

        let entry = table.resolve("struct:Config").expect("should resolve");
        assert_eq!(entry.kind, NodeKind::Struct);
        assert_eq!(entry.qualified_name, "Config");

        assert!(table.resolve("fn:nonexistent").is_none());
    }

    #[test]
    fn test_symbol_resolve_fuzzy() {
        let graph = build_test_graph();
        let table = SymbolTable::build_from_graph(&graph);

        let results = table.resolve_fuzzy("foo");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].handle, "fn:sample::foo");

        // Case-insensitive
        let results = table.resolve_fuzzy("CONFIG");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].handle, "struct:Config");

        // Partial match on prefix
        let results = table.resolve_fuzzy("fn:");
        assert_eq!(results.len(), 2);

        // No match
        let results = table.resolve_fuzzy("zzz_no_match");
        assert!(results.is_empty());
    }

    #[test]
    fn test_symbol_invalidate_file() {
        let graph = build_test_graph();
        let mut table = SymbolTable::build_from_graph(&graph);
        assert_eq!(table.len(), 5);

        // Invalidate src/sample.rs — removes foo and bar
        table.invalidate_file(Path::new("src/sample.rs"));
        assert_eq!(table.len(), 3);

        assert!(table.resolve("fn:sample::foo").is_none());
        assert!(table.resolve("fn:sample::bar").is_none());
        assert!(table.resolve("struct:Config").is_some());
        assert!(table.resolve("enum:Status").is_some());
        assert!(table.resolve("mod:helpers").is_some());
    }

    #[test]
    fn test_symbol_handle_for_node() {
        let graph = build_test_graph();
        let table = SymbolTable::build_from_graph(&graph);

        let foo_id = NodeId::new("src/sample.rs", "sample::foo", NodeKind::Function);
        let handle = table.handle_for_node(&foo_id).expect("should find handle");
        assert_eq!(handle, "fn:sample::foo");

        let config_id = NodeId::new("src/lib.rs", "Config", NodeKind::Struct);
        let handle = table
            .handle_for_node(&config_id)
            .expect("should find handle");
        assert_eq!(handle, "struct:Config");

        let fake_id = NodeId(99999);
        assert!(table.handle_for_node(&fake_id).is_none());
    }

    // ---------------------------------------------------------------
    // ResolutionJob cycle-detection tests
    //
    // These exercise the linked-list parent-chain machinery directly,
    // which is the public surface a future recursive resolver will use.
    // The current `SymbolTable::resolve` is non-recursive and so cannot
    // produce a cycle on its own — these tests cover the reusable
    // primitive via simulated recursive descent.
    // ---------------------------------------------------------------

    /// Simulate a recursive resolver driven by an adjacency map of
    /// symbol-handle -> dependent handles. Returns `Err` on cycle.
    fn resolve_recursive(
        deps: &HashMap<&'static str, Vec<&'static str>>,
        target: &str,
        parent: Option<&ResolutionJob>,
    ) -> Result<(), SymbolError> {
        // Cycle check: if `target` is already an ancestor, abort.
        if let Some(p) = parent {
            if p.is_cycle(target) {
                return Err(SymbolError::Cycle {
                    path: p.cycle_path(target),
                });
            }
        }

        let job = match parent {
            Some(p) => p.child(target),
            None => ResolutionJob::root(target),
        };

        if let Some(children) = deps.get(target) {
            for child in children {
                resolve_recursive(deps, child, Some(&job))?;
            }
        }
        Ok(())
    }

    #[test]
    fn resolve_detects_a_b_a_cycle_robust() {
        let mut deps: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        deps.insert("fn:A", vec!["fn:B"]);
        deps.insert("fn:B", vec!["fn:A"]);

        let err = resolve_recursive(&deps, "fn:A", None).expect_err("must detect cycle");
        let SymbolError::Cycle { path } = err;
        assert!(
            path.contains(&"fn:A".to_string()),
            "cycle path missing A: {path:?}"
        );
        assert!(
            path.contains(&"fn:B".to_string()),
            "cycle path missing B: {path:?}"
        );
        // Closes the loop: starts and ends at the offending symbol.
        assert_eq!(path.first().map(String::as_str), Some("fn:A"));
        assert_eq!(path.last().map(String::as_str), Some("fn:A"));
    }

    #[test]
    fn resolve_detects_self_loop_robust() {
        let mut deps: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        deps.insert("fn:A", vec!["fn:A"]);

        let err = resolve_recursive(&deps, "fn:A", None).expect_err("self-loop must error");
        let SymbolError::Cycle { path } = err;
        // A -> A is the smallest possible cycle.
        assert_eq!(path, vec!["fn:A".to_string(), "fn:A".to_string()]);
    }

    #[test]
    fn resolve_succeeds_on_acyclic_chain_normal() {
        // A -> B -> C -> D, no back-edges.
        let mut deps: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        deps.insert("fn:A", vec!["fn:B"]);
        deps.insert("fn:B", vec!["fn:C"]);
        deps.insert("fn:C", vec!["fn:D"]);
        deps.insert("fn:D", vec![]);

        resolve_recursive(&deps, "fn:A", None).expect("acyclic chain must resolve");
    }

    #[test]
    fn resolution_job_is_cycle_walks_parent_chain() {
        // Hand-build a 3-deep stack: A -> B -> C and probe.
        let a = ResolutionJob::root("fn:A");
        let b = a.child("fn:B");
        let c = b.child("fn:C");

        assert!(c.is_cycle("fn:A"), "ancestor A must be detected");
        assert!(c.is_cycle("fn:B"), "ancestor B must be detected");
        assert!(c.is_cycle("fn:C"), "self must be detected");
        assert!(!c.is_cycle("fn:Z"), "non-ancestor must not match");
    }

    #[test]
    fn resolution_job_cycle_path_is_diagnostic() {
        let a = ResolutionJob::root("fn:A");
        let b = a.child("fn:B");
        let c = b.child("fn:C");

        // Closing the loop on A while resolving C: path is A -> B -> C -> A.
        let path = c.cycle_path("fn:A");
        assert_eq!(
            path,
            vec![
                "fn:A".to_string(),
                "fn:B".to_string(),
                "fn:C".to_string(),
                "fn:A".to_string(),
            ]
        );

        // Target not in chain produces empty path.
        assert!(c.cycle_path("fn:Z").is_empty());
    }

    // ---------------------------------------------------------------
    // try_resolve_recursive — red-node deferred cycle recovery tests
    //
    // These exercise the ResolutionResult sentinel and the aggregation
    // policy. A synthetic dependency map drives the resolver closure so
    // we can test cycle topologies independently of a real graph.
    // ---------------------------------------------------------------

    /// Build a `SymbolTable` populated with the union of every handle that
    /// appears (as either source or dep) in the adjacency map. This lets
    /// `try_resolve_recursive`'s final "does the symbol exist" check pass
    /// for any handle the test deliberately includes.
    fn table_for_handles(handles: &[&'static str]) -> SymbolTable {
        let mut table = SymbolTable::new();
        for (i, h) in handles.iter().enumerate() {
            // Use a synthetic NodeId / minimal entry — these tests don't
            // exercise the entry payload, only existence + the handle string.
            let entry = SymbolEntry {
                node_id: NodeId(i as u64),
                handle: (*h).to_string(),
                file_path: PathBuf::from("synthetic.rs"),
                span: make_span("synthetic.rs"),
                qualified_name: (*h).to_string(),
                kind: NodeKind::Function,
            };
            table.by_handle.insert((*h).to_string(), entry);
        }
        table
    }

    #[test]
    fn try_resolve_recursive_returns_red_on_a_b_a_cycle_robust() {
        let mut deps: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        deps.insert("fn:A", vec!["fn:B"]);
        deps.insert("fn:B", vec!["fn:A"]);

        let table = table_for_handles(&["fn:A", "fn:B"]);
        let resolver = |h: &str, _job: &ResolutionJob<'_, '_>| -> Vec<&'static str> {
            deps.get(h).cloned().unwrap_or_default()
        };

        let result = try_resolve_recursive(&table, "fn:A", None, &resolver);
        assert!(result.is_red(), "A->B->A must produce Red, got {result:?}");
        let ResolutionResult::Red { path } = result else {
            unreachable!()
        };
        assert!(path.iter().any(|p| p == "fn:A"), "path missing A: {path:?}");
        assert!(path.iter().any(|p| p == "fn:B"), "path missing B: {path:?}");
    }

    #[test]
    fn try_resolve_recursive_returns_resolved_on_acyclic_normal() {
        // A -> B -> C, all known, no back-edges.
        let mut deps: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        deps.insert("fn:A", vec!["fn:B"]);
        deps.insert("fn:B", vec!["fn:C"]);
        deps.insert("fn:C", vec![]);

        let table = table_for_handles(&["fn:A", "fn:B", "fn:C"]);
        let resolver = |h: &str, _job: &ResolutionJob<'_, '_>| -> Vec<&'static str> {
            deps.get(h).cloned().unwrap_or_default()
        };

        let result = try_resolve_recursive(&table, "fn:A", None, &resolver);
        assert_eq!(result, ResolutionResult::Resolved("fn:A".to_string()));
        assert!(!result.is_red());
        assert_eq!(result.resolved().map(String::as_str), Some("fn:A"));
    }

    #[test]
    fn try_resolve_recursive_returns_unknown_for_missing_handle_robust() {
        // Resolver returns no dependencies, table contains no entries.
        let deps: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        let table = SymbolTable::new();
        let resolver = |h: &str, _job: &ResolutionJob<'_, '_>| -> Vec<&'static str> {
            deps.get(h).cloned().unwrap_or_default()
        };

        let result = try_resolve_recursive(&table, "fn:does_not_exist", None, &resolver);
        assert_eq!(result, ResolutionResult::Unknown);
        assert!(!result.is_red());
        assert!(result.resolved().is_none());
    }

    #[test]
    fn try_resolve_recursive_aggregates_red_paths_in_diamond_robust() {
        // A depends on B and C; both B and C depend on A. The resolution of
        // A descends into B (which sees A as ancestor → Red[A,B]) and into
        // C (which sees A as ancestor → Red[A,C]). The aggregate should
        // mention both B and C in some form (plus A).
        let mut deps: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        deps.insert("fn:A", vec!["fn:B", "fn:C"]);
        deps.insert("fn:B", vec!["fn:A"]);
        deps.insert("fn:C", vec!["fn:A"]);

        let table = table_for_handles(&["fn:A", "fn:B", "fn:C"]);
        let resolver = |h: &str, _job: &ResolutionJob<'_, '_>| -> Vec<&'static str> {
            deps.get(h).cloned().unwrap_or_default()
        };

        let result = try_resolve_recursive(&table, "fn:A", None, &resolver);
        let ResolutionResult::Red { path } = result else {
            panic!("diamond cycle must produce Red, got {result:?}");
        };
        assert!(
            path.iter().any(|p| p == "fn:A"),
            "diamond aggregate missing A: {path:?}"
        );
        assert!(
            path.iter().any(|p| p == "fn:B"),
            "diamond aggregate missing B (must show both branches): {path:?}"
        );
        assert!(
            path.iter().any(|p| p == "fn:C"),
            "diamond aggregate missing C (must show both branches): {path:?}"
        );
        // Dedup invariant: union semantics, no duplicate entries.
        let mut sorted = path.clone();
        sorted.sort();
        let mut deduped = sorted.clone();
        deduped.dedup();
        assert_eq!(sorted, deduped, "Red.path must be deduplicated: {path:?}");
    }

    #[test]
    fn red_node_does_not_poison_unrelated_consumers_normal() {
        // X cycles (X -> X self-loop). Y is acyclic and unrelated.
        // Resolving Y must NOT bleed Red from X — they share no edges.
        let mut deps: HashMap<&'static str, Vec<&'static str>> = HashMap::new();
        deps.insert("fn:X", vec!["fn:X"]);
        deps.insert("fn:Y", vec!["fn:Z"]);
        deps.insert("fn:Z", vec![]);

        let table = table_for_handles(&["fn:X", "fn:Y", "fn:Z"]);
        let resolver = |h: &str, _job: &ResolutionJob<'_, '_>| -> Vec<&'static str> {
            deps.get(h).cloned().unwrap_or_default()
        };

        // X is Red (self-loop).
        let x = try_resolve_recursive(&table, "fn:X", None, &resolver);
        assert!(x.is_red(), "self-loop X must be Red");

        // Y is Resolved — completely independent of X's cycle.
        let y = try_resolve_recursive(&table, "fn:Y", None, &resolver);
        assert_eq!(
            y,
            ResolutionResult::Resolved("fn:Y".to_string()),
            "Y unrelated to X must resolve cleanly, got {y:?}"
        );
        assert!(!y.is_red(), "unrelated symbol must not be poisoned");
    }

    #[test]
    fn test_symbol_deterministic() {
        let graph = build_test_graph();
        let table1 = SymbolTable::build_from_graph(&graph);
        let table2 = SymbolTable::build_from_graph(&graph);

        // Same graph produces same handles
        let mut handles1 = table1.all_handles();
        let mut handles2 = table2.all_handles();
        handles1.sort();
        handles2.sort();
        assert_eq!(handles1, handles2);

        // Each handle resolves to same entry data
        for handle in &handles1 {
            let e1 = table1.resolve(handle).unwrap();
            let e2 = table2.resolve(handle).unwrap();
            assert_eq!(e1.node_id, e2.node_id);
            assert_eq!(e1.qualified_name, e2.qualified_name);
            assert_eq!(e1.kind, e2.kind);
            assert_eq!(e1.file_path, e2.file_path);
        }
    }

    // ---------------------------------------------------------------
    // bounded_edit_distance tests
    // ---------------------------------------------------------------

    #[test]
    fn edit_distance_identical_is_zero() {
        assert_eq!(bounded_edit_distance("foo", "foo", 5), 0);
        // Case-folded: equal modulo case.
        assert_eq!(bounded_edit_distance("Foo", "fOO", 5), 0);
    }

    #[test]
    fn edit_distance_empty_inputs() {
        assert_eq!(bounded_edit_distance("", "", 3), 0);
        assert_eq!(bounded_edit_distance("", "abc", 5), 3);
        assert_eq!(bounded_edit_distance("abc", "", 5), 3);
    }

    #[test]
    fn edit_distance_basic_edits() {
        // single substitution
        assert_eq!(bounded_edit_distance("cat", "bat", 5), 1);
        // single insertion
        assert_eq!(bounded_edit_distance("cat", "cats", 5), 1);
        // single deletion
        assert_eq!(bounded_edit_distance("cats", "cat", 5), 1);
        // classic kitten/sitting = 3
        assert_eq!(bounded_edit_distance("kitten", "sitting", 5), 3);
    }

    #[test]
    fn edit_distance_early_exit_on_length_gap() {
        // Length diff alone exceeds bound; must short-circuit to max+1.
        let d = bounded_edit_distance("a", "abcdefghij", 3);
        assert_eq!(d, 4, "len-gap short-circuit should return max_dist+1");
    }

    #[test]
    fn edit_distance_early_exit_on_row_min() {
        // Strings of equal length but very different content; row-min
        // pruning kicks in.
        let d = bounded_edit_distance("aaaaaa", "bbbbbb", 2);
        assert_eq!(d, 3, "row-min pruning should return max_dist+1");
    }

    #[test]
    fn edit_distance_at_exact_bound() {
        // Distance == max_dist must be reported as-is, not pruned.
        assert_eq!(bounded_edit_distance("cat", "bat", 1), 1);
    }

    // ---------------------------------------------------------------
    // parse_query tests
    // ---------------------------------------------------------------

    #[test]
    fn parse_query_empty_input() {
        let p = parse_query("");
        assert_eq!(p.text, "");
        assert!(p.kinds.is_empty());
        assert!(p.path_filters.is_empty());
        assert!(p.name_filters.is_empty());
        assert!(!p.has_filters());
    }

    #[test]
    fn parse_query_plain_text_only() {
        let p = parse_query("foo bar baz");
        assert_eq!(p.text, "foo bar baz");
        assert!(!p.has_filters());
    }

    #[test]
    fn parse_query_extracts_kind() {
        let p = parse_query("kind:function authenticate");
        assert_eq!(p.text, "authenticate");
        assert_eq!(p.kinds, vec![NodeKind::Function]);
        assert!(p.has_filters());
    }

    #[test]
    fn parse_query_extracts_path_and_name() {
        let p = parse_query("path:src/api name:auth handler");
        assert_eq!(p.text, "handler");
        assert_eq!(p.path_filters, vec!["src/api".to_string()]);
        assert_eq!(p.name_filters, vec!["auth".to_string()]);
        assert!(p.has_filters());
    }

    #[test]
    fn parse_query_recognizes_all_kinds() {
        let p = parse_query(
            "kind:function kind:struct kind:enum kind:trait kind:module \
             kind:variant kind:field kind:type kind:const kind:interface",
        );
        assert_eq!(p.text, "");
        assert_eq!(
            p.kinds,
            vec![
                NodeKind::Function,
                NodeKind::Struct,
                NodeKind::Enum,
                NodeKind::Trait,
                NodeKind::Module,
                NodeKind::EnumVariant,
                NodeKind::Field,
                NodeKind::TypeAlias,
                NodeKind::Constant,
                NodeKind::Interface,
            ]
        );
    }

    #[test]
    fn parse_query_passes_through_unknown_prefix() {
        // `foo:bar` is not a recognized prefix; must show up verbatim in text.
        let p = parse_query("foo:bar real query");
        assert_eq!(p.text, "foo:bar real query");
        assert!(!p.has_filters());
    }

    #[test]
    fn parse_query_passes_through_unknown_kind_payload() {
        // Recognized prefix but bogus payload: keep the token in text rather
        // than silently dropping it.
        let p = parse_query("kind:thingamajig leftover");
        assert_eq!(p.text, "kind:thingamajig leftover");
        assert!(p.kinds.is_empty());
        assert!(!p.has_filters());
    }

    #[test]
    fn parse_query_kind_is_case_insensitive() {
        let p = parse_query("kind:Function kind:STRUCT");
        assert_eq!(p.kinds, vec![NodeKind::Function, NodeKind::Struct]);
    }

    #[test]
    fn parse_query_multiple_filters_accumulate() {
        let p = parse_query("kind:fn kind:struct path:src/a path:src/b name:foo name:bar query");
        assert_eq!(p.text, "query");
        assert_eq!(p.kinds, vec![NodeKind::Function, NodeKind::Struct]);
        assert_eq!(
            p.path_filters,
            vec!["src/a".to_string(), "src/b".to_string()]
        );
        assert_eq!(p.name_filters, vec!["foo".to_string(), "bar".to_string()]);
    }

    #[test]
    fn parse_query_empty_path_or_name_payload_passes_through() {
        // `path:` with no payload is meaningless; leave it in text so the
        // user sees their typo.
        let p = parse_query("path: name: legitimate");
        assert_eq!(p.text, "path: name: legitimate");
        assert!(p.path_filters.is_empty());
        assert!(p.name_filters.is_empty());
    }

    // ---------------------------------------------------------------
    // filtered_search tests
    // ---------------------------------------------------------------

    #[test]
    fn filtered_search_kind_only_returns_all_matching_kind() {
        let graph = build_test_graph();
        let query = parse_query("kind:function");
        let hits = filtered_search(&graph, &query, 50);
        // foo and bar are the two functions in the test graph.
        assert_eq!(hits.len(), 2);
        for id in &hits {
            assert_eq!(graph.get_node(id).unwrap().kind, NodeKind::Function);
        }
    }

    #[test]
    fn filtered_search_path_filter_narrows_set() {
        let graph = build_test_graph();
        let query = parse_query("path:sample.rs");
        let hits = filtered_search(&graph, &query, 50);
        assert_eq!(hits.len(), 2, "only nodes in src/sample.rs should match");
        for id in &hits {
            let node = graph.get_node(id).unwrap();
            assert!(node.file_path.to_string_lossy().contains("sample.rs"));
        }
    }

    #[test]
    fn filtered_search_name_filter_case_insensitive() {
        let graph = build_test_graph();
        let query = parse_query("name:CONFIG");
        let hits = filtered_search(&graph, &query, 50);
        assert_eq!(hits.len(), 1);
        assert_eq!(graph.get_node(&hits[0]).unwrap().name, "Config");
    }

    #[test]
    fn filtered_search_combined_filters_intersect_across_axes() {
        let graph = build_test_graph();
        // kind=function AND path=sample.rs AND name=foo → just foo.
        let query = parse_query("kind:function path:sample.rs name:foo");
        let hits = filtered_search(&graph, &query, 50);
        assert_eq!(hits.len(), 1);
        let node = graph.get_node(&hits[0]).unwrap();
        assert_eq!(node.name, "foo");
        assert_eq!(node.kind, NodeKind::Function);
    }

    #[test]
    fn filtered_search_respects_limit() {
        let graph = build_test_graph();
        let query = parse_query("kind:function");
        let hits = filtered_search(&graph, &query, 1);
        assert_eq!(hits.len(), 1, "limit must cap output");
    }

    #[test]
    fn filtered_search_empty_text_walks_all_nodes() {
        let graph = build_test_graph();
        // No text, just a kind filter → enumerate the whole graph and keep
        // nodes matching the kind.
        let query = parse_query("kind:struct");
        let hits = filtered_search(&graph, &query, 50);
        assert_eq!(hits.len(), 1);
        assert_eq!(graph.get_node(&hits[0]).unwrap().kind, NodeKind::Struct);
    }

    #[test]
    fn filtered_search_no_matches_returns_empty() {
        let graph = build_test_graph();
        let query = parse_query("kind:function name:does_not_exist_anywhere");
        let hits = filtered_search(&graph, &query, 50);
        assert!(hits.is_empty());
    }
}
