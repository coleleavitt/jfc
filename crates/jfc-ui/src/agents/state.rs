//! Agent and skill data types, serde frontmatter structs, and low-level
//! parsing helpers (`parse_skill`, `parse_agent`, `split_frontmatter`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use jfc_core::{AgentCost, AgentDef, Effort, MemoryScope, PermissionMode};

/// A loaded skill: frontmatter metadata + markdown body. The body becomes a
/// `<skill_content>` system message when the skill is invoked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub source: PathBuf,
    pub description: Option<String>,
    pub body: String,
}

/// Parse a skill .md file: optional YAML frontmatter (between `---` lines)
/// followed by a markdown body. Frontmatter fields: `name`, `description`.
/// If `name` is missing, falls back to the filename stem.
pub(super) fn parse_skill(path: &Path, raw: &str) -> Option<Skill> {
    let (front, body) = split_frontmatter(raw);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed");
    let mut name = stem.to_owned();
    let mut description = None;
    if let Some(yaml) = front {
        if let Ok(parsed) = serde_yaml::from_str::<SkillFront>(yaml) {
            if let Some(n) = parsed.name {
                name = n;
            }
            description = parsed.description;
        }
    }
    Some(Skill {
        name,
        source: path.to_path_buf(),
        description,
        body: body.trim().to_owned(),
    })
}

pub(super) fn parse_agent(path: &Path, raw: &str) -> Option<AgentDef> {
    let (front, body) = split_frontmatter(raw);
    let yaml = front?;
    let parsed: AgentFront = serde_yaml::from_str(yaml).ok()?;
    Some(AgentDef {
        name: parsed.name,
        source: path.to_path_buf(),
        model: parsed.model,
        isolation: parsed.isolation,
        skills: parsed.skills.unwrap_or_default(),
        allowed_tools: parsed.allowed_tools.unwrap_or_default(),
        disallowed_tools: parsed.disallowed_tools.unwrap_or_default(),
        permission_mode: parsed.permission_mode,
        forks_parent_context: parsed.forks_parent_context,
        background: parsed.background,
        color: parsed.color,
        effort: parsed.effort,
        max_turns: parsed.max_turns,
        max_input_tokens: parsed.max_input_tokens,
        memory: parsed.memory,
        mcp_servers: parsed.mcp_servers.unwrap_or_default(),
        hooks: parsed.hooks.unwrap_or_default(),
        // Auto-dispatch metadata is YAML-parsed via the same AgentFront
        // path; defaults to None / empty so existing user-defined
        // agents in `.claude/agents/` keep working without churn.
        key_trigger: parsed.key_trigger,
        use_when: parsed.use_when.unwrap_or_default(),
        avoid_when: parsed.avoid_when.unwrap_or_default(),
        cost: parsed.cost,
        system_prompt: body.trim().to_owned(),
    })
}

pub(super) fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
    if !raw.starts_with("---") {
        return (None, raw);
    }
    let after_open = &raw[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);
    let Some(close) = after_open.find("\n---") else {
        return (None, raw);
    };
    let yaml = &after_open[..close];
    let rest = &after_open[close + 4..];
    let rest = rest.strip_prefix('\n').unwrap_or(rest);
    (Some(yaml), rest)
}

#[derive(Debug, Deserialize)]
pub(super) struct SkillFront {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(super) struct AgentFront {
    pub name: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub skills: Option<Vec<String>>,
    #[serde(default, rename = "allowedTools")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, rename = "disallowedTools")]
    pub disallowed_tools: Option<Vec<String>>,
    #[serde(default, rename = "permissionMode")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default, rename = "forksParentContext")]
    pub forks_parent_context: Option<serde_json::Value>,
    #[serde(default)]
    pub background: Option<bool>,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub effort: Option<Effort>,
    #[serde(default, rename = "maxTurns")]
    pub max_turns: Option<u32>,
    #[serde(default, rename = "maxInputTokens")]
    pub max_input_tokens: Option<u64>,
    #[serde(default)]
    pub memory: Option<MemoryScope>,
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    pub hooks: Option<std::collections::HashMap<String, Vec<String>>>,
    /// Auto-dispatch metadata — see `AgentDef` field docs.
    #[serde(default, rename = "keyTrigger")]
    pub key_trigger: Option<String>,
    #[serde(default, rename = "useWhen")]
    pub use_when: Option<Vec<String>>,
    #[serde(default, rename = "avoidWhen")]
    pub avoid_when: Option<Vec<String>>,
    #[serde(default)]
    pub cost: Option<AgentCost>,
}
