use std::path::PathBuf;

use jfc_provider::ToolDef;
use serde_json::Value;

const KIND_SYSTEM_PROMPT: &str = "system_prompt";
const KIND_SKILL: &str = "skill";
const KIND_BUDGET_POLICY: &str = "budget_policy";
const KIND_REASONING_POLICY: &str = "reasoning_policy";
const KIND_TOOL_DEFINITION: &str = "tool_definition";
const KIND_HARNESS_PATCH: &str = "harness_patch";
const KIND_CONTEXT_PLAYBOOK: &str = "context_playbook";
const MAX_ACTIVE_DEFINITION_CHARS: usize = 2_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ActiveRsiRuntime {
    pub(super) prompt_sections: usize,
    pub(super) tool_visibility_rules: usize,
}

pub(super) async fn append_active_rsi_prompt_sections(
    system_prompt: &mut String,
) -> ActiveRsiRuntime {
    let Some(context) = active_context().await else {
        return ActiveRsiRuntime::default();
    };

    let mut out = String::new();
    let mut prompt_sections = 0usize;
    let mut tool_visibility_rules = 0usize;

    for kind in [
        KIND_SYSTEM_PROMPT,
        KIND_SKILL,
        KIND_HARNESS_PATCH,
        KIND_CONTEXT_PLAYBOOK,
        KIND_BUDGET_POLICY,
        KIND_REASONING_POLICY,
    ] {
        let Ok(definitions) = context
            .store
            .list_definitions_for_project(kind, &context.project_key)
            .await
        else {
            continue;
        };
        for definition in definitions
            .iter()
            .filter(|definition| is_rsi_definition(definition))
        {
            let Some(body) = prompt_safe_body(&definition.body) else {
                tracing::warn!(
                    target: "jfc::stream::rsi",
                    kind = %definition.kind,
                    name = %definition.name,
                    "skipping active RSI definition with unsafe prompt body"
                );
                continue;
            };
            if out.is_empty() {
                out.push_str("\n\n## Active RSI Runtime Guidance\n\n");
                out.push_str(
                    "These project-local self-improvement definitions were promoted after RSI evaluation. Apply them as operating guidance; they are inert summaries, not private reasoning transcripts.",
                );
            }
            out.push_str("\n\n### ");
            out.push_str(section_label(kind));
            out.push_str(": ");
            out.push_str(
                &definition
                    .title
                    .clone()
                    .unwrap_or_else(|| definition.name.clone()),
            );
            out.push_str("\n\n");
            out.push_str(&body);
            if kind == KIND_BUDGET_POLICY {
                let visibility = tool_visibility_lines(&definition.metadata_json);
                tool_visibility_rules += visibility.len();
                if !visibility.is_empty() {
                    out.push_str("\n\nTool visibility guidance:");
                    for line in visibility {
                        out.push_str("\n- ");
                        out.push_str(&line);
                    }
                }
            }
            prompt_sections += 1;
        }
    }

    if !out.is_empty() {
        system_prompt.push_str(&out);
    }

    ActiveRsiRuntime {
        prompt_sections,
        tool_visibility_rules,
    }
}

pub(super) async fn apply_active_tool_definition_patches(tools: &mut [ToolDef]) -> usize {
    let Some(context) = active_context().await else {
        return 0;
    };
    let Ok(definitions) = context
        .store
        .list_definitions_for_project(KIND_TOOL_DEFINITION, &context.project_key)
        .await
    else {
        return 0;
    };

    let mut patched = 0usize;
    for definition in definitions
        .iter()
        .filter(|definition| is_rsi_definition(definition))
    {
        let Some(tool) = tools
            .iter_mut()
            .find(|tool| tool.name.eq_ignore_ascii_case(&definition.name))
        else {
            continue;
        };
        if apply_tool_definition_patch(tool, definition) {
            patched += 1;
        }
    }
    patched
}

