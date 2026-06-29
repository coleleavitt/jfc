use jfc_provider::ToolDef;

pub(crate) fn is_code_navigation_tool_name(name: &str) -> bool {
    let normalized = normalize_tool_name(name);
    const DIRECT_NAMES: &[&str] = &[
        "codegraph_search",
        "codegraph_explore",
        "codegraph_arch",
        "codegraph_node",
        "codegraph_callers",
        "codegraph_callees",
        "codegraph_impact",
        "codegraph_files",
        "codegraph_paths",
        "codegraph_xref",
        "codegraph_vuln",
        "codegraph_verify_roles",
        "codegraph_status",
    ];

    DIRECT_NAMES.contains(&normalized.as_str())
        || normalized.starts_with("mcp__codegraph__")
        || normalized.contains("__codegraph_")
}

pub(crate) fn prioritize_code_navigation_tools(tools: &mut [ToolDef]) {
    tools.sort_by_key(|tool| !is_code_navigation_tool_name(&tool.name));
}

pub(crate) fn enrich_code_navigation_tool_def(tool: &mut ToolDef) {
    let Some(detail) = code_navigation_tool_detail(&tool.name) else {
        return;
    };
    if tool.description.contains("Selection guidance:") {
        return;
    }
    let description = tool.description.trim();
    tool.description = if description.is_empty() {
        format!("Selection guidance: {detail}")
    } else {
        format!("{description} Selection guidance: {detail}")
    };
}

fn code_navigation_tool_detail(name: &str) -> Option<&'static str> {
    if !is_code_navigation_tool_name(name) {
        return None;
    }
    let raw = code_navigation_raw_name(name).to_ascii_lowercase();
    match raw.as_str() {
        "codegraph_explore" => Some(
            "CodeGraph indexed source exploration. Returns related symbols, source snippets, file references, callers/callees, and dependency context. Use first for architecture questions, how-does-this-work prompts, bug tracing, and multi-symbol mapping. Input: natural-language query naming symbols, files, or concepts. Prefer over Read/Grep for source structure. Avoid for literal log text, config keys, generated files, or non-code content.",
        ),
        "codegraph_search" => Some(
            "CodeGraph indexed source symbol search. Returns matching definitions and locations for functions, types, methods, routes, and components. Use first when you know or can guess an identifier. Input: symbol or partial symbol name. Prefer over Grep for identifiers because the graph resolves definitions. Avoid for arbitrary prose, logs, or non-indexed files.",
        ),
        "codegraph_node" => Some(
            "CodeGraph indexed source node lookup. Returns one symbol's location, signature, call trail, and optionally full body. Use after codegraph_search or codegraph_explore when you need the exact implementation. Input: exact symbol name, with optional file or line to disambiguate. Prefer over reading a whole file for one symbol. Avoid when you need several related symbols at once; use codegraph_explore.",
        ),
        "codegraph_arch" => Some(
            "CodeGraph indexed source architecture map. Returns modules, key definitions, and dependencies in and out of a subsystem. Use first for area surveys, ownership boundaries, and module structure. Input: subsystem path or omit for the whole project. Prefer over broad ls/Read surveys. Avoid for literal text searches or single-symbol bodies.",
        ),
        "codegraph_callers" => Some(
            "CodeGraph indexed caller lookup. Returns functions or methods that call a symbol. Use for impact analysis, bug provenance, and API usage discovery. Input: exact symbol name. Prefer over Grep for call relationships. Avoid for strings that are not symbols.",
        ),
        "codegraph_callees" => Some(
            "CodeGraph indexed callee lookup. Returns functions or methods called by a symbol. Use to understand one function's execution path and dependencies. Input: exact symbol name. Prefer over manually reading through a function body first. Avoid for broad architecture surveys; use codegraph_explore or codegraph_arch.",
        ),
        "codegraph_impact" => Some(
            "CodeGraph indexed impact analysis. Returns symbols likely affected by changing a symbol. Use before refactors, API changes, or behavior edits. Input: exact symbol name plus optional depth. Prefer over manual caller-tree reconstruction. Avoid for non-code text and unindexed generated files.",
        ),
        "codegraph_files" => Some(
            "CodeGraph indexed file tree and symbol map. Returns indexed files with language and symbol counts. Use to locate relevant source areas or inspect a directory's shape. Input: optional path, glob pattern, depth, or format. Prefer over broad ls/find/Read surveys. Avoid for file contents you are about to edit; use Read after narrowing.",
        ),
        "codegraph_paths" => Some(
            "CodeGraph indexed reachability path lookup. Returns call/reference chains from a source symbol to a sink symbol. Use to prove whether one code path can reach another. Input: exact source and sink symbols. Prefer over manual call-graph tracing. Avoid for vague area surveys; use codegraph_explore first.",
        ),
        "codegraph_xref" => Some(
            "CodeGraph indexed cross-reference lookup. Returns incoming references grouped by kind, including callers, reads, writes, type refs, and impls. Use when a change may affect reads/writes beyond direct calls. Input: exact symbol name. Prefer over Grep for symbol references. Avoid for literal strings and non-code files.",
        ),
        "codegraph_vuln" => Some(
            "CodeGraph indexed vulnerability scan. Returns graph-corroborated missing-guard, taint, and concurrency findings with confidence. Use for security-oriented code review or suspicious authorization/data-flow areas. Input: optional confidence threshold. Prefer over ad hoc Grep when looking for systemic bug classes. Avoid treating findings as final without source review.",
        ),
        "codegraph_verify_roles" => Some(
            "CodeGraph indexed role verification. Returns only missing-guard findings corroborated by proposed sources, sinks, guards, and sanitizers. Use when you can name candidate roles and need graph evidence. Input: proposed symbol roles. Prefer over unsupported security speculation. Avoid if you have not identified candidate symbols yet; explore first.",
        ),
        "codegraph_status" => Some(
            "CodeGraph index health check. Returns index file, node, and edge counts. Use when CodeGraph results look missing or stale. Input: optional project path. Prefer before falling back to broad Read/Grep because a bad index explains bad graph coverage. Avoid using it as the first code-understanding tool when explore/search are healthy.",
        ),
        raw if raw.starts_with("codegraph_") => Some(
            "CodeGraph indexed source helper. Use for source graph analysis when this visible tool's schema matches the task. Input: follow the tool schema exactly. Prefer over Read/Grep for indexed source structure. Avoid for literal logs, config keys, generated files, or non-code text.",
        ),
        _ => None,
    }
}

fn code_navigation_raw_name(name: &str) -> &str {
    let trimmed = name.trim();
    trimmed.rsplit("__").next().unwrap_or(trimmed)
}

fn normalize_tool_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, description: &str) -> ToolDef {
        ToolDef {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema: serde_json::json!({ "type": "object" }),
        }
    }

    #[test]
    fn enrich_code_navigation_tool_def_adds_selection_boundaries_regression() {
        let mut tool = tool("mcp__codegraph__codegraph_explore", "Explore code.");

        enrich_code_navigation_tool_def(&mut tool);

        assert!(tool.description.contains("CodeGraph indexed source"));
        assert!(tool.description.contains("Use first"));
        assert!(tool.description.contains("Prefer over Read/Grep"));
        assert!(tool.description.contains("Avoid for literal log text"));
        assert!(tool.description.contains("Input: natural-language query"));
    }

    #[test]
    fn enrich_code_navigation_tool_def_leaves_non_codegraph_tools_alone_normal() {
        let mut tool = tool("mcp__filesystem__read_file", "Read a file.");

        enrich_code_navigation_tool_def(&mut tool);

        assert_eq!(tool.description, "Read a file.");
    }
}
