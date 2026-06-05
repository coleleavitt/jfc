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
pub fn parse_skill(path: &Path, raw: &str) -> Option<Skill> {
    let (front, body) = split_frontmatter(raw);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed");
    let mut name = stem.to_owned();
    let mut description = None;
    if let Some(yaml) = front
        && let Ok(parsed) = serde_yaml::from_str::<SkillFront>(yaml)
    {
        if let Some(n) = parsed.name {
            name = n;
        }
        description = parsed.description;
    }
    Some(Skill {
        name,
        source: path.to_path_buf(),
        description,
        body: body.trim().to_owned(),
    })
}

pub fn parse_agent(path: &Path, raw: &str) -> Option<AgentDef> {
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
        key_trigger: parsed.key_trigger,
        use_when: parsed.use_when.unwrap_or_default(),
        avoid_when: parsed.avoid_when.unwrap_or_default(),
        cost: parsed.cost,
        system_prompt: body.trim().to_owned(),
    })
}

pub fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
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
struct SkillFront {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentFront {
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
    #[serde(default, rename = "keyTrigger")]
    pub key_trigger: Option<String>,
    #[serde(default, rename = "useWhen")]
    pub use_when: Option<Vec<String>>,
    #[serde(default, rename = "avoidWhen")]
    pub avoid_when: Option<Vec<String>>,
    #[serde(default)]
    pub cost: Option<AgentCost>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_skill_with_frontmatter_normal() {
        let raw = "---\nname: my-skill\ndescription: A test skill\n---\n# Body\n\nDo the thing.";
        let s = parse_skill(Path::new("/x/skills/my.md"), raw).expect("parsed");
        assert_eq!(s.name, "my-skill");
        assert_eq!(s.description.as_deref(), Some("A test skill"));
        assert!(s.body.contains("Do the thing"));
    }

    #[test]
    fn parse_skill_no_frontmatter_uses_filename_stem_normal() {
        let s = parse_skill(Path::new("/x/skills/snake.md"), "Just a body").expect("parsed");
        assert_eq!(s.name, "snake");
        assert_eq!(s.description, None);
        assert_eq!(s.body, "Just a body");
    }

    #[test]
    fn parse_agent_full_frontmatter_normal() {
        let raw = "---\nname: impl\nmodel: opus\nisolation: worktree\nskills:\n  - rust-style\nallowedTools:\n  - Read\n  - Edit\ndisallowedTools:\n  - Task\npermissionMode: acceptEdits\nbackground: true\ncolor: \"#ff0000\"\n---\n# Implementer\n\nYou implement features.";
        let a = parse_agent(Path::new("/x/agents/impl.md"), raw).expect("parsed");
        assert_eq!(a.name, "impl");
        assert_eq!(a.model.as_deref(), Some("opus"));
        assert_eq!(a.isolation.as_deref(), Some("worktree"));
        assert_eq!(a.skills, vec!["rust-style".to_owned()]);
        assert_eq!(a.allowed_tools, vec!["Read", "Edit"]);
        assert_eq!(a.disallowed_tools, vec!["Task"]);
        assert_eq!(a.permission_mode, Some(PermissionMode::AcceptEdits));
        assert_eq!(a.background, Some(true));
        assert_eq!(a.color.as_deref(), Some("#ff0000"));
        assert!(a.system_prompt.contains("You implement features"));
    }

    #[test]
    fn parse_agent_no_frontmatter_returns_none_robust() {
        let s = parse_agent(Path::new("/x/agents/x.md"), "Just a body");
        assert!(s.is_none());
    }

    #[test]
    fn parse_agent_malformed_yaml_returns_none_robust() {
        let raw = "---\nname: [missing close bracket\n---\nbody";
        assert!(parse_agent(Path::new("/x/a.md"), raw).is_none());
    }

    #[test]
    fn split_frontmatter_extracts_yaml_normal() {
        let raw = "---\nkey: value\n---\nbody";
        let (front, body) = split_frontmatter(raw);
        assert_eq!(front, Some("key: value"));
        assert_eq!(body, "body");
    }

    #[test]
    fn permission_mode_serde_roundtrip_normal() {
        for (mode, expected) in [
            (PermissionMode::Default, "default"),
            (PermissionMode::AcceptEdits, "acceptEdits"),
            (PermissionMode::BypassPermissions, "bypassPermissions"),
            (PermissionMode::Plan, "plan"),
            (PermissionMode::DontAsk, "dontAsk"),
            (PermissionMode::Auto, "auto"),
        ] {
            let s = serde_yaml::to_string(&mode).unwrap();
            assert!(s.trim().contains(expected), "{mode:?} → {s:?}");
            let parsed: PermissionMode = serde_yaml::from_str(&format!("---\n{expected}")).unwrap();
            assert_eq!(parsed, mode);
        }
    }

    #[test]
    fn parse_agent_full_v126_frontmatter_normal() {
        let raw = "---\n\
            name: deep-thinker\n\
            model: claude-opus-4-7\n\
            effort: high\n\
            maxTurns: 25\n\
            memory: project\n\
            mcpServers:\n  - github\n  - search\n\
            hooks:\n  pre-edit:\n    - ./scripts/lint.sh\n  post-test:\n    - echo done\n\
            ---\nYou are a deep thinker.";
        let agent = parse_agent(Path::new("/x/agents/dt.md"), raw).expect("parsed");
        assert_eq!(agent.effort, Some(Effort::High));
        assert_eq!(agent.max_turns, Some(25));
        assert_eq!(agent.memory, Some(MemoryScope::Project));
        assert_eq!(agent.mcp_servers, vec!["github", "search"]);
        assert_eq!(
            agent.hooks.get("pre-edit").map(|v| v.as_slice()),
            Some(&["./scripts/lint.sh".to_string()][..])
        );
        assert!(agent.system_prompt.contains("deep thinker"));
    }

    #[test]
    fn effort_xhigh_renames_normal() {
        let parsed: Effort = serde_yaml::from_str("xhigh").unwrap();
        assert_eq!(parsed, Effort::XHigh);
        let serialized = serde_yaml::to_string(&Effort::XHigh).unwrap();
        assert!(serialized.contains("xhigh"), "got: {serialized}");
    }

    #[test]
    fn effort_all_levels_round_trip_normal() {
        for (level, expected) in [
            (Effort::Minimal, "minimal"),
            (Effort::Low, "low"),
            (Effort::Medium, "medium"),
            (Effort::High, "high"),
            (Effort::XHigh, "xhigh"),
        ] {
            let s = serde_yaml::to_string(&level).unwrap();
            assert!(s.trim().contains(expected), "{level:?} → {s:?}");
        }
    }

    #[test]
    fn memory_scopes_all_three_parse_normal() {
        for (s, expected) in [
            ("user", MemoryScope::User),
            ("project", MemoryScope::Project),
            ("local", MemoryScope::Local),
        ] {
            let parsed: MemoryScope = serde_yaml::from_str(s).unwrap();
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn parse_agent_minimal_defaults_new_fields_robust() {
        let raw = "---\nname: bare\n---\nbody";
        let agent = parse_agent(Path::new("/x/bare.md"), raw).expect("parsed");
        assert_eq!(agent.effort, None);
        assert_eq!(agent.max_turns, None);
        assert_eq!(agent.memory, None);
        assert!(agent.mcp_servers.is_empty());
        assert!(agent.hooks.is_empty());
    }

    #[test]
    fn unknown_effort_value_returns_none_robust() {
        let raw = "---\nname: bad\neffort: ultra\n---\nbody";
        let result = parse_agent(Path::new("/x/bad.md"), raw);
        assert!(result.is_none());
    }
}
