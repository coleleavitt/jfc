//! Heavy tool-execution handlers extracted from `execute_tool`.
//!
//! `dispatch::execute_tool` is the O(1) match router; the arms that carry
//! substantial inline logic (graph queries, coverage annotation, symbol
//! edits with cascade, and the bounty market cycle) live here as named
//! functions so the dispatch table stays scannable. Per the rust-lang
//! wg-macros guidance (kpreid): prefer plain functions for logic; keep the
//! dispatch surface thin. Each fn takes exactly the destructured tool
//! arguments plus the ambient `cwd` (and `task_store` where the cascade
//! auto-queue needs it), and returns an `ExecutionResult` — early returns
//! inside translate verbatim from their former match-arm bodies.

use std::path::Path;
use std::sync::Arc;

use crate::runtime::ExecutionResult;
use jfc_session::TaskStore;

use super::economy::{EconomyAgentInvoker, EconomySwarmProvider, apply_winning_solution};
use super::registry::{
    get_or_build_graph_session, invalidate_graph_session_cache, market_orchestrator,
    record_edited_file, record_graph_query, snapshot_active_provider, with_graph_session_mut,
};
use super::safe_tools::maybe_run_slop_guard;

/// `graph_query` tool — run a code-graph DSL query, optionally appending a
/// preconditions block and a chain-able handle footer.
/// Whether the caller asked for JSON output. Default is markdown.
fn wants_json(format: Option<&str>) -> bool {
    matches!(format.map(str::to_ascii_lowercase).as_deref(), Some("json"))
}

pub(super) fn execute_graph_query(
    query: String,
    max_tokens: Option<usize>,
    include_handles: Option<bool>,
    format: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    if wants_json(format) {
        let session = get_or_build_graph_session(cwd);
        return match session.query_raw(&query) {
            Ok(result) => {
                let env = jfc_graph::schema::wrap_query_result(result);
                match serde_json::to_string_pretty(&env) {
                    Ok(s) => ExecutionResult::success(s),
                    Err(e) => ExecutionResult::failure(format!("JSON serialization error: {e}")),
                }
            }
            Err(e) => ExecutionResult::failure(format!("Graph query error: {e}")),
        };
    }
    let budget = max_tokens.unwrap_or(4000);
    let want_handles = include_handles.unwrap_or(true);
    let session = get_or_build_graph_session(cwd);
    // Run twice: once raw (so we can record the structured
    // QueryResult to history *and* extract chain-able handles)
    // and once formatted with the budget. The raw call is
    // cheap — same parse, just skips the formatting pass —
    // and the alternative (changing format_query_result to
    // also expose the QueryResult) would touch the jfc-graph
    // public API.
    let raw_for_predicates = session.query_raw(&query).ok();
    if let Some(ref raw) = raw_for_predicates {
        record_graph_query(&query, raw);
    }
    match session.query(&query, budget) {
        Ok(output) => {
            let mut text = output.text.clone();
            // Magic's path-dependent analysis: when the
            // query asked for `preconditions`, append the
            // enclosing if/match/while predicate at every
            // outgoing call site of each caller. The model
            // sees "to call X you must have passed (a > 0)"
            // without having to grep for callers manually.
            if query.contains("preconditions")
                && let Some(ref raw) = raw_for_predicates
            {
                let mut preds_block = String::new();
                for node_id in raw.nodes.iter().take(10) {
                    let preds =
                        jfc_graph::predicates::outgoing_call_predicates(&session.graph, node_id);
                    if preds.is_empty() {
                        continue;
                    }
                    if let Some(node) = session.graph.get_node(node_id) {
                        preds_block.push_str(&format!(
                            "\n  • {} ({}):\n",
                            node.name,
                            node.file_path.display()
                        ));
                    }
                    for (target, ps) in preds.iter().take(3) {
                        let chain = ps
                            .iter()
                            .map(|p| p.text.as_str())
                            .collect::<Vec<_>>()
                            .join(" → ");
                        preds_block.push_str(&format!("      → {target}: {chain}\n"));
                    }
                }
                if !preds_block.is_empty() {
                    text.push_str("\n\n--- preconditions ---");
                    text.push_str(&preds_block);
                }
            }
            // Append a machine-parseable handle footer so the
            // model can pipe this query's matches into the
            // next turn (e.g. `path fn:foo → fn:bar`). Bounded
            // at 50 entries to keep the budget bite small even
            // when a query returns hundreds of nodes.
            if want_handles && let Some(ref raw) = raw_for_predicates {
                let handles = raw.handles(&session.graph);
                if !handles.is_empty() {
                    text.push_str("\n\n--- handles ---");
                    const HANDLE_CAP: usize = 50;
                    let total = handles.len();
                    for h in handles.iter().take(HANDLE_CAP) {
                        text.push('\n');
                        text.push_str(h);
                    }
                    if total > HANDLE_CAP {
                        text.push_str(&format!(
                            "\n... and {} more (use a tighter query to see all)",
                            total - HANDLE_CAP
                        ));
                    }
                }
            }
            if output.was_truncated {
                ExecutionResult::success(format!(
                    "{text}\n\n[Showing {}/{} nodes]",
                    output.nodes_shown, output.nodes_total
                ))
            } else {
                ExecutionResult::success(text)
            }
        }
        Err(e) => ExecutionResult::failure(format!("Graph query error: {e}")),
    }
}

