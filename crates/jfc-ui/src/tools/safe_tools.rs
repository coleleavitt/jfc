/// Synchronous helpers, safe-tool executors, and utility functions used by
/// the tool dispatcher. Nothing in here touches global process state
/// directly — callers go through `registry` for that.
use std::path::Path;
use std::process::Stdio;

use jfc_graph::nodes::{NodeData, NodeKind, Visibility};
use std::collections::BTreeMap;
use tokio::process::Command;

use crate::runtime::ExecutionResult;

use super::defs::all_tool_defs;
use super::registry::snapshot_mcp_registry;

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
}

// ---------------------------------------------------------------------------
// Code-index constants and helpers
// ---------------------------------------------------------------------------

pub(super) const CODE_INDEX_DEFAULT_LIMIT: usize = 80;
pub(super) const CODE_INDEX_MAX_LIMIT: usize = 200;

pub(super) fn execute_code_index(
    cwd: &Path,
    path: Option<&str>,
    query: Option<&str>,
    kind: Option<&str>,
    max_entries: Option<usize>,
) -> ExecutionResult {
    let kind_filter = match kind.and_then(trim_nonempty) {
        Some(raw) => match parse_code_index_kind(raw) {
            Some(kind) => Some(kind),
            None => {
                return ExecutionResult::failure(format!(
                    "code_index kind must be one of: function, struct, enum, module, trait (got {raw:?})"
                ));
            }
        },
        None => None,
    };

    let path_filter = path.and_then(trim_nonempty).map(normalize_filter);
    let query_filter = query.and_then(trim_nonempty).map(normalize_filter);
    let limit = max_entries
        .unwrap_or(CODE_INDEX_DEFAULT_LIMIT)
        .clamp(1, CODE_INDEX_MAX_LIMIT);

    let session = super::registry::get_or_build_graph_session(cwd);
    let mut nodes = session
        .graph
        .all_node_ids()
        .into_iter()
        .filter_map(|id| session.graph.get_node(id))
        .filter(|node| {
            kind_filter.is_none_or(|kind| node.kind == kind)
                && path_filter
                    .as_deref()
                    .is_none_or(|filter| code_index_path_matches(cwd, node, filter))
                && query_filter
                    .as_deref()
                    .is_none_or(|filter| code_index_query_matches(cwd, node, filter))
        })
        .collect::<Vec<_>>();

    nodes.sort_by(|a, b| {
        code_index_display_path(cwd, &a.file_path)
            .cmp(&code_index_display_path(cwd, &b.file_path))
            .then(a.span.start_line.cmp(&b.span.start_line))
            .then(a.kind.cmp(&b.kind))
            .then(a.qualified_name.cmp(&b.qualified_name))
    });

    let total_matching = nodes.len();
    let shown = total_matching.min(limit);
    let mut by_file: BTreeMap<String, Vec<&NodeData>> = BTreeMap::new();
    for node in nodes.into_iter().take(limit) {
        by_file
            .entry(code_index_display_path(cwd, &node.file_path))
            .or_default()
            .push(node);
    }

    let mut out = String::new();
    out.push_str(&format!(
        "Code index: {shown}/{total_matching} matching symbols shown · graph {} nodes / {} edges",
        session.graph.node_count(),
        session.graph.edge_count()
    ));

    let filters = code_index_filter_summary(path, query, kind);
    if !filters.is_empty() {
        out.push_str(&format!("\nfilters: {}", filters.join(", ")));
    }
    out.push_str("\nUse handles with graph_query or symbol_edit.");

    if by_file.is_empty() {
        out.push_str("\n\nNo symbols matched.");
        return ExecutionResult::success(out);
    }

    for (file, file_nodes) in by_file {
        out.push_str("\n\n");
        out.push_str(&file);
        for node in file_nodes {
            let incoming = session.graph.get_edges_to(&node.id).len();
            let outgoing = session.graph.get_edges_from(&node.id).len();
            let metadata = code_index_metadata_summary(node);
            out.push_str(&format!(
                "\n  {} {} lines {}-{} · {} · in {} / out {} · {}",
                code_index_kind_label(node.kind),
                node.qualified_name,
                node.span.start_line,
                node.span.end_line,
                code_index_visibility_label(&node.visibility),
                incoming,
                outgoing,
                code_index_handle(node)
            ));
            if !metadata.is_empty() {
                out.push_str(" · ");
                out.push_str(&metadata.join(", "));
            }
        }
    }

    if total_matching > shown {
        out.push_str(&format!(
            "\n\n... and {} more (use path/query/kind or raise max_entries up to {CODE_INDEX_MAX_LIMIT})",
            total_matching - shown
        ));
    }

    ExecutionResult::success(out)
}

