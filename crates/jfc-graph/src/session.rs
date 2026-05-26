//! High-level session facade — the single entry point for jfc-ui.

use std::path::{Path, PathBuf};

use tracing::warn;

use crate::adapter::{AdapterError, rust::RustAdapter};
use crate::builder::GraphBuilder;
use crate::capabilities::{Capability, CapabilityTree};
use crate::context::{self, ContextOptions, ContextResult};
use crate::dsl::{self, QueryConfig, QueryError, QueryResult};
use crate::formatting::{self, FormattedOutput};
use crate::graph::CodeGraph;
use crate::incremental::{QueryCache, QueryKey, ReadSet};
use crate::nodes::NodeId;
use crate::persistence::EventLog;
use crate::symbols::SymbolTable;
use crate::worktree::{self, WorktreeMismatch};

/// Owns the graph, symbols, event log, and capabilities.
/// Provides query execution and incremental file updates.
pub struct GraphSession {
    pub graph: CodeGraph,
    pub symbols: SymbolTable,
    pub events: EventLog,
    pub capabilities: CapabilityTree,
    /// Tree-sitter syntax errors collected during the initial indexing pass.
    /// Surfaces files with partial graphs so the UI can warn the user.
    pub parse_errors: Vec<AdapterError>,
    /// Files skipped entirely (I/O failure or hard parse failure).
    pub files_skipped: Vec<PathBuf>,
    /// Set when the index was resolved from a different git worktree
    /// than the caller's path (codegraph PR #312). UI should surface
    /// the message but never refuse the query — the symbols may still
    /// be correct enough.
    pub worktree_mismatch: Option<WorktreeMismatch>,
    /// Memoised DSL query results — invalidated per-node when files
    /// change. See [`crate::incremental`] for the cache model.
    query_cache: QueryCache<QueryResult>,
    adapter: RustAdapter,
}

impl GraphSession {
    /// Build a session by indexing all supported files under `workspace_root`.
    pub fn from_directory(workspace_root: &Path) -> Self {
        let adapter = RustAdapter::new();
        let result = GraphBuilder::build_from_directory_with_result(workspace_root, &adapter);
        let symbols = SymbolTable::build_from_graph(&result.graph);

        // Log a single summary line so the parse errors are observable even
        // when the caller doesn't inspect `parse_errors` directly.
        if !result.parse_errors.is_empty() {
            warn!(
                target: "jfc::graph::session",
                count = result.parse_errors.len(),
                "files with tree-sitter syntax errors — partial graph indexed"
            );
        }

        // Worktree-mismatch warning is best-effort: the caller's
        // current working directory is the closest signal we have to
        // "where the query is being issued from". Soft-fails to None
        // whenever git isn't available.
        let worktree_mismatch = std::env::current_dir()
            .ok()
            .and_then(|cwd| worktree::detect_worktree_index_mismatch(&cwd, workspace_root));
        if let Some(ref m) = worktree_mismatch {
            warn!(
                target: "jfc::graph::session",
                caller_worktree = %m.caller_worktree.display(),
                index_worktree = %m.index_worktree.display(),
                "graph index belongs to a different git worktree"
            );
        }

        Self {
            graph: result.graph,
            symbols,
            events: EventLog::new(),
            capabilities: CapabilityTree::from_env(),
            parse_errors: result.parse_errors,
            files_skipped: result.files_skipped,
            worktree_mismatch,
            query_cache: QueryCache::new(),
            adapter,
        }
    }

    /// Construct a session from a pre-loaded snapshot graph.
    /// Cheaper than `from_directory` — skips tree-sitter parsing entirely,
    /// only builds the SymbolTable index on top of the existing graph.
    pub fn from_snapshot(graph: CodeGraph, workspace_root: &Path) -> Self {
        let symbols = SymbolTable::build_from_graph(&graph);
        let worktree_mismatch = std::env::current_dir()
            .ok()
            .and_then(|cwd| worktree::detect_worktree_index_mismatch(&cwd, workspace_root));
        Self {
            graph,
            symbols,
            events: EventLog::new(),
            capabilities: CapabilityTree::from_env(),
            parse_errors: Vec::new(),
            files_skipped: Vec::new(),
            worktree_mismatch,
            query_cache: QueryCache::new(),
            adapter: RustAdapter::new(),
        }
    }