/// `graph_context` tool — build a codegraph_context-style markdown
/// payload from a free-form task description. Composes search + BFS
/// + type-hierarchy expansion + per-file diversity cap into one
/// agent-friendly answer with feature/bug/exploration intent
/// reminders. Use this FIRST for "how does X work" / architecture
/// questions instead of chaining graph_search + graph_query.
pub(super) fn execute_graph_context(
    task: String,
    max_nodes: Option<usize>,
    include_code: Option<bool>,
    format: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let opts = jfc_graph::context::ContextOptions {
        max_nodes: max_nodes.unwrap_or(20).clamp(1, 100),
        include_code: include_code.unwrap_or(true),
        traversal_depth: 1,
    };
    let result = session.context(&task, opts);
    if wants_json(format) {
        let env = jfc_graph::schema::wrap_context_result(&result);
        return match serde_json::to_string_pretty(&env) {
            Ok(s) => ExecutionResult::success(s),
            Err(e) => ExecutionResult::failure(format!("JSON serialization error: {e}")),
        };
    }
    ExecutionResult::success(result.markdown)
}

/// `graph_search` tool — find symbols by name with qualified-name
/// support (`crate::module::sym`, `Class.method`, etc.). Returns kind,
/// location, signature, and handle for each hit.
pub(super) fn execute_graph_search(
    query: String,
    limit: Option<usize>,
    include_code: bool,
    format: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let n = limit.unwrap_or(10).clamp(1, 100);
    if wants_json(format) {
        let hits: Vec<_> = session.resolve(&query).into_iter().take(n).collect();
        let payload = serde_json::json!({
            "schema_version": jfc_graph::schema::SCHEMA_VERSION,
            "kind": "graph_search",
            "data": {
                "query": query,
                "hits": hits,
            }
        });
        return match serde_json::to_string_pretty(&payload) {
            Ok(s) => ExecutionResult::success(s),
            Err(e) => ExecutionResult::failure(format!("JSON serialization error: {e}")),
        };
    }
    // include_code collapses the search→sed loop: each hit renders with its
    // full source body in one call.
    if include_code {
        return ExecutionResult::success(session.search_with_code(&query, n));
    }
    ExecutionResult::success(session.search(&query, n))
}

/// `graph_outline` tool — structural map of one file (every symbol + line
/// range), replacing the `nl -ba file` line-number lookup pattern.
pub(super) fn execute_graph_outline(file: String, cwd: &Path) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    ExecutionResult::success(session.outline(&file))
}