pub(super) fn trim_nonempty(value: &str) -> Option<&str> {
    let value = value.trim();
    (!value.is_empty()).then_some(value)
}

fn normalize_filter(value: &str) -> String {
    value.replace('\\', "/").to_ascii_lowercase()
}

fn parse_code_index_kind(kind: &str) -> Option<NodeKind> {
    match kind
        .trim()
        .to_ascii_lowercase()
        .replace(['_', '-'], "")
        .as_str()
    {
        "fn" | "func" | "function" => Some(NodeKind::Function),
        "struct" => Some(NodeKind::Struct),
        "enum" => Some(NodeKind::Enum),
        "mod" | "module" => Some(NodeKind::Module),
        "trait" => Some(NodeKind::Trait),
        "enumvariant" | "variant" => Some(NodeKind::EnumVariant),
        "field" | "property" => Some(NodeKind::Field),
        "typealias" | "type" => Some(NodeKind::TypeAlias),
        "constant" | "const" | "static" => Some(NodeKind::Constant),
        "interface" => Some(NodeKind::Interface),
        _ => None,
    }
}

fn code_index_path_matches(cwd: &Path, node: &NodeData, filter: &str) -> bool {
    normalize_filter(&code_index_display_path(cwd, &node.file_path)).contains(filter)
        || normalize_filter(&node.file_path.display().to_string()).contains(filter)
}

fn code_index_query_matches(cwd: &Path, node: &NodeData, filter: &str) -> bool {
    normalize_filter(&node.name).contains(filter)
        || normalize_filter(&node.qualified_name).contains(filter)
        || code_index_path_matches(cwd, node, filter)
}

fn code_index_display_path(cwd: &Path, path: &Path) -> String {
    let display_path = path.strip_prefix(cwd).unwrap_or(path);
    display_path.display().to_string().replace('\\', "/")
}

fn code_index_filter_summary(
    path: Option<&str>,
    query: Option<&str>,
    kind: Option<&str>,
) -> Vec<String> {
    let mut filters = Vec::new();
    if let Some(kind) = kind.and_then(trim_nonempty) {
        filters.push(format!("kind={kind}"));
    }
    if let Some(query) = query.and_then(trim_nonempty) {
        filters.push(format!("query={query}"));
    }
    if let Some(path) = path.and_then(trim_nonempty) {
        filters.push(format!("path={path}"));
    }
    filters
}

fn code_index_kind_label(kind: NodeKind) -> &'static str {
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

fn code_index_visibility_label(visibility: &Visibility) -> &'static str {
    match visibility {
        Visibility::Public => "pub",
        Visibility::Crate => "crate",
        Visibility::Super => "super",
        Visibility::Private => "private",
    }
}

fn code_index_handle(node: &NodeData) -> String {
    format!(
        "{}:{}",
        match node.kind {
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
        },
        node.qualified_name
    )
}

fn code_index_metadata_summary(node: &NodeData) -> Vec<String> {
    let mut parts = Vec::new();
    match node.kind {
        NodeKind::Function => {
            if node
                .metadata
                .get("async")
                .is_some_and(|value| matches!(value.as_str(), "true" | "1"))
            {
                parts.push("async".to_owned());
            }
            if let Some(params) = node.metadata.get("param_count") {
                parts.push(format!("params={params}"));
            }
            if let Some(tested) = node.metadata.get("coverage_tested") {
                parts.push(format!("tested={tested}"));
            }
        }
        NodeKind::Struct => {
            if let Some(fields) = node.metadata.get("field_count") {
                parts.push(format!("fields={fields}"));
            }
        }
        NodeKind::Enum => {
            if let Some(variants) = node.metadata.get("variant_count") {
                parts.push(format!("variants={variants}"));
            }
        }
        NodeKind::Trait => {
            if let Some(methods) = node.metadata.get("method_count") {
                parts.push(format!("methods={methods}"));
            }
        }
        NodeKind::Module => {}
        _ => {}
    }
    parts
}

// ---------------------------------------------------------------------------
// Tool search / suggest
// ---------------------------------------------------------------------------