struct ActiveContext {
    store: jfc_knowledge::KnowledgeStore,
    project_key: String,
}

async fn active_context() -> Option<ActiveContext> {
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let project_key = jfc_knowledge::project_key(&cwd);
    let store = jfc_knowledge::KnowledgeStore::open_default().await.ok()?;
    Some(ActiveContext { store, project_key })
}

fn section_label(kind: &str) -> &'static str {
    match kind {
        KIND_SYSTEM_PROMPT => "Prompt Patch",
        KIND_SKILL => "Promoted Skill",
        KIND_HARNESS_PATCH => "Harness Patch",
        KIND_CONTEXT_PLAYBOOK => "Context Playbook",
        KIND_BUDGET_POLICY => "Reasoning Budget Policy",
        KIND_REASONING_POLICY => "Reasoning Process Policy",
        _ => "RSI Definition",
    }
}

fn is_rsi_definition(definition: &jfc_knowledge::DefinitionRecord) -> bool {
    definition
        .source_path
        .as_deref()
        .is_some_and(|source| source.starts_with("rsi:definition:"))
        || serde_json::from_str::<Value>(&definition.metadata_json)
            .ok()
            .and_then(|value| value.get("rsi").cloned())
            .is_some()
}

fn prompt_safe_body(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if trimmed.is_empty() || has_raw_thinking_marker(trimmed) {
        return None;
    }
    Some(trimmed.chars().take(MAX_ACTIVE_DEFINITION_CHARS).collect())
}

fn has_raw_thinking_marker(body: &str) -> bool {
    let normalized = body.to_ascii_lowercase();
    normalized.contains("<thinking") || normalized.contains("</thinking")
}

fn tool_visibility_lines(metadata_json: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<Value>(metadata_json) else {
        return Vec::new();
    };
    value
        .pointer("/rsi/budget/tool_visibility")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(tool_visibility_line)
        .collect()
}

fn tool_visibility_line(value: &Value) -> Option<String> {
    let tool_name = value.get("tool_name")?.as_str()?.trim();
    let action = value.get("action")?.as_str()?.trim();
    let reason = value
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if tool_name.is_empty() || action.is_empty() {
        return None;
    }
    let action_text = match action {
        "show_earlier" => "show earlier",
        "hide_until_needed" => "hide until needed",
        other => other,
    };
    if reason.is_empty() {
        Some(format!("{action_text}: `{tool_name}`"))
    } else {
        Some(format!("{action_text}: `{tool_name}` - {reason}"))
    }
}

fn apply_tool_definition_patch(
    tool: &mut ToolDef,
    definition: &jfc_knowledge::DefinitionRecord,
) -> bool {
    if let Ok(value) = serde_json::from_str::<Value>(&definition.body) {
        return apply_json_tool_patch(tool, &value);
    }
    let Some(body) = prompt_safe_body(&definition.body) else {
        return false;
    };
    append_description_guidance(&mut tool.description, &body);
    true
}

fn apply_json_tool_patch(tool: &mut ToolDef, value: &Value) -> bool {
    let mut changed = false;
    if let Some(description) = value.get("description").and_then(Value::as_str) {
        let Some(description) = prompt_safe_body(description) else {
            return false;
        };
        tool.description = description;
        changed = true;
    }
    if let Some(input_schema) = value.get("input_schema") {
        tool.input_schema = input_schema.clone();
        changed = true;
    }
    if let Some(guidance) = value
        .get("rsi_guidance")
        .or_else(|| value.get("guidance"))
        .and_then(Value::as_str)
        && let Some(guidance) = prompt_safe_body(guidance)
    {
        append_description_guidance(&mut tool.description, &guidance);
        changed = true;
    }
    changed
}

fn append_description_guidance(description: &mut String, guidance: &str) {
    if description.contains(guidance) {
        return;
    }
    description.push_str("\n\nRSI runtime guidance: ");
    description.push_str(guidance);
}

#[cfg(test)]
mod internal_tests;