/// `graph_grep` tool — regex content search over indexed files, each match
/// enriched with its enclosing symbol. Serves log/error/string-literal
/// searches the symbol index can't answer.
pub(super) fn execute_graph_grep(
    pattern: String,
    glob: Option<&str>,
    limit: Option<usize>,
    cwd: &Path,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let n = limit.unwrap_or(50).clamp(1, 500);
    ExecutionResult::success(session.grep(&pattern, glob, n))
}

/// `graph_callers` tool — find every function that calls `symbol`,
/// rendered file-grouped with signatures inline.
pub(super) fn execute_graph_callers(
    symbol: String,
    limit: Option<usize>,
    format: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let n = limit.unwrap_or(20).clamp(1, 100);
    if wants_json(format) {
        let (nodes, note) = jfc_graph::context::callers_for(&session.graph, &symbol, n);
        return json_neighbors("graph_callers", &symbol, &nodes, note.as_deref());
    }
    ExecutionResult::success(session.callers(&symbol, n))
}

/// `graph_callees` tool — find every function `symbol` calls,
/// rendered file-grouped with signatures inline.
pub(super) fn execute_graph_callees(
    symbol: String,
    limit: Option<usize>,
    format: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let n = limit.unwrap_or(20).clamp(1, 100);
    if wants_json(format) {
        let (nodes, note) = jfc_graph::context::callees_for(&session.graph, &symbol, n);
        return json_neighbors("graph_callees", &symbol, &nodes, note.as_deref());
    }
    ExecutionResult::success(session.callees(&symbol, n))
}

/// `graph_impact` tool — walk incoming calls outward to surface
/// every symbol whose behaviour might shift if `symbol` changes.
/// Output is grouped by file with `name:line` inline lists.
pub(super) fn execute_graph_impact(
    symbol: String,
    depth: Option<u8>,
    format: Option<&str>,
    cwd: &Path,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let d = depth.unwrap_or(2).clamp(1, 10);
    if wants_json(format) {
        let (nodes, note) = jfc_graph::context::impact_for(&session.graph, &symbol, d);
        return json_neighbors("graph_impact", &symbol, &nodes, note.as_deref());
    }
    ExecutionResult::success(session.impact(&symbol, d))
}

/// `graph_node` tool — get detailed info about ONE symbol (location,
/// signature, visibility, source). For container types, renders a
/// compact member outline rather than the full body.
pub(super) fn execute_graph_node(
    symbol: String,
    include_code: bool,
    cwd: &Path,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    ExecutionResult::success(session.node(&symbol, include_code))
}

/// `graph_explore` tool — returns source for SEVERAL related symbols
/// grouped by file, plus a relationship map, in ONE capped call.
pub(super) fn execute_graph_explore(
    query: String,
    max_files: Option<usize>,
    cwd: &Path,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let n = max_files.unwrap_or(12).clamp(1, 50);
    ExecutionResult::success(session.explore(&query, n))
}

pub(super) fn execute_graph_status(cwd: &Path) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let graph = &session.graph;
    let node_count = graph.node_count();
    let edge_count = graph.edge_count();
    let file_count: usize = {
        let mut files = std::collections::HashSet::new();
        for id in graph.all_node_ids() {
            if let Some(node) = graph.get_node(id) {
                files.insert(node.file_path.clone());
            }
        }
        files.len()
    };
    let warm = session.content_cache_warm_files();
    ExecutionResult::success(format!(
        "Graph status: {node_count} nodes, {edge_count} edges, {file_count} indexed files\n\
         Content cache: {warm} files warm (graph_grep)\n\
         Root: {}\n\
         Watcher: active",
        cwd.display()
    ))
}