pub async fn all_tool_defs_with_mcp() -> Vec<jfc_provider::ToolDef> {
    let mut tools = all_tool_defs();
    let builtin_names: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name.clone()).collect();
    if let Some(registry) = snapshot_mcp_registry() {
        for tool in registry.all_advertised_tool_defs().await {
            // Codegraph #284: external MCP servers occasionally advertise
            // tool names that double our own prefix (e.g. an MCP server
            // re-publishes a `graph_search` tool when we already host
            // `graph_search` natively, producing a `mcp__jfc__graph_search`
            // collision). Drop those: the agent gets a single, canonical
            // implementation and shadowing surprises don't reach the model.
            if builtin_names.contains(&tool.name) {
                tracing::warn!(
                    target: "jfc::tools::mcp",
                    tool = %tool.name,
                    "dropping MCP-advertised tool that collides with a builtin name"
                );
                continue;
            }
            tools.push(tool);
        }
    }
    tools
}

pub(super) async fn execute_tool_search(
    query: &str,
    limit: Option<u64>,
    cwd: &Path,
) -> ExecutionResult {
    let query = query.trim().to_ascii_lowercase();
    let limit = limit.unwrap_or(20).clamp(1, 50) as usize;
    let mut rows: Vec<(usize, String)> = Vec::new();

    for tool in all_tool_defs_with_mcp().await {
        let haystack = format!(
            "{} {} {}",
            tool.name,
            tool.description,
            tool.input_schema
                .get("properties")
                .map(|v| v.to_string())
                .unwrap_or_default()
        )
        .to_ascii_lowercase();
        let score = relevance_score(&haystack, &query);
        if score > 0 {
            rows.push((
                score,
                format!(
                    "- tool `{}`: {}\n  schema: {}",
                    tool.name,
                    tool.description,
                    compact_schema(&tool.input_schema)
                ),
            ));
        }
    }

    for skill in crate::agents::load_skills(cwd) {
        let haystack = format!(
            "{} {} {}",
            skill.name,
            skill.description.clone().unwrap_or_default(),
            skill.body.lines().take(6).collect::<Vec<_>>().join(" ")
        )
        .to_ascii_lowercase();
        let score = relevance_score(&haystack, &query);
        if score > 0 {
            rows.push((
                score.saturating_add(1),
                format!(
                    "- skill `{}`: {}\n  invoke: Skill {{ \"name\": \"{}\" }}",
                    skill.name,
                    skill.description.as_deref().unwrap_or("no description"),
                    skill.name
                ),
            ));
        }
    }

    rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let body = rows
        .into_iter()
        .take(limit)
        .map(|(_, row)| row)
        .collect::<Vec<_>>()
        .join("\n");
    if body.is_empty() {
        ExecutionResult::success(format!("No tools or skills matched query `{query}`."))
    } else {
        ExecutionResult::success(format!("Matches for `{query}`:\n{body}"))
    }
}

pub(super) async fn execute_tool_suggest(
    intent: &str,
    limit: Option<u64>,
    cwd: &Path,
) -> ExecutionResult {
    execute_tool_search(intent, Some(limit.unwrap_or(8).clamp(1, 20)), cwd).await
}

fn relevance_score(haystack: &str, query: &str) -> usize {
    if query.is_empty() {
        return 1;
    }
    let mut score = 0usize;
    if haystack.contains(query) {
        score += 8;
    }
    for term in query.split_whitespace().filter(|s| !s.is_empty()) {
        if haystack.contains(term) {
            score += 2;
        }
    }
    score
}

fn compact_schema(schema: &serde_json::Value) -> String {
    let required = schema
        .get("required")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "none".to_owned());
    let props = schema
        .get("properties")
        .and_then(|v| v.as_object())
        .map(|obj| obj.keys().cloned().collect::<Vec<_>>().join(", "))
        .unwrap_or_else(|| "none".to_owned());
    format!("required [{required}], properties [{props}]")
}

// ---------------------------------------------------------------------------
// Shell / process helpers
// ---------------------------------------------------------------------------

