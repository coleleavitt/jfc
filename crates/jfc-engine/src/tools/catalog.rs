use std::collections::HashSet;

use jfc_provider::{ProviderContent, ProviderMessage, ToolDef};

const STARTER_TOOL_NAMES: &[&str] = &[
    "Bash",
    "Read",
    "Write",
    "Edit",
    "MultiEdit",
    "Glob",
    "Grep",
    "TaskCreate",
    "TaskUpdate",
    "TaskList",
    "TaskDone",
    "TaskStop",
    "TaskGet",
    "TaskValidate",
    "Skill",
    "ToolSearch",
    "ToolSuggest",
    "Task",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "SendUserMessage",
];

const MAX_INTENT_MATCHES: usize = 12;
const MAX_DISCOVERED_MATCHES: usize = 24;

pub fn progressive_tool_defs(
    all: Vec<ToolDef>,
    messages: &[ProviderMessage],
    user_intent: Option<&str>,
) -> Vec<ToolDef> {
    let mut selected: HashSet<String> = STARTER_TOOL_NAMES
        .iter()
        .map(|name| normalize_tool_name(name))
        .collect();

    // CodeGraph is an MCP-provided code navigation surface in most JFC
    // installs. Keep it model-visible on the first action turn so coding
    // tasks start with symbol/impact context instead of broad Read/Grep sweeps.
    for tool in &all {
        if is_code_navigation_tool_name(&tool.name) {
            selected.insert(normalize_tool_name(&tool.name));
        }
    }

    // Keep every tool name already present in replayed assistant history.
    // Anthropic sees historical `tool_use` blocks on every continuation; if a
    // previously-used tool is omitted from the current `tools` catalog, the
    // request can degrade into an immediate empty refusal instead of a normal
    // continuation.
    for name in historical_tool_use_names(messages) {
        selected.insert(normalize_tool_name(&name));
    }

    let mut discovered_added = 0usize;
    for name in discovered_tool_names(messages) {
        if discovered_added >= MAX_DISCOVERED_MATCHES {
            break;
        }
        if selected.insert(normalize_tool_name(&name)) {
            discovered_added += 1;
        }
    }

    if let Some(intent) = user_intent {
        for name in intent_tool_matches(intent, &all, MAX_INTENT_MATCHES) {
            selected.insert(normalize_tool_name(&name));
        }
    }

    all.into_iter()
        .filter(|tool| selected.contains(&normalize_tool_name(&tool.name)))
        .collect()
}

fn discovered_tool_names(messages: &[ProviderMessage]) -> Vec<String> {
    let tool_search_ids: HashSet<&str> = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|content| match content {
            ProviderContent::ToolUse { id, name, .. } if is_tool_discovery_call(name) => {
                Some(id.as_str())
            }
            _ => None,
        })
        .collect();

    if tool_search_ids.is_empty() {
        return Vec::new();
    }

    let mut names = Vec::new();
    for content in messages.iter().flat_map(|message| message.content.iter()) {
        let ProviderContent::ToolResult {
            tool_use_id,
            content,
            is_error,
        } = content
        else {
            continue;
        };
        if *is_error || !tool_search_ids.contains(tool_use_id.as_str()) {
            continue;
        }
        extract_tool_names_from_search_result(content, &mut names);
    }
    dedup_preserve_order(names)
}

fn historical_tool_use_names(messages: &[ProviderMessage]) -> Vec<String> {
    let mut names = Vec::new();
    for content in messages.iter().flat_map(|message| message.content.iter()) {
        if let ProviderContent::ToolUse { name, .. } = content
            && !is_tool_discovery_call(name)
        {
            names.push(name.clone());
        }
    }
    dedup_preserve_order(names)
}

fn extract_tool_names_from_search_result(content: &str, names: &mut Vec<String>) {
    for line in content.lines() {
        let Some((_, tail)) = line.split_once("tool `") else {
            continue;
        };
        let Some((name, _)) = tail.split_once('`') else {
            continue;
        };
        if !name.trim().is_empty() {
            names.push(name.trim().to_owned());
        }
    }
}