pub(super) fn execute_graph_files(path_filter: Option<&str>, cwd: &Path) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let graph = &session.graph;
    let mut files = std::collections::BTreeSet::new();
    for id in graph.all_node_ids() {
        if let Some(node) = graph.get_node(id) {
            let display = node
                .file_path
                .strip_prefix(cwd)
                .unwrap_or(&node.file_path)
                .display()
                .to_string()
                .replace('\\', "/");
            files.insert(display);
        }
    }

    let mut files: Vec<String> = files.into_iter().collect();
    if let Some(filter) = path_filter {
        let f = filter.to_ascii_lowercase();
        files.retain(|p| p.to_ascii_lowercase().contains(&f));
    }

    let total = files.len();
    let shown = total.min(200);
    let mut out = format!("{shown}/{total} indexed files");
    if let Some(filter) = path_filter {
        out.push_str(&format!(" (filter: {filter})"));
    }
    out.push('\n');
    for f in files.iter().take(200) {
        out.push_str(f);
        out.push('\n');
    }
    if total > shown {
        out.push_str(&format!(
            "... and {} more (narrow with `path` filter)\n",
            total - shown
        ));
    }
    ExecutionResult::success(out)
}

/// Render a neighbour list (callers / callees / impact) as a versioned
/// JSON envelope. Centralised so the three tools stay in sync on shape.
fn json_neighbors(
    kind: &str,
    symbol: &str,
    nodes: &[jfc_graph::nodes::NodeId],
    note: Option<&str>,
) -> ExecutionResult {
    let payload = serde_json::json!({
        "schema_version": jfc_graph::schema::SCHEMA_VERSION,
        "kind": kind,
        "data": {
            "symbol": symbol,
            "nodes": nodes,
            "note": note,
        }
    });
    match serde_json::to_string_pretty(&payload) {
        Ok(s) => ExecutionResult::success(s),
        Err(e) => ExecutionResult::failure(format!("JSON serialization error: {e}")),
    }
}

/// `run_coverage` tool — parse/collect lcov, annotate the graph with hit
/// counts, then run possible-types propagation.
pub(super) fn execute_run_coverage(
    lcov_path: Option<String>,
    include_untested_list: bool,
    cwd: &Path,
) -> ExecutionResult {
    use jfc_graph::coverage::{annotate_graph_from_lcov, parse_lcov};
    use jfc_graph::possible_types::propagate_possible_types;

    let lcov_result = if let Some(ref path) = lcov_path {
        let file = match std::fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                return ExecutionResult::failure(format!("Failed to open lcov file {path}: {e}"));
            }
        };
        let reader = std::io::BufReader::new(file);
        Ok(parse_lcov(reader))
    } else {
        // Run cargo llvm-cov to generate lcov output.
        let output = std::process::Command::new("cargo")
            .args(["llvm-cov", "--lcov", "--output-path", "-"])
            .current_dir(cwd)
            .output();
        match output {
            Ok(out) if out.status.success() => {
                let reader = std::io::BufReader::new(std::io::Cursor::new(out.stdout));
                Ok(parse_lcov(reader))
            }
            Ok(out) => Err(format!(
                "cargo llvm-cov failed (exit {}):\n{}",
                out.status,
                String::from_utf8_lossy(&out.stderr)
            )),
            Err(e) => Err(format!(
                "Failed to run cargo llvm-cov: {e}. \
                 Install with: rustup component add llvm-tools && cargo install cargo-llvm-cov"
            )),
        }
    };

    match with_graph_session_mut(cwd, |session| {
        let mut summary = String::new();

        match lcov_result {
            Ok((lcov_data, warnings)) => {
                let (annotated, untested) =
                    annotate_graph_from_lcov(&mut session.graph, &lcov_data, cwd);
                let tested = annotated - untested;

                summary.push_str(&format!(
                    "Coverage annotated: {annotated} functions ({tested} tested, {untested} untested)"
                ));
                if warnings > 0 {
                    summary.push_str(&format!(", {warnings} lcov parse warnings"));
                }

                // List untested functions if requested.
                if include_untested_list && untested > 0 {
                    summary.push_str("\n\nUntested functions:");
                    let mut count = 0;
                    for node in session
                        .graph
                        .nodes_by_kind(jfc_graph::nodes::NodeKind::Function)
                    {
                        if node.metadata.get("coverage_tested").map(|v| v.as_str()) == Some("false")
                        {
                            summary.push_str(&format!(
                                "\n  - {} ({}:{})",
                                node.qualified_name,
                                node.file_path.display(),
                                node.span.start_line,
                            ));
                            count += 1;
                            if count >= 100 {
                                summary.push_str(&format!(
                                    "\n  ... and {} more (use `graph_query` with `untested` to see all)",
                                    untested - count
                                ));
                                break;
                            }
                        }
                    }
                }
            }
            Err(e) => {
                summary.push_str(&format!("Coverage collection failed: {e}\n\n"));
                summary.push_str(
                    "Skipping coverage annotation, running possible-types analysis only.",
                );
            }
        }

        // Step 2: Always run possible-types propagation.
        let (pt_annotated, pt_inputs, pt_returns) = propagate_possible_types(&mut session.graph);
        summary.push_str(&format!(
            "\n\nPossible-types propagated: {pt_annotated} functions, \
         {pt_inputs} input type entries, {pt_returns} return type entries"
        ));
        summary.push_str("\n\nUse `graph_query` with:");
        summary.push_str("\n  - `untested` operator to filter to uncovered functions");
        summary.push_str("\n  - `possible_types` operator to see type flow per function");
        summary.push_str("\n  Example: `entrypoints kind=PublicApi | untested`");
        summary.push_str("\n  Example: `fn(\"handler\") | possible_types`");

        ExecutionResult::success(summary)
    }) {
        Ok(result) => result,
        Err(message) => ExecutionResult::failure(message),
    }
}