pub(super) fn configure_tool_command(command: &mut Command) {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("SUDO_ASKPASS", "/bin/false")
        .env("SSH_ASKPASS", "/bin/false");

    #[cfg(unix)]
    unsafe {
        command.pre_exec(|| {
            if setsid() == -1 {
                Err(std::io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }
}

pub(super) fn terminal_safe_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\u{1b}' => match chars.peek().copied() {
                Some('[') => {
                    chars.next();
                    for c in chars.by_ref() {
                        if ('@'..='~').contains(&c) {
                            break;
                        }
                    }
                }
                Some(']') => {
                    chars.next();
                    let mut previous_was_esc = false;
                    for c in chars.by_ref() {
                        if c == '\u{7}' || (previous_was_esc && c == '\\') {
                            break;
                        }
                        previous_was_esc = c == '\u{1b}';
                    }
                }
                Some(_) => {
                    chars.next();
                }
                None => {}
            },
            '\t' | '\n' | '\r' => out.push(ch),
            c if c.is_control() => {}
            c => out.push(c),
        }
    }

    out
}

pub(super) fn non_interactive_shell_command(command: &str) -> String {
    let trimmed = command.trim_start();
    let leading_len = command.len() - trimmed.len();

    if trimmed == "sudo" {
        return format!("{}sudo -n", &command[..leading_len]);
    }

    let Some(rest) = trimmed.strip_prefix("sudo ") else {
        return command.to_string();
    };

    if rest.starts_with("-n ") || rest == "-n" || rest.starts_with("--non-interactive ") {
        command.to_string()
    } else {
        format!("{}sudo -n {}", &command[..leading_len], rest)
    }
}

// ---------------------------------------------------------------------------
// Permission helper (feature-gated)
// ---------------------------------------------------------------------------

#[cfg(feature = "permission-automation")]
pub(crate) fn tool_permission_path(input: &crate::types::ToolInput) -> Option<&str> {
    use crate::types::ToolInput;
    match input {
        ToolInput::Edit { file_path, .. }
        | ToolInput::Write { file_path, .. }
        | ToolInput::Read { file_path, .. } => Some(file_path.as_str()),
        ToolInput::Bash {
            workdir: Some(workdir),
            ..
        }
        | ToolInput::Glob {
            path: Some(workdir),
            ..
        }
        | ToolInput::Grep {
            path: Some(workdir),
            ..
        }
        | ToolInput::Search {
            path: Some(workdir),
            ..
        } => Some(workdir.as_str()),
        ToolInput::MemoryDelete { path } => Some(path.as_str()),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Slop guard
// ---------------------------------------------------------------------------

/// The sentinel marker appended to tool outputs when slop_guard finds issues.
/// Used by the event loop to detect and aggregate findings across a batch.
pub(crate) const SLOP_GUARD_MARKER: &str = "\n\n--- Slop Guard ---\n";

/// Run the slop_guard checks on a file that was just written/edited.
/// Returns the original result with findings appended on success,
/// or the original result unchanged if slop_guard panics, times out
/// (>2s), or finds nothing.
pub(super) async fn maybe_run_slop_guard(
    mut result: ExecutionResult,
    file_path: &Path,
    file_content: &str,
    cwd: &Path,
) -> ExecutionResult {
    use std::time::Duration;

    // Non-blocking: if slop_guard panics or exceeds 2s, skip silently.
    let path = file_path.to_path_buf();
    let content = file_content.to_string();
    let workspace = cwd.to_path_buf();

    let handle = tokio::spawn(async move {
        crate::slop_guard::run_all_checks(&path, &content, &workspace).await
    });

    let guard_result = tokio::time::timeout(Duration::from_secs(2), handle).await;

    match guard_result {
        Ok(Ok(report)) => {
            tracing::debug!(
                target: "jfc::slop_guard",
                file = %file_path.display(),
                has_findings = report.has_findings,
                "slop_guard completed"
            );
            if report.has_findings {
                let formatted = crate::slop_guard::format_report(&report);
                tracing::debug!(
                    target: "jfc::slop_guard",
                    file = %file_path.display(),
                    findings = %formatted,
                    "slop_guard findings"
                );
                result.output.push_str(SLOP_GUARD_MARKER);
                result.output.push_str(&formatted);
            }
        }
        Ok(Err(_join_err)) => {
            tracing::debug!(
                target: "jfc::slop_guard",
                file = %file_path.display(),
                "slop_guard panicked, skipping"
            );
        }
        Err(_timeout) => {
            tracing::debug!(
                target: "jfc::slop_guard",
                file = %file_path.display(),
                "slop_guard timed out (>2s), skipping"
            );
        }
    }

    result
}