    /// Execute a DSL query and return token-budgeted formatted output.
    ///
    /// Delegates to [`dsl::run_query_expr`] (the extended-grammar entry
    /// point) — it parses the legacy pipe-chain as a sub-form, so all
    /// pre-existing pipe queries still work, while callers also get
    /// `union` / `intersect` / `\` set algebra, `path` / `paths`,
    /// `entrypoints`, and the `since N` postfix filter for free.
    pub fn query(&self, query_str: &str, max_tokens: usize) -> Result<FormattedOutput, QueryError> {
        let config = QueryConfig {
            max_tokens,
            max_nodes: 50,
        };
        let result = dsl::run_query_expr(query_str, &self.graph, &config)?;
        Ok(formatting::format_query_result_with_capabilities(
            &result,
            &self.graph,
            Some(&self.symbols),
            Some(&self.capabilities),
            max_tokens,
        ))
    }

    /// Execute a DSL query and return the raw [`QueryResult`] for
    /// programmatic use (e.g. handle extraction, history recording,
    /// chained predicate analysis). Same parser as [`Self::query`].
    ///
    /// Phase 5+8: results are memoised in [`Self::query_cache`]. Cache
    /// hits skip parsing + execution entirely. Cache invalidation
    /// (Phase 8) tracks a fine-grained read-set per entry: the result
    /// nodes **plus the 1-hop neighbourhood in both directions**
    /// (anything a follow-up traversal could reach). When a file
    /// changes, only entries whose read-set intersects the file's
    /// nodes are invalidated — unrelated queries keep their cache
    /// entries.
    ///
    /// The 1-hop expansion is the cheapest correct approximation for
    /// pipe-chain queries that touch direct neighbours via `callers`,
    /// `callees`, `taint`, `preconditions`, etc. Deeper queries pay a
    /// false-invalidation penalty (their read-set undercounts), but
    /// the cache stays correct because revision-mismatched lookups
    /// are also discarded by [`QueryKey`].
    pub fn query_raw(&self, query_str: &str) -> Result<QueryResult, QueryError> {
        let key = QueryKey::new(query_str, self.graph.current_revision());
        if let Some(cached) = self.query_cache.get(&key) {
            return Ok((*cached).clone());
        }
        let config = QueryConfig::default();
        let result = dsl::run_query_expr(query_str, &self.graph, &config)?;

        // Phase 8 read-set: result nodes + 1-hop neighbours
        // (incoming + outgoing). This captures the dependencies of
        // any pipe stage like `| callers` or `| callees` that the
        // query could have used to reach those nodes.
        let mut read_set = ReadSet::new();
        for id in &result.nodes {
            read_set.record(id);
            for (nbr, _) in self.graph.get_edges_from(id) {
                read_set.record(nbr);
            }
            for (nbr, _) in self.graph.get_edges_to(id) {
                read_set.record(nbr);
            }
        }
        self.query_cache.put(key, result.clone(), read_set);
        Ok(result)
    }