/// `symbol_edit` tool — replace a symbol's span by handle, optionally
/// computing (and auto-queuing) the caller cascade.
pub(super) async fn execute_symbol_edit(
    handle: String,
    new_content: String,
    validate: bool,
    dispatch_cascade: bool,
    cwd: &Path,
    task_store: Option<Arc<TaskStore>>,
) -> ExecutionResult {
    let session = get_or_build_graph_session(cwd);
    let entry = match session.symbols().resolve(&handle) {
        Some(e) => e.clone(),
        None => {
            let fuzzy = session.symbols().resolve_fuzzy(&handle);
            if fuzzy.is_empty() {
                return ExecutionResult::failure(format!(
                    "Symbol not found: '{}'. Use graph_query to discover handles.",
                    handle
                ));
            }
            return ExecutionResult::failure(format!(
                "Symbol '{}' not found. Did you mean: {}?",
                handle,
                fuzzy
                    .iter()
                    .take(5)
                    .map(|e| e.handle.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    };

    // v131-style cascade: when the edit changes a function
    // signature, the surrounding call sites likely need
    // updating too. Generate per-file CascadeTask descriptors
    // and surface them in the tool's success string so the
    // model knows what it needs to fix next without having
    // to grep for callers itself. Validation runs first so
    // an obviously-broken edit blocks before we touch disk.
    let mut cascade_summary = String::new();
    if validate {
        let cascade = jfc_graph::cascade::generate_cascade(
            &session.graph,
            &entry.node_id,
            new_content.lines().next().unwrap_or("").trim(),
            &format!("symbol_edit on '{handle}'"),
        );
        if !cascade.is_empty() {
            let total_sites: usize = cascade.iter().map(|t| t.call_sites.len()).sum();
            let mut summary = format!(
                "\n\n--- cascade ---\n{} call site{} across {} file{} may need updating:",
                total_sites,
                if total_sites == 1 { "" } else { "s" },
                cascade.len(),
                if cascade.len() == 1 { "" } else { "s" }
            );
            for task in &cascade {
                summary.push_str(&format!(
                    "\n  - {} ({} site{}): {}",
                    task.call_sites
                        .first()
                        .map(|s| s.file_path.display().to_string())
                        .unwrap_or_default(),
                    task.call_sites.len(),
                    if task.call_sites.len() == 1 { "" } else { "s" },
                    task.call_sites
                        .iter()
                        .map(|s| s.caller_name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            summary.push_str("\nDispatch the Task tool per file to update them in parallel.");
            cascade_summary = summary;
            tracing::info!(
                target: "jfc::tools",
                sites = total_sites,
                files = cascade.len(),
                "symbol_edit produced cascade"
            );
            // Optional auto-queue: when the caller passed
            // `dispatch_cascade=true` AND a TaskStore is
            // available, drop one entry per file into the
            // store so the user (and the model, via /tasks)
            // sees the cascade plan as concrete trackable
            // work. metadata.kind = "cascade" lets the UI
            // and `/cascade` filter for these specifically.
            if dispatch_cascade && let Some(ts) = task_store.as_ref() {
                let mut queued_ids: Vec<String> = Vec::new();
                for ct in &cascade {
                    let file_disp = ct
                        .call_sites
                        .first()
                        .map(|s| s.file_path.display().to_string())
                        .unwrap_or_else(|| "<unknown>".to_owned());
                    let subject = format!(
                        "Update {} call site{} in {}",
                        ct.call_sites.len(),
                        if ct.call_sites.len() == 1 { "" } else { "s" },
                        file_disp,
                    );
                    let active = format!("Updating call sites in {file_disp}");
                    match ts.create::<jfc_session::TaskId>(
                        subject,
                        ct.instruction.clone(),
                        Some(active),
                        Vec::new(),
                    ) {
                        Ok(t) => {
                            let metadata = serde_json::json!({
                                "kind": "cascade",
                                "source_handle": handle,
                                "file": file_disp,
                                "callers": ct
                                    .call_sites
                                    .iter()
                                    .map(|s| s.caller_name.clone())
                                    .collect::<Vec<_>>(),
                                "new_signature": ct.new_signature,
                            });
                            let _ = ts.update(
                                t.id.as_str(),
                                jfc_session::TaskPatch {
                                    metadata: Some(metadata),
                                    ..Default::default()
                                },
                            );
                            queued_ids.push(t.id.to_string());
                        }
                        Err(e) => {
                            tracing::warn!(
                                target: "jfc::tools",
                                error = %e,
                                "cascade task create failed"
                            );
                        }
                    }
                }
                if !queued_ids.is_empty() {
                    cascade_summary.push_str(&format!(
                        "\n\nQueued {} cascade task{} ({}). Use the Task tool with the \
                         task IDs above as descriptions, or run /cascade to view them.",
                        queued_ids.len(),
                        if queued_ids.len() == 1 { "" } else { "s" },
                        queued_ids.join(", "),
                    ));
                }
            }
        }
    }

    let file_content = match std::fs::read_to_string(&entry.file_path) {
        Ok(c) => c,
        Err(e) => return ExecutionResult::failure(format!("Read failed: {e}")),
    };

    let start = entry.span.byte_range.start;
    let end = entry.span.byte_range.end;
    if end > file_content.len() {
        return ExecutionResult::failure("Span out of bounds — file changed since graph was built");
    }

    let new_file = format!(
        "{}{}{}",
        &file_content[..start],
        new_content,
        &file_content[end..]
    );
    if let Err(e) = std::fs::write(&entry.file_path, &new_file) {
        return ExecutionResult::failure(format!("Write failed: {e}"));
    }
    // Invalidate the cached graph session for this workspace so
    // the next graph_query re-parses the modified file and the
    // user sees the symbol's new shape. Also queue the file
    // for auto-context injection on the next stream call.
    invalidate_graph_session_cache(Some(cwd));
    record_edited_file(&entry.file_path);

    let result = ExecutionResult::success(format!(
        "Edited symbol '{}' in {}{}",
        handle,
        entry.file_path.display(),
        cascade_summary
    ));
    // Slop guard: check the new file content for quality issues.
    maybe_run_slop_guard(result, &entry.file_path, &new_file, cwd).await
}

/// `post_bounty` tool — register a bounty and (when `auto_dispatch`) drive
/// the full Solve→Validate→Settle cycle.
pub(super) async fn execute_post_bounty(
    description: String,
    budget: u64,
    acceptance_criteria: String,
    max_solvers: Option<u8>,
    auto_dispatch: bool,
    cwd: &Path,
) -> ExecutionResult {
    // The orchestrator's lock is process-global; only one
    // post_bounty runs at a time. That's fine — bounties are
    // posted in the LLM's main loop, not from concurrent
    // subagents. If two tool calls race, the second waits.
    //
    // Posting always succeeds first. If `auto_dispatch=true`,
    // we then drop the lock, run the cycle (which spawns
    // real subagent LLM calls and can take minutes), and
    // re-acquire the lock to read the settlement. Holding
    // the orchestrator mutex across the network round-trips
    // would block /market and concurrent post_bounty calls.
    let bounty_id = {
        let mut orch = market_orchestrator().lock().await;
        match orch.post_bounty(description, budget, acceptance_criteria, max_solvers) {
            Ok(id) => id,
            Err(e) => {
                return ExecutionResult::failure(format!("post_bounty failed: {e}"));
            }
        }
    };
    let max_solvers_text = match max_solvers {
        Some(n) => n.to_string(),
        None => {
            let orch = market_orchestrator().lock().await;
            orch.charter().max_solvers.to_string()
        }
    };
    if !auto_dispatch {
        return ExecutionResult::success(format!(
            "Bounty `{bounty_id}` registered. State=Open, budget={budget} tok, \
             max_solvers={max_solvers_text}. Solvers and validators have NOT \
             run yet — the post step only registers the bounty in the market. \
             To execute the full Post→Solve→Validate→Settle cycle (real LLM \
             subagents compete + cross-validate), call run_bounty with \
             bounty_id=\"{bounty_id}\". Or repost with auto_dispatch=true to \
             register and run in one shot."
        ));
    }
    // Drive the real cycle. The orchestrator mutex is
    // dropped before the await so /market and concurrent
    // post_bounty calls aren't blocked across the network
    // round-trips.
    let Some((provider, model)) = snapshot_active_provider() else {
        return ExecutionResult::success(format!(
            "Bounty `{bounty_id}` registered (budget {budget} tok, \
             max_solvers={max_solvers_text}, State=Open). \
             auto_dispatch=true was requested but the tool layer \
             has no active provider registered, so the cycle did \
             not run. The bounty stays Open — call run_bounty \
             once the provider is wired."
        ));
    };
    let invoker = EconomyAgentInvoker::new(provider, model);
    let swarm = EconomySwarmProvider::new(cwd.to_path_buf());
    // Solver + validator counts: respect the bounty's
    // max_solvers, default to 2 to keep the per-bounty
    // round-trip count predictable. One validator per
    // surviving solution — sealed validation gives one
    // independent verdict per solver.
    let n_solvers = max_solvers.unwrap_or(2).clamp(1, 5);
    tracing::info!(
        target: "jfc::ui::bounty",
        bounty_id = %bounty_id,
        n_solvers = n_solvers,
        cwd = %cwd.display(),
        "post_bounty auto_dispatch: kicking off cycle"
    );
    let cycle_result = {
        let mut orch = market_orchestrator().lock().await;
        orch.run_bounty_cycle(&bounty_id, &invoker, &swarm, n_solvers, 1)
            .await
    };
    match cycle_result {
        Ok(outcome) => {
            let written =
                apply_winning_solution(cwd, &bounty_id, outcome.winning_solution.as_ref());
            tracing::info!(
                target: "jfc::ui::bounty",
                bounty_id = %bounty_id,
                winner = outcome.settlement.winner.as_ref().map(|a| a.0.as_str()).unwrap_or("(none)"),
                files_written = written.files.len(),
                "post_bounty auto_dispatch settled"
            );
            ExecutionResult::success(format!(
                "Bounty `{bounty_id}` settled.\n\
                 Winner: {}\n\
                 Total cost: {} tok\n\
                 Payouts: {}\n\
                 Trust updates: {}\n\
                 {}\n\
                 Run /market to see updated trust + budget.",
                outcome
                    .settlement
                    .winner
                    .as_ref()
                    .map(|a| a.0.as_str())
                    .unwrap_or("(no winning solution)"),
                outcome.settlement.total_cost,
                outcome.settlement.payouts.len(),
                outcome.settlement.trust_updates.len(),
                written.summary,
            ))
        }
        Err(e) => {
            ExecutionResult::failure(format!("auto_dispatch cycle for `{bounty_id}` failed: {e}"))
        }
    }
}

/// `run_bounty` tool — drive an already-posted Open bounty through the full
/// Solve→Validate→Settle cycle.
pub(super) async fn execute_run_bounty(
    bounty_id: String,
    max_solvers: Option<u8>,
    cwd: &Path,
) -> ExecutionResult {
    // Drive an already-posted Open bounty through the full
    // Solve→Validate→Settle cycle. Same code path as
    // PostBounty's auto_dispatch=true, just without the
    // post step. Lets the model post first (cheap registration)
    // and dispatch later when ready, instead of all-or-nothing.
    let Some((provider, model)) = snapshot_active_provider() else {
        return ExecutionResult::failure(
            "run_bounty: no active provider registered with the \
             tool layer. main.rs must call \
             tools::register_active_provider during startup.",
        );
    };
    // Verify the bounty exists and is in Open state before
    // we go through all the worktree + LLM-call setup.
    let state = {
        let orch = market_orchestrator().lock().await;
        orch.bounty_state(&bounty_id)
    };
    let Some(state) = state else {
        return ExecutionResult::failure(format!("run_bounty: bounty `{bounty_id}` not found"));
    };
    if !matches!(state, jfc_economy::types::MarketState::Open) {
        return ExecutionResult::failure(format!(
            "run_bounty: bounty `{bounty_id}` is in state {state:?}, \
             not Open — only Open bounties can be dispatched"
        ));
    }
    let invoker = EconomyAgentInvoker::new(provider, model);
    let swarm = EconomySwarmProvider::new(cwd.to_path_buf());
    let n_solvers = max_solvers.unwrap_or(2).clamp(1, 5);
    tracing::info!(
        target: "jfc::ui::bounty",
        bounty_id = %bounty_id,
        n_solvers = n_solvers,
        cwd = %cwd.display(),
        "run_bounty: kicking off cycle"
    );
    let cycle_result = {
        let mut orch = market_orchestrator().lock().await;
        orch.run_bounty_cycle(&bounty_id, &invoker, &swarm, n_solvers, 1)
            .await
    };
    match cycle_result {
        Ok(outcome) => {
            let written =
                apply_winning_solution(cwd, &bounty_id, outcome.winning_solution.as_ref());
            tracing::info!(
                target: "jfc::ui::bounty",
                bounty_id = %bounty_id,
                winner = outcome.settlement.winner.as_ref().map(|a| a.0.as_str()).unwrap_or("(none)"),
                files_written = written.files.len(),
                "run_bounty settled"
            );
            ExecutionResult::success(format!(
                "Bounty `{bounty_id}` settled.\n\
                 Winner: {}\n\
                 Total cost: {} tok\n\
                 Payouts: {}\n\
                 Trust updates: {}\n\
                 {}\n\
                 Run /market or market_status to see updated trust + budget.",
                outcome
                    .settlement
                    .winner
                    .as_ref()
                    .map(|a| a.0.as_str())
                    .unwrap_or("(no winning solution)"),
                outcome.settlement.total_cost,
                outcome.settlement.payouts.len(),
                outcome.settlement.trust_updates.len(),
                written.summary,
            ))
        }
        Err(e) => {
            ExecutionResult::failure(format!("run_bounty cycle for `{bounty_id}` failed: {e}"))
        }
    }
}
