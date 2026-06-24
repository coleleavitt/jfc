use std::path::Path;

use jfc_provider::ToolDef;

use crate::runtime::ExecutionResult;

use super::code_navigation::enrich_code_navigation_tool_def;
use super::defs::model_tool_defs;
use super::registry::snapshot_mcp_registry;

pub async fn all_tool_defs_with_mcp() -> Vec<ToolDef> {
    let mut tools = model_tool_defs();
    let builtin_names: std::collections::HashSet<String> =
        tools.iter().map(|t| t.name.clone()).collect();
    if let Some(registry) = snapshot_mcp_registry() {
        for mut tool in registry.all_advertised_tool_defs().await {
            if builtin_names.contains(&tool.name) {
                tracing::warn!(
                    target: "jfc::tools::mcp",
                    tool = %tool.name,
                    "dropping MCP-advertised tool that collides with a builtin name"
                );
                continue;
            }
            enrich_code_navigation_tool_def(&mut tool);
            tools.push(tool);
        }
    }
    tools
}

pub async fn execute_tool_search(query: &str, limit: Option<u64>, cwd: &Path) -> ExecutionResult {
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
        if !skill.is_discoverable() || !skill.is_model_invocable() {
            continue;
        }
        let haystack = format!(
            "{} {} {} {}",
            skill.name,
            skill.description.clone().unwrap_or_default(),
            skill.context.as_str(),
            skill.body.lines().take(6).collect::<Vec<_>>().join(" ")
        )
        .to_ascii_lowercase();
        let score = relevance_score(&haystack, &query);
        if score > 0 {
            let mut details = Vec::new();
            if skill.context.is_fork() {
                details.push("fork".to_owned());
            }
            if !skill.files.is_empty() {
                details.push(format!("{} package files", skill.files.len()));
            }
            let detail_suffix = if details.is_empty() {
                String::new()
            } else {
                format!(" ({})", details.join(", "))
            };
            rows.push((
                score.saturating_add(1),
                format!(
                    "- skill `{}`{}: {}\n  invoke: Skill {{ \"name\": \"{}\" }}",
                    skill.name,
                    detail_suffix,
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

pub async fn execute_tool_suggest(intent: &str, limit: Option<u64>, cwd: &Path) -> ExecutionResult {
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
