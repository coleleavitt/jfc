use std::collections::HashSet;

use jfc_provider::{ProviderContent, ProviderMessage, ToolDef};

mod intent;
#[cfg(test)]
mod tests;

use super::code_navigation::{is_code_navigation_tool_name, prioritize_code_navigation_tools};
use intent::intent_tool_matches;

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
    "TeamCreate",
    "SendMessage",
    "TeamMemberMode",
    "Advisor",
    "Research",
    "Council",
    "AskModel",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "SendUserMessage",
];

const DEFAULT_MAX_INTENT_MATCHES: usize = 12;
const DEFAULT_MAX_DISCOVERED_MATCHES: usize = 24;

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
        if super::defs::is_model_hidden_builtin_tool_name(&name) {
            continue;
        }
        selected.insert(normalize_tool_name(&name));
    }

    let max_discovered_matches = discovered_match_limit();
    let mut discovered_added = 0usize;
    for name in discovered_tool_names(messages) {
        if discovered_added >= max_discovered_matches {
            break;
        }
        if super::defs::is_model_hidden_builtin_tool_name(&name) {
            continue;
        }
        if selected.insert(normalize_tool_name(&name)) {
            discovered_added += 1;
        }
    }

    if let Some(intent) = user_intent {
        for name in intent_tool_matches(intent, &all, intent_match_limit()) {
            selected.insert(normalize_tool_name(&name));
        }
    }

    let mut tools = all
        .into_iter()
        .filter(|tool| selected.contains(&normalize_tool_name(&tool.name)))
        .collect::<Vec<_>>();
    prioritize_code_navigation_tools(&mut tools);
    tools
}

fn intent_match_limit() -> usize {
    env_usize("JFC_MAX_INTENT_TOOL_MATCHES", DEFAULT_MAX_INTENT_MATCHES)
}

fn discovered_match_limit() -> usize {
    env_usize(
        "JFC_MAX_DISCOVERED_TOOL_MATCHES",
        DEFAULT_MAX_DISCOVERED_MATCHES,
    )
}

fn env_usize(name: &str, default: usize) -> usize {
    std::env::var(name)
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(default)
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

fn dedup_preserve_order(values: Vec<String>) -> Vec<String> {
    let mut seen = HashSet::new();
    values
        .into_iter()
        .filter(|value| seen.insert(normalize_tool_name(value)))
        .collect()
}