/// Match tools to a user's intent for progressive disclosure.
///
/// Primary path: jfc-core's [`jfc_core::ToolIndex`] — a TF-IDF/cosine retriever
/// over each tool's `name + description + schema`. This is the
/// *Improving Tool Retrieval* / ToolRet recipe (the triple-convergence gap
/// finding) made load-bearing: it ranks by term-weighted relevance rather than
/// raw substring presence, so a query like "search the web for docs" surfaces
/// `WebSearch` even when no token is a literal name substring.
///
/// The earlier hand-rolled [`intent_score`] substring scorer is retained as a
/// deterministic fallback: if TF-IDF finds nothing (e.g. the intent shares no
/// vocabulary with any tool), we fall back so progressive disclosure never
/// silently advertises *fewer* tools than the substring heuristic would have.
fn intent_tool_matches(intent: &str, all: &[ToolDef], limit: usize) -> Vec<String> {
    let terms = intent_terms(intent);
    if terms.is_empty() {
        return Vec::new();
    }

    // Build the TF-IDF index over the full catalog, keyed by tool name.
    let docs: Vec<(String, String)> = all
        .iter()
        .map(|tool| {
            let schema = tool
                .input_schema
                .get("properties")
                .map(|value| value.to_string())
                .unwrap_or_default();
            (
                tool.name.clone(),
                format!("{} {} {}", tool.name, tool.description, schema),
            )
        })
        .collect();
    let index = jfc_core::ToolIndex::build(docs);

    // Query with the cleaned intent terms (stopwords already stripped).
    let query = terms.join(" ");
    let hits = index.search(&query, limit);
    if !hits.is_empty() {
        return hits.into_iter().map(|(name, _score)| name).collect();
    }

    // Fallback: the original substring scorer, so we never regress to fewer
    // matches than before when TF-IDF and the query share no vocabulary.
    let mut scored: Vec<(usize, String)> = all
        .iter()
        .filter_map(|tool| {
            let score = intent_score(tool, &terms);
            (score >= 4).then(|| (score, tool.name.clone()))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    scored
        .into_iter()
        .take(limit)
        .map(|(_, name)| name)
        .collect()
}

fn intent_score(tool: &ToolDef, terms: &[String]) -> usize {
    let name = tool.name.to_ascii_lowercase();
    let description = tool.description.to_ascii_lowercase();
    let schema = tool
        .input_schema
        .get("properties")
        .map(|value| value.to_string().to_ascii_lowercase())
        .unwrap_or_default();

    let mut score = 0usize;
    for term in terms {
        if name == *term {
            score += 10;
        } else if name.contains(term) {
            score += 6;
        }
        if description.contains(term) {
            score += 3;
        }
        if schema.contains(term) {
            score += 1;
        }
    }
    score
}

fn intent_terms(intent: &str) -> Vec<String> {
    let mut terms = Vec::new();
    for raw in intent
        .to_ascii_lowercase()
        .split(|c: char| !c.is_ascii_alphanumeric() && c != '_')
    {
        let term = raw.trim();
        if term.len() < 3 || INTENT_STOPWORDS.contains(&term) {
            continue;
        }
        terms.push(term.to_owned());
    }
    dedup_preserve_order(terms)
}

const INTENT_STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "that", "this", "from", "into", "onto", "all", "any", "can",
    "could", "would", "should", "please", "thank", "you", "use", "using", "tool", "tools", "task",
    "tasks", "make", "made", "work", "works", "working", "need", "needs", "want", "wants", "about",
    "what", "when", "where", "why", "how", "fix", "add", "do", "run", "get", "set", "list", "show",
    "tell", "find", "read", "write", "edit", "update", "create", "delete", "remove",
];

fn is_tool_discovery_call(name: &str) -> bool {
    name.eq_ignore_ascii_case("ToolSearch")
        || name.eq_ignore_ascii_case("tool_search")
        || name.eq_ignore_ascii_case("tool_search_tool")
        || name.eq_ignore_ascii_case("ToolSuggest")
        || name.eq_ignore_ascii_case("tool_suggest")
        || name.eq_ignore_ascii_case("tool_suggest_tool")
}

fn normalize_tool_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

pub(crate) fn is_code_navigation_tool_name(name: &str) -> bool {
    let normalized = normalize_tool_name(name);
    const DIRECT_NAMES: &[&str] = &[
        "codegraph_search",
        "codegraph_explore",
        "codegraph_node",
        "codegraph_callers",
        "codegraph_callees",
        "codegraph_impact",
        "codegraph_files",
        "codegraph_status",
    ];

    DIRECT_NAMES.contains(&normalized.as_str())
        || normalized.starts_with("mcp__codegraph__")
        || normalized.contains("__codegraph_")
}

fn dedup_preserve_order(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(normalize_tool_name(value)))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_provider::{ProviderRole, ToolDef};

    fn tool(name: &str, description: &str) -> ToolDef {
        ToolDef {
            name: name.to_owned(),
            description: description.to_owned(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "query": { "type": "string" }
                }
            }),
        }
    }

    fn assistant_tool_use(id: &str, name: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::Assistant,
            content: vec![ProviderContent::ToolUse {
                id: id.to_owned(),
                name: name.to_owned(),
                input: serde_json::json!({}),
                thought_signature: None,
            }],
        }
    }

    fn user_tool_result(id: &str, body: &str) -> ProviderMessage {
        ProviderMessage {
            role: ProviderRole::User,
            content: vec![ProviderContent::ToolResult {
                tool_use_id: id.to_owned(),
                content: body.to_owned(),
                is_error: false,
            }],
        }
    }

    #[test]
    fn progressive_catalog_keeps_core_tools_normal() {
        let all = vec![
            tool("Read", "read files"),
            tool("ToolSearch", "search tools"),
            tool(
                "mcp__codegraph__codegraph_explore",
                "Explore code graph context",
            ),
            tool("run_coverage", "coverage reports"),
        ];

        let selected = progressive_tool_defs(all, &[], Some("hello"));
        let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

        assert!(names.contains(&"Read"));
        assert!(names.contains(&"ToolSearch"));
        assert!(names.contains(&"mcp__codegraph__codegraph_explore"));
        assert!(!names.contains(&"run_coverage"));
    }

    #[test]
    fn progressive_catalog_keeps_codegraph_tools_visible_regression() {
        let all = vec![
            tool("Read", "read files"),
            tool("mcp__codegraph__codegraph_search", "Search indexed symbols"),
            tool(
                "mcp__codegraph__codegraph_explore",
                "Explore related symbols and code",
            ),
            tool("run_coverage", "coverage reports"),
        ];

        let selected = progressive_tool_defs(all, &[], Some("fix this bug"));
        let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

        assert!(names.contains(&"mcp__codegraph__codegraph_search"));
        assert!(names.contains(&"mcp__codegraph__codegraph_explore"));
        assert!(!names.contains(&"run_coverage"));
    }

    #[test]
    fn progressive_catalog_reveals_tool_search_results_normal() {
        let all = vec![
            tool("Read", "read files"),
            tool("ToolSearch", "search tools"),
            tool("run_coverage", "coverage reports"),
        ];
        let messages = vec![
            assistant_tool_use("toolu_1", "ToolSearch"),
            user_tool_result(
                "toolu_1",
                "Matches for `coverage`:\n- tool `run_coverage`: Run cargo llvm-cov",
            ),
        ];

        let selected = progressive_tool_defs(all, &messages, None);
        let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

        assert!(names.contains(&"run_coverage"));
    }

    #[test]
    fn progressive_catalog_keeps_all_historical_tool_names_regression() {
        let all = vec![
            tool("Read", "read files"),
            tool("ToolSearch", "search tools"),
            tool("WebSearch", "search the web for current information"),
            tool("run_coverage", "coverage reports"),
        ];
        let mut messages = Vec::new();
        messages.push(assistant_tool_use("toolu_old", "WebSearch"));
        for idx in 0..12 {
            messages.push(ProviderMessage {
                role: ProviderRole::User,
                content: vec![ProviderContent::Text(format!("turn {idx}"))],
            });
        }
        messages.push(assistant_tool_use("toolu_recent", "Read"));

        let selected = progressive_tool_defs(all, &messages, Some("continue"));
        let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

        assert!(
            names.contains(&"WebSearch"),
            "historical tool_use names must stay advertised on replay"
        );
        assert!(!names.contains(&"run_coverage"));
    }

    #[test]
    fn historical_tool_count_does_not_starve_discovered_tools_regression() {
        let mut all = vec![
            tool("Read", "read files"),
            tool("ToolSearch", "search tools"),
            tool("run_coverage", "coverage reports"),
        ];
        let mut messages = vec![
            assistant_tool_use("toolu_search", "ToolSearch"),
            user_tool_result(
                "toolu_search",
                "Matches for `coverage`:\n- tool `run_coverage`: Run cargo llvm-cov",
            ),
        ];

        for idx in 0..32 {
            let name = format!("Historical{idx}");
            all.push(tool(&name, "already used in replayed history"));
            messages.push(assistant_tool_use(&format!("toolu_hist_{idx}"), &name));
        }

        let selected = progressive_tool_defs(all, &messages, Some("continue"));
        let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

        assert!(names.contains(&"run_coverage"));
    }

    #[test]
    fn progressive_catalog_selects_tools_from_intent_normal() {
        let all = vec![
            tool("Read", "read files"),
            tool("ToolSearch", "search tools"),
            tool("WebSearch", "search the web for current information"),
        ];

        let selected = progressive_tool_defs(all, &[], Some("search the web for docs"));
        let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

        assert!(names.contains(&"WebSearch"));
    }

    // The TF-IDF retriever surfaces a tool by *description* relevance even when
    // the intent shares no token with the tool's name — the substring scorer
    // (`name.contains`) would miss this. Proves the jfc-core ToolIndex is the
    // live ranker on the progressive-disclosure path.
    #[test]
    fn progressive_catalog_ranks_by_description_relevance_normal() {
        let all = vec![
            tool("Read", "read a file from disk"),
            tool("ToolSearch", "discover tools"),
            tool(
                "post_bounty",
                "register a coding bounty and let solver agents compete to win the reward",
            ),
            tool(
                "run_coverage",
                "annotate functions with test coverage hit counts",
            ),
        ];

        // "reward" / "compete" only appear in post_bounty's DESCRIPTION, never
        // in any tool name — the substring name scorer would not surface it.
        let selected = progressive_tool_defs(all, &[], Some("competition for a reward"));
        let names: Vec<&str> = selected.iter().map(|tool| tool.name.as_str()).collect();

        assert!(
            names.contains(&"post_bounty"),
            "TF-IDF should surface post_bounty by description relevance, got {names:?}"
        );
    }
}