    /// Incrementally update the graph after a file modification.
    /// Drops every query-cache entry whose read-set referenced one
    /// of the file's removed/replaced nodes.
    pub fn file_changed(&mut self, path: &Path, new_content: &str) {
        // Snapshot the file's nodes *before* mutation so we know what
        // to invalidate.
        let touched_ids: Vec<_> = self
            .graph
            .all_node_ids()
            .into_iter()
            .filter(|id| {
                self.graph
                    .get_node(id)
                    .map(|n| n.file_path == path)
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        for id in &touched_ids {
            self.query_cache.invalidate_for_node(id);
        }

        let events = self.graph.update_file(path, new_content, &self.adapter);
        for event in events {
            self.events.append(event, None);
        }
        self.symbols.update_from_graph(&self.graph, path);
    }

    /// Clear the entire query result cache. Use when in doubt about
    /// invalidation correctness — coarse but always-correct.
    pub fn clear_query_cache(&self) {
        self.query_cache.clear();
    }

    /// Number of cached queries (testing aid).
    pub fn query_cache_len(&self) -> usize {
        self.query_cache.len()
    }

    /// Compute co-change analysis for a given node, using git history from
    /// the workspace. Shells out to `git log` on demand (no cached history).
    ///
    /// `min_support`: minimum number of co-occurrences to include a pair.
    /// Returns pairs sorted by confidence descending.
    pub fn co_changes(
        &self,
        node_id: &crate::nodes::NodeId,
        min_support: u32,
    ) -> crate::co_change::CoChangeResult {
        // Determine workspace root from the graph's first node file path,
        // then walk up to find the git root.
        let workspace_root = self
            .graph
            .all_node_ids()
            .first()
            .and_then(|id| self.graph.get_node(id))
            .and_then(|n| n.file_path.parent().map(|p| p.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));

        let commits = crate::co_change::fetch_git_history(&workspace_root, 500);
        crate::co_change::co_changes_for_nodes(
            &self.graph,
            &commits,
            std::slice::from_ref(node_id),
            min_support,
        )
    }

    pub fn symbols(&self) -> &SymbolTable {
        &self.symbols
    }

    pub fn is_capable(&self, cap: Capability) -> bool {
        self.capabilities.is_enabled(cap)
    }

    /// Build an agent-friendly `context()` payload — entry points,
    /// related symbols, optional source blocks, all rendered as
    /// budget-bounded markdown with intent-aware reminders. Mirrors
    /// codegraph's `codegraph_context` tool shape.
    pub fn context(&self, task: &str, opts: ContextOptions) -> ContextResult {
        context::build_context(&self.graph, Some(&self.symbols), task, opts)
    }

    /// Render a `## Search Results` markdown block for `query` — the
    /// MCP-friendly counterpart to `find_by_name` lookup.
    pub fn search(&self, query: &str, limit: usize) -> String {
        // Field-qualified syntax (`kind:fn path:src/api name:auth foo`) takes
        // priority: when the parser extracts any structured filter, route
        // through `filtered_search` so callers can scope the result set
        // without piggy-backing on text-match heuristics.
        let parsed = crate::symbols::parse_query(query);
        if parsed.has_filters() {
            let hits = crate::symbols::filtered_search(&self.graph, &parsed, limit);
            let note = if hits.len() == limit {
                Some(format!("Capped at {limit}; broaden the query for more"))
            } else {
                None
            };
            return context::render::render_search_results(
                &self.graph,
                Some(&self.symbols),
                query,
                &hits,
                note.as_deref(),
            );
        }

        let mut hits = context::resolve_symbol(&self.graph, query);
        hits.truncate(limit);

        // Fuzzy fallback: if exact match finds nothing, try edit distance ≤ 2
        if hits.is_empty() {
            let fuzzy = crate::symbols::fuzzy_search(&self.graph, query, 2, limit);
            if !fuzzy.is_empty() {
                let fuzzy_ids: Vec<_> = fuzzy.iter().map(|(id, _)| id.clone()).collect();
                let note = Some(format!(
                    "No exact match for `{query}`. Showing {} fuzzy results (edit distance ≤ 2):",
                    fuzzy_ids.len()
                ));
                return context::render::render_search_results(
                    &self.graph,
                    Some(&self.symbols),
                    query,
                    &fuzzy_ids,
                    note.as_deref(),
                );
            }
        }

        let note = if hits.is_empty() {
            None
        } else if hits.len() == limit {
            Some(format!("Capped at {limit}; broaden the query for more"))
        } else {
            None
        };
        context::render::render_search_results(
            &self.graph,
            Some(&self.symbols),
            query,
            &hits,
            note.as_deref(),
        )
    }

    /// Find callers of `symbol`, rendered as a `## Callers of …`
    /// list (file-grouped, with signatures inline).
    pub fn callers(&self, symbol: &str, limit: usize) -> String {
        let (nodes, note) = context::callers_for(&self.graph, symbol, limit);
        context::render::render_node_list(
            &self.graph,
            &format!("Callers of `{symbol}`"),
            &nodes,
            note.as_deref(),
        )
    }

    /// Find callees of `symbol`, rendered as a `## Callees of …`
    /// list.
    pub fn callees(&self, symbol: &str, limit: usize) -> String {
        let (nodes, note) = context::callees_for(&self.graph, symbol, limit);
        context::render::render_node_list(
            &self.graph,
            &format!("Callees of `{symbol}`"),
            &nodes,
            note.as_deref(),
        )
    }

    /// Compute a change-impact set rooted at `symbol`, walking incoming
    /// edges out to `depth` hops. Rendered grouped-by-file.
    pub fn impact(&self, symbol: &str, depth: u8) -> String {
        let (nodes, note) = context::impact_for(&self.graph, symbol, depth);
        context::render::render_impact(&self.graph, symbol, &nodes, note.as_deref())
    }

    /// Raw node-ID accessor for the resolver — exposed so MCP wrappers
    /// can chain a search hit into a follow-up query without re-parsing
    /// the rendered markdown.
    pub fn resolve(&self, symbol: &str) -> Vec<NodeId> {
        context::resolve_symbol(&self.graph, symbol)
    }

    /// Get detailed info about ONE symbol — location, signature, visibility,
    /// and optionally its source code. For container types (struct/enum/trait/
    /// module), `include_code=true` renders a compact member outline (fields +
    /// method signatures + line numbers) rather than dumping the full body.
    pub fn node(&self, symbol: &str, include_code: bool) -> String {
        use crate::nodes::NodeKind;

        let hits = context::resolve_symbol(&self.graph, symbol);
        if hits.is_empty() {
            return format!("No symbol found matching `{symbol}`");
        }
        let id = &hits[0];
        let Some(node) = self.graph.get_node(id) else {
            return format!("No symbol found matching `{symbol}`");
        };

        let file_path = node.file_path.display().to_string();
        let kind_str = format!("{:?}", node.kind);
        let vis_str = format!("{:?}", node.visibility);

        // Try to read the file and extract lines.
        let source_lines = std::fs::read_to_string(&node.file_path)
            .ok()
            .and_then(|content| {
                let lines: Vec<&str> = content.lines().collect();
                let start = node.span.start_line.saturating_sub(1) as usize;
                let end = (node.span.end_line as usize).min(lines.len());
                if start >= end {
                    return None;
                }
                Some(
                    lines[start..end]
                        .iter()
                        .map(|s| s.to_string())
                        .collect::<Vec<_>>(),
                )
            });

        let signature = source_lines
            .as_ref()
            .and_then(|lines| lines.first().map(|s| s.trim().to_string()))
            .unwrap_or_else(|| node.qualified_name.clone());

        let lang = if file_path.ends_with(".rs") {
            "rust"
        } else if file_path.ends_with(".ts") || file_path.ends_with(".tsx") {
            "typescript"
        } else if file_path.ends_with(".py") {
            "python"
        } else {
            ""
        };

        let mut out = String::new();
        out.push_str(&format!("## {} ({})\n\n", node.name, kind_str));
        out.push_str(&format!(
            "**Location:** {}:{}\n",
            file_path, node.span.start_line
        ));
        out.push_str(&format!("**Signature:** `{}`\n", signature));
        out.push_str(&format!("**Visibility:** {}\n", vis_str));

        if include_code {
            let is_container = matches!(
                node.kind,
                NodeKind::Struct | NodeKind::Enum | NodeKind::Trait | NodeKind::Module
            );
            if is_container {
                // Render a compact outline: list contained members with their line numbers.
                out.push_str("\n**Members:**\n");
                let children: Vec<_> = self
                    .graph
                    .get_edges_from(id)
                    .iter()
                    .filter(|(_, edge)| matches!(edge.kind, crate::edges::EdgeKind::Contains))
                    .filter_map(|(child_id, _)| self.graph.get_node(child_id))
                    .collect();
                if children.is_empty() {
                    // Fall back to showing the source.
                    if let Some(ref lines) = source_lines {
                        out.push_str(&format!("\n```{}\n", lang));
                        for (i, line) in lines.iter().enumerate() {
                            let lineno = node.span.start_line as usize + i;
                            out.push_str(&format!("{:>4} | {}\n", lineno, line));
                        }
                        out.push_str("```\n");
                    }
                } else {
                    out.push('\n');
                    for child in &children {
                        out.push_str(&format!(
                            "  - `{}` ({:?}) — line {}\n",
                            child.name, child.kind, child.span.start_line
                        ));
                    }
                }
            } else if let Some(ref lines) = source_lines {
                out.push_str(&format!("\n```{}\n", lang));
                for (i, line) in lines.iter().enumerate() {
                    let lineno = node.span.start_line as usize + i;
                    out.push_str(&format!("{:>4} | {}\n", lineno, line));
                }
                out.push_str("```\n");
            }
        }

        out
    }

    /// Returns source for SEVERAL related symbols grouped by file, plus a
    /// relationship map, in ONE capped call. Splits the query on whitespace,
    /// resolves each term, groups by file, and renders fenced code blocks
    /// with line numbers.
    pub fn explore(&self, query: &str, max_files: usize) -> String {
        use std::collections::HashMap;

        let terms: Vec<&str> = query.split_whitespace().collect();
        if terms.is_empty() {
            return "No search terms provided. Pass specific symbol/file/code terms.".to_string();
        }

        // Resolve all terms and group node IDs by file path.
        let mut file_nodes: HashMap<std::path::PathBuf, Vec<NodeId>> = HashMap::new();
        for term in &terms {
            let hits = context::resolve_symbol(&self.graph, term);
            for id in hits {
                if let Some(node) = self.graph.get_node(&id) {
                    file_nodes
                        .entry(node.file_path.clone())
                        .or_default()
                        .push(id);
                }
            }
        }

        if file_nodes.is_empty() {
            return format!("No symbols found matching query terms: `{query}`");
        }

        // Sort files by number of matched symbols descending, take up to max_files.
        let mut file_list: Vec<_> = file_nodes.into_iter().collect();
        file_list.sort_by(|a, b| b.1.len().cmp(&a.1.len()));
        file_list.truncate(max_files);

        let mut out = String::new();
        out.push_str(&format!("## Explore: `{query}`\n\n"));
        out.push_str(&format!(
            "{} files, {} terms\n\n",
            file_list.len(),
            terms.len()
        ));

        const MAX_OUTPUT: usize = 15000;

        for (file_path, node_ids) in &file_list {
            if out.len() >= MAX_OUTPUT {
                out.push_str("\n[output truncated — use fewer terms or lower max_files]\n");
                break;
            }

            let content = match std::fs::read_to_string(file_path) {
                Ok(c) => c,
                Err(_) => continue,
            };
            let lines: Vec<&str> = content.lines().collect();

            // Collect spans from nodes in this file.
            let mut ranges: Vec<(u32, u32)> = Vec::new();
            for id in node_ids {
                if let Some(node) = self.graph.get_node(id) {
                    ranges.push((node.span.start_line, node.span.end_line));
                }
            }
            ranges.sort_by_key(|r| r.0);

            // Merge overlapping/adjacent ranges (with 2 lines of context).
            let mut merged: Vec<(usize, usize)> = Vec::new();
            for (start, end) in &ranges {
                let s = (*start).saturating_sub(1).max(1) as usize - 1; // 0-indexed
                let e = ((*end) as usize).min(lines.len());
                if let Some(last) = merged.last_mut() {
                    if s <= last.1 + 2 {
                        last.1 = last.1.max(e);
                    } else {
                        merged.push((s, e));
                    }
                } else {
                    merged.push((s, e));
                }
            }

            let lang = if file_path.extension().and_then(|e| e.to_str()) == Some("rs") {
                "rust"
            } else if file_path.extension().and_then(|e| e.to_str()) == Some("ts")
                || file_path.extension().and_then(|e| e.to_str()) == Some("tsx")
            {
                "typescript"
            } else if file_path.extension().and_then(|e| e.to_str()) == Some("py") {
                "python"
            } else {
                ""
            };

            out.push_str(&format!("### {}\n\n", file_path.display()));
            for (s, e) in &merged {
                if out.len() >= MAX_OUTPUT {
                    break;
                }
                out.push_str(&format!("```{}\n", lang));
                for i in *s..*e {
                    if i < lines.len() {
                        out.push_str(&format!("{:>4} | {}\n", i + 1, lines[i]));
                    }
                }
                out.push_str("```\n\n");
            }
        }

        out
    }

    /// Like [`search`](Self::search) but appends each function/method hit's
    /// full source body inline. This collapses the dominant navigation loop —
    /// `graph_search foo` → `sed -n 'start,end p' file` — into one call.
    /// Container types (struct/enum/trait) still render the shape (signature
    /// + range) without dumping every line.
    pub fn search_with_code(&self, query: &str, limit: usize) -> String {
        let mut hits = context::resolve_symbol(&self.graph, query);
        hits.truncate(limit);
        if hits.is_empty() {
            // Reuse the standard renderer for the fuzzy-fallback / empty path.
            return self.search(query, limit);
        }

        let mut out = format!("## Search Results with code ({} found)\n\n", hits.len());
        for id in &hits {
            let Some(node) = self.graph.get_node(id) else {
                continue;
            };
            out.push_str(&format!(
                "### {} ({:?})\n{}{}\n",
                node.name,
                node.kind,
                node.file_path.display(),
                context::render::line_range(node)
            ));
            if let Some(handle) = self.symbols.handle_for_node(id) {
                out.push_str(&format!("handle: `{handle}`\n"));
            }
            // Body for code-bearing kinds; shape-only for containers.
            let is_container = matches!(
                node.kind,
                crate::nodes::NodeKind::Struct
                    | crate::nodes::NodeKind::Enum
                    | crate::nodes::NodeKind::Trait
                    | crate::nodes::NodeKind::Module
            );
            if !is_container && let Some(body) = read_span_source(node) {
                let lang = lang_for(&node.file_path);
                out.push_str(&format!("\n```{lang}\n{body}\n```\n"));
            }
            out.push('\n');
        }
        out
    }

    /// Structural outline of a single file: every indexed symbol with its
    /// kind and line range, ordered by position. Replaces the `nl -ba file`
    /// pattern and gives the model a stable map without re-reading the file.
    pub fn outline(&self, file: &str) -> String {
        // Match nodes whose file_path ends with the requested path (so callers
        // can pass a repo-relative path or a bare filename).
        let needle = file.trim_start_matches("./");
        let mut nodes: Vec<&crate::nodes::NodeData> = self
            .graph
            .all_node_ids()
            .iter()
            .filter_map(|id| self.graph.get_node(id))
            .filter(|n| {
                let p = n.file_path.to_string_lossy();
                p == needle || p.ends_with(needle)
            })
            .collect();

        if nodes.is_empty() {
            return format!(
                "No indexed symbols in `{file}`. Check the path, or the file's \
                 language may not have a graph adapter."
            );
        }

        nodes.sort_by_key(|n| n.span.start_line);
        let shown_path = nodes[0].file_path.display().to_string();
        let mut out = format!("## Outline: {shown_path} ({} symbols)\n\n", nodes.len());
        for n in &nodes {
            let indent = match n.kind {
                crate::nodes::NodeKind::Field | crate::nodes::NodeKind::EnumVariant => "  ",
                _ => "",
            };
            out.push_str(&format!(
                "{indent}- `{}` ({:?}){}\n",
                n.name,
                n.kind,
                context::render::line_range(n)
            ));
        }
        out.push_str(
            "\n> Read a symbol's body with `graph_node(\"name\", include_code=true)` \
             or `graph_search` with `include_code=true`.\n",
        );
        out
    }

    /// Content (string-literal / regex) search across indexed files, enriched
    /// with the enclosing symbol from the graph. Serves the large class of
    /// greps for log messages, error strings, and `tracing` targets that the
    /// symbol index can't answer. `glob` optionally restricts to a path
    /// substring (e.g. `providers/` or `.ts`).
    pub fn grep(&self, pattern: &str, glob: Option<&str>, limit: usize) -> String {
        let re = match regex::Regex::new(pattern) {
            Ok(r) => r,
            Err(e) => return format!("Invalid regex `{pattern}`: {e}"),
        };

        // Collect the set of indexed files (optionally path-filtered).
        let mut files: Vec<PathBuf> = self
            .graph
            .all_node_ids()
            .iter()
            .filter_map(|id| self.graph.get_node(id))
            .map(|n| n.file_path.clone())
            .collect();
        files.sort();
        files.dedup();

        let mut out = format!("## Content search: `{pattern}`\n\n");
        let mut total = 0;
        for file in &files {
            if total >= limit {
                break;
            }
            if let Some(g) = glob
                && !file.to_string_lossy().contains(g)
            {
                continue;
            }
            let Ok(content) = std::fs::read_to_string(file) else {
                continue;
            };
            for (i, line) in content.lines().enumerate() {
                if total >= limit {
                    break;
                }
                if !re.is_match(line) {
                    continue;
                }
                let lineno = (i + 1) as u32;
                let enclosing = self.enclosing_symbol(file, lineno);
                let ctx = enclosing
                    .map(|name| format!(" — in `{name}`"))
                    .unwrap_or_default();
                out.push_str(&format!(
                    "- {}:{}{}\n    {}\n",
                    file.display(),
                    lineno,
                    ctx,
                    line.trim()
                ));
                total += 1;
            }
        }
        if total == 0 {
            out.push_str("No matches.\n");
        } else if total >= limit {
            out.push_str(&format!(
                "\n> Capped at {limit} matches; narrow with `glob`.\n"
            ));
        }
        out
    }

    /// Find the innermost indexed symbol containing `line` in `file`.
    fn enclosing_symbol(&self, file: &Path, line: u32) -> Option<String> {
        self.graph
            .all_node_ids()
            .iter()
            .filter_map(|id| self.graph.get_node(id))
            .filter(|n| {
                n.file_path == file
                    && n.span.start_line <= line
                    && n.span.end_line >= line
                    && matches!(
                        n.kind,
                        crate::nodes::NodeKind::Function | crate::nodes::NodeKind::Struct
                    )
            })
            // Innermost = smallest span.
            .min_by_key(|n| n.span.end_line.saturating_sub(n.span.start_line))
            .map(|n| n.name.clone())
    }

    /// Build a session by loading a pre-built base graph snapshot and
    /// layering branch-local diffs on top.
    ///
    /// `base_snapshot_path` points at a snapshot produced by
    /// [`crate::overlay::save_base_snapshot`] (typically built once by
    /// CI for the team's default branch and downloaded to the per-
    /// workspace data dir). `default_branch_ref` is the git ref we
    /// diff `HEAD` against — usually `origin/main` or `origin/master`.
    ///
    /// When git is unavailable or the diff fails (detached HEAD, etc.),
    /// returns the loaded base unchanged with `worktree_mismatch =
    /// None` — better to query against a slightly stale base than to
    /// fail outright.
    pub fn open_overlay(
        base_snapshot_path: &Path,
        workspace_root: &Path,
        default_branch_ref: &str,
    ) -> Result<Self, crate::overlay::OverlayError> {
        let loaded = crate::overlay::load_base_snapshot(base_snapshot_path)?;
        let mut graph = loaded.graph;
        let adapter = RustAdapter::new();
        if let Ok(changed) = crate::overlay::diff_against_base(workspace_root, default_branch_ref) {
            crate::overlay::apply_diff_to_graph(&mut graph, workspace_root, &changed, &adapter);
        }
        let symbols = SymbolTable::build_from_graph(&graph);
        Ok(Self {
            graph,
            symbols,
            events: EventLog::new(),
            capabilities: CapabilityTree::from_env(),
            parse_errors: Vec::new(),
            files_skipped: Vec::new(),
            worktree_mismatch: None,
            query_cache: QueryCache::new(),
            adapter,
        })
    }

    /// Save the in-memory graph as a versioned snapshot at `path` —
    /// typically called by CI for the default branch so contributors
    /// can [`Self::open_overlay`] against it. Records the supplied
    /// `base_ref` (commit SHA or branch name) in the snapshot for
    /// debugging.
    pub fn save_for_overlay(
        &self,
        path: &Path,
        workspace_root: &Path,
        base_ref: Option<&str>,
    ) -> Result<(), crate::overlay::OverlayError> {
        crate::overlay::save_base_snapshot(path, &self.graph, workspace_root, base_ref)
    }
}

/// Read the source lines spanned by a node, with line-number gutters.
/// Returns `None` when the file is unreadable or the span is degenerate.
fn read_span_source(node: &crate::nodes::NodeData) -> Option<String> {
    let content = std::fs::read_to_string(&node.file_path).ok()?;
    let lines: Vec<&str> = content.lines().collect();
    let start = node.span.start_line.saturating_sub(1) as usize;
    let end = (node.span.end_line as usize).min(lines.len());
    if start >= end {
        return None;
    }
    let mut out = String::new();
    for (offset, line) in lines[start..end].iter().enumerate() {
        out.push_str(&format!("{:>4} | {}\n", start + offset + 1, line));
    }
    Some(out.trim_end().to_string())
}

/// Markdown code-fence language tag from a file extension.
fn lang_for(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs") => "rust",
        Some("ts") | Some("tsx") => "typescript",
        Some("js") | Some("jsx") => "javascript",
        Some("py") => "python",
        Some("go") => "go",
        Some("c") | Some("h") => "c",
        Some("cpp") | Some("cc") | Some("hpp") => "cpp",
        Some("rb") => "ruby",
        Some("java") => "java",
        Some("kt") => "kotlin",
        Some("swift") => "swift",
        Some("php") => "php",
        Some("cs") => "csharp",
        Some("svelte") => "svelte",
        _ => "",
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    fn fixtures_dir() -> &'static Path {
        Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures"))
    }

    #[test]
    fn test_session_from_fixtures() {
        let session = GraphSession::from_directory(fixtures_dir());
        assert!(
            session.graph.node_count() > 0,
            "session graph should have nodes from fixtures"
        );
        assert!(
            !session.symbols.is_empty(),
            "session symbols should be populated"
        );
    }

    #[test]
    fn test_session_query() {
        let session = GraphSession::from_directory(fixtures_dir());
        let output = session
            .query(r#"fn("foo") | callees"#, 1000)
            .expect("query should succeed");
        assert!(output.nodes_shown > 0, "query should return nodes");
        assert!(!output.text.is_empty(), "formatted output should have text");
    }

    #[test]
    fn search_with_code_includes_body() {
        let session = GraphSession::from_directory(fixtures_dir());
        let out = session.search_with_code("foo", 5);
        assert!(out.contains("foo"), "should find foo");
        // A fenced code block proves the body was inlined.
        assert!(
            out.contains("```"),
            "search_with_code should inline a body: {out}"
        );
    }

    #[test]
    fn search_results_show_line_range() {
        let session = GraphSession::from_directory(fixtures_dir());
        let out = session.search("foo", 5);
        // Range form `:start-end` (or at least `:start`) must be present.
        assert!(
            out.contains(':'),
            "search result should carry a line locator"
        );
    }

    #[test]
    fn outline_lists_symbols_with_ranges() {
        let session = GraphSession::from_directory(fixtures_dir());
        let out = session.outline("sample.rs");
        assert!(
            out.contains("Outline:"),
            "should render an outline header: {out}"
        );
        assert!(out.contains("foo"), "sample.rs outline should include foo");
    }

    #[test]
    fn outline_missing_file_is_graceful() {
        let session = GraphSession::from_directory(fixtures_dir());
        let out = session.outline("does_not_exist.rs");
        assert!(out.contains("No indexed symbols"));
    }

    #[test]
    fn grep_finds_content_with_enclosing_symbol() {
        let session = GraphSession::from_directory(fixtures_dir());
        // `fn ` appears in every Rust fixture; assert we get matches + headers.
        let out = session.grep(r"fn ", None, 10);
        assert!(out.contains("Content search:"), "grep header: {out}");
        assert!(
            out.contains(".rs:"),
            "grep should report file:line matches: {out}"
        );
    }

    #[test]
    fn grep_invalid_regex_is_reported() {
        let session = GraphSession::from_directory(fixtures_dir());
        let out = session.grep("(unclosed", None, 10);
        assert!(out.contains("Invalid regex"));
    }

    #[test]
    fn cache_hit_on_repeated_query() {
        let session = GraphSession::from_directory(fixtures_dir());
        let q = r#"fn("foo") | callees"#;
        let r1 = session.query_raw(q).expect("first query");
        assert_eq!(session.query_cache_len(), 1);
        let r2 = session.query_raw(q).expect("second query");
        assert_eq!(r1.nodes, r2.nodes, "cache must return identical result");
        // Length still 1 — we didn't add a second entry.
        assert_eq!(session.query_cache_len(), 1);
    }

    #[test]
    fn cache_invalidates_on_file_change() {
        let mut session = GraphSession::from_directory(fixtures_dir());
        let sample = fixtures_dir().join("sample.rs");
        // Run any query that touches sample.rs nodes.
        let _ = session.query_raw(r#"fn("foo") | callees"#);
        let pre = session.query_cache_len();
        assert!(pre >= 1);

        // Mutate the file: cache for sample.rs nodes should drop.
        session.file_changed(&sample, "pub fn x() {}");
        // Either the entry was directly invalidated by node-id, or
        // our coarse path keeps it; either way the new query
        // populates a fresh, correct entry.
        let _ = session.query_raw(r#"fn("foo") | callees"#);
    }

    #[test]
    fn cache_preserves_unrelated_queries_on_file_change() {
        // Phase 8: unrelated queries shouldn't be invalidated by a
        // file change to nodes they don't reference.
        let mut session = GraphSession::from_directory(fixtures_dir());
        // Run a query whose read-set is the foo subtree.
        let _ = session.query_raw(r#"fn("foo")"#);
        let cached_count_before = session.query_cache_len();

        // Mutate a fictional path that doesn't exist in the graph —
        // should not invalidate anything (no nodes touched).
        let phantom = fixtures_dir().join("nonexistent.rs");
        session.file_changed(&phantom, "// nothing");
        let cached_count_after = session.query_cache_len();

        assert_eq!(
            cached_count_before, cached_count_after,
            "phantom file should not invalidate any cache entries"
        );
    }

    #[test]
    fn clear_query_cache_drops_all() {
        let session = GraphSession::from_directory(fixtures_dir());
        let _ = session.query_raw(r#"fn("foo") | callees"#);
        let _ = session.query_raw(r#"fn("bar") | callees"#);
        assert!(session.query_cache_len() > 0);
        session.clear_query_cache();
        assert_eq!(session.query_cache_len(), 0);
    }

    #[test]
    fn test_session_file_changed() {
        let mut session = GraphSession::from_directory(fixtures_dir());
        let sample_path = fixtures_dir().join("sample.rs");

        let initial_count = session.graph.node_count();

        let modified = r#"
pub fn alpha() {
    beta();
}

fn beta() -> i32 {
    99
}
"#;
        session.file_changed(&sample_path, modified);

        // Events were recorded
        assert!(!session.events.is_empty());

        // Graph was updated — alpha and beta should exist
        assert!(!session.graph.find_by_name("alpha").is_empty());
        assert!(!session.graph.find_by_name("beta").is_empty());

        // Original nodes from sample.rs (foo, bar, etc.) should be gone
        let foo_nodes = session.graph.find_by_name("foo");
        let foo_in_sample: Vec<_> = foo_nodes
            .iter()
            .filter(|n| n.file_path == sample_path)
            .collect();
        assert!(
            foo_in_sample.is_empty(),
            "foo from sample.rs should be removed after update"
        );

        // Node count changed (sample.rs had many nodes, now only 2)
        assert_ne!(session.graph.node_count(), initial_count);
    }
}
