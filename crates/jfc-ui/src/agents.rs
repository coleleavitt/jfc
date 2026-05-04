//! v126 agent + skill loaders.
//!
//! Mirrors the layout from the v126 architecture spec:
//! - Skills live in `<project>/.claude/skills/*.md` (and user/`~/.claude/skills/`)
//! - Agents live in `<project>/.claude/agents/*.md` (and user/`~/.claude/agents/`)
//! - Both files use YAML frontmatter (between `---` delimiters) followed by a
//!   markdown body that becomes the system-prompt fragment.
//!
//! This module parses those files and returns structured records. Wiring into
//! the actual spawn/inject pipeline is up to callers (slash commands, the
//! Skill tool, the Task tool).
//!
//! What's intentionally NOT here:
//! - The teammate lifecycle (spawn / idle / dismiss) — that's a multi-process
//!   undertaking; the loader only parses the static definitions.
//! - Worktree creation — `git worktree add` is its own can of worms; the
//!   `isolation` field is parsed and surfaced for the caller to act on.
//! - Remote / marketplace skills — only filesystem sources for now.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// A loaded skill: frontmatter metadata + markdown body. The body becomes a
/// `<skill_content>` system message when the skill is invoked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub source: PathBuf,
    pub description: Option<String>,
    pub body: String,
}

/// A loaded agent definition. Mirrors the v126 schema:
///
/// ```yaml
/// ---
/// name: my-agent
/// model: opus
/// isolation: worktree
/// skills: [my-skill]
/// allowedTools: [Read, Edit, Bash]
/// disallowedTools: [Task]
/// permissionMode: acceptEdits
/// forksParentContext: true
/// ---
/// # System Prompt
/// You are …
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDef {
    pub name: String,
    pub source: PathBuf,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub isolation: Option<String>,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default, rename = "allowedTools")]
    pub allowed_tools: Vec<String>,
    #[serde(default, rename = "disallowedTools")]
    pub disallowed_tools: Vec<String>,
    #[serde(default, rename = "permissionMode")]
    pub permission_mode: Option<PermissionMode>,
    #[serde(default, rename = "forksParentContext")]
    pub forks_parent_context: Option<serde_json::Value>,
    #[serde(default)]
    pub background: Option<bool>,
    #[serde(default)]
    pub color: Option<String>,
    /// OpenAI reasoning_effort knob (cli.js:225236-225238). Untyped at the
    /// agent layer — providers translate.
    #[serde(default)]
    pub effort: Option<Effort>,
    /// Upper bound on agentic-loop iterations (cli.js:225244). Used by the
    /// dispatcher to fail-safe a runaway agent.
    #[serde(default, rename = "maxTurns")]
    pub max_turns: Option<u32>,
    /// Memory scope for stored snippets (cli.js:225233). `user` = global,
    /// `project` = .claude/memory/, `local` = ephemeral.
    #[serde(default)]
    pub memory: Option<MemoryScope>,
    /// MCP servers this agent has permission to talk to (cli.js:225242).
    /// Just a name list — enforcement lives in the MCP dispatcher.
    #[serde(default, rename = "mcpServers")]
    pub mcp_servers: Vec<String>,
    /// Pre/post hooks keyed by event name (cli.js:225242). Values are
    /// shell commands. `pre-edit`, `post-test`, `pre-bash`, etc.
    #[serde(default)]
    pub hooks: std::collections::HashMap<String, Vec<String>>,
    pub system_prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Minimal,
    Low,
    Medium,
    High,
    /// `xhigh` (rather than `x_high`) matches v126's serialized form.
    #[serde(rename = "xhigh")]
    XHigh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MemoryScope {
    User,
    Project,
    Local,
}

/// v126 permission modes — controls how tool calls are gated. `Auto` = LLM
/// classifier decides (jfc's `auto_mode`). Defaults to `Default` (prompt the
/// user for dangerous ops).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PermissionMode {
    /// Prompt for dangerous ops (Edit, Bash, Write, ApplyPatch).
    Default,
    /// Auto-accept file edits (Edit/Write/ApplyPatch); still prompt for Bash.
    AcceptEdits,
    /// Auto-accept everything — explicit opt-in only.
    BypassPermissions,
    /// Analysis only — no tool execution at all.
    Plan,
    /// Deny if not pre-approved (no prompts).
    DontAsk,
    /// LLM classifier decides per call.
    Auto,
}

impl Default for PermissionMode {
    fn default() -> Self {
        Self::Default
    }
}

/// Load every skill discoverable from project + user roots. Project skills
/// override user skills with the same name.
pub fn load_skills(project_root: &Path) -> Vec<Skill> {
    let mut out: Vec<Skill> = Vec::new();
    let user_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude/skills");
    let project_dir = project_root.join(".claude/skills");
    for dir in [user_dir, project_dir] {
        if !dir.exists() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(skill) = parse_skill(&path, &raw) else {
                continue;
            };
            // Project entries arrive after user, so retain wins overrides
            // by removing the prior entry with the same name first.
            out.retain(|s| s.name != skill.name);
            out.push(skill);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Same precedence rules as `load_skills`, but for agent definitions.
pub fn load_agents(project_root: &Path) -> Vec<AgentDef> {
    let mut out: Vec<AgentDef> = Vec::new();
    let user_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/"))
        .join(".claude/agents");
    let project_dir = project_root.join(".claude/agents");
    for dir in [user_dir, project_dir] {
        if !dir.exists() {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let Ok(raw) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(agent) = parse_agent(&path, &raw) else {
                continue;
            };
            out.retain(|a| a.name != agent.name);
            out.push(agent);
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// Parse a skill .md file: optional YAML frontmatter (between `---` lines)
/// followed by a markdown body. Frontmatter fields: `name`, `description`.
/// If `name` is missing, falls back to the filename stem.
fn parse_skill(path: &Path, raw: &str) -> Option<Skill> {
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

fn parse_agent(path: &Path, raw: &str) -> Option<AgentDef> {
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
        memory: parsed.memory,
        mcp_servers: parsed.mcp_servers.unwrap_or_default(),
        hooks: parsed.hooks.unwrap_or_default(),
        system_prompt: body.trim().to_owned(),
    })
}

fn split_frontmatter(raw: &str) -> (Option<&str>, &str) {
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
    name: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentFront {
    name: String,
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    isolation: Option<String>,
    #[serde(default)]
    skills: Option<Vec<String>>,
    #[serde(default, rename = "allowedTools")]
    allowed_tools: Option<Vec<String>>,
    #[serde(default, rename = "disallowedTools")]
    disallowed_tools: Option<Vec<String>>,
    #[serde(default, rename = "permissionMode")]
    permission_mode: Option<PermissionMode>,
    #[serde(default, rename = "forksParentContext")]
    forks_parent_context: Option<serde_json::Value>,
    #[serde(default)]
    background: Option<bool>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    effort: Option<Effort>,
    #[serde(default, rename = "maxTurns")]
    max_turns: Option<u32>,
    #[serde(default)]
    memory: Option<MemoryScope>,
    #[serde(default, rename = "mcpServers")]
    mcp_servers: Option<Vec<String>>,
    #[serde(default)]
    hooks: Option<std::collections::HashMap<String, Vec<String>>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    // Normal: a well-formed skill file with frontmatter parses into a
    // Skill record.
    #[test]
    fn parse_skill_with_frontmatter_normal() {
        let raw = "---\nname: my-skill\ndescription: A test skill\n---\n# Body\n\nDo the thing.";
        let s = parse_skill(Path::new("/x/skills/my.md"), raw).expect("parsed");
        assert_eq!(s.name, "my-skill");
        assert_eq!(s.description.as_deref(), Some("A test skill"));
        assert!(s.body.contains("Do the thing"));
    }

    // Normal: a skill without frontmatter still parses, falling back to the
    // filename stem for the `name` field.
    #[test]
    fn parse_skill_no_frontmatter_uses_filename_stem_normal() {
        let s = parse_skill(Path::new("/x/skills/snake.md"), "Just a body").expect("parsed");
        assert_eq!(s.name, "snake");
        assert_eq!(s.description, None);
        assert_eq!(s.body, "Just a body");
    }

    // Normal: a well-formed agent file parses into an AgentDef.
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

    // Robust: an agent file without frontmatter is rejected — we need at
    // least the `name` field. Returns None.
    #[test]
    fn parse_agent_no_frontmatter_returns_none_robust() {
        let s = parse_agent(Path::new("/x/agents/x.md"), "Just a body");
        assert!(s.is_none());
    }

    // Robust: malformed YAML in the frontmatter returns None for agents
    // (which require `name`). Skills tolerate it (fallback to filename).
    #[test]
    fn parse_agent_malformed_yaml_returns_none_robust() {
        let raw = "---\nname: [missing close bracket\n---\nbody";
        assert!(parse_agent(Path::new("/x/a.md"), raw).is_none());
    }

    // Normal: `split_frontmatter` extracts YAML between `---` delimiters.
    #[test]
    fn split_frontmatter_extracts_yaml_normal() {
        let raw = "---\nkey: value\n---\nbody";
        let (front, body) = split_frontmatter(raw);
        assert_eq!(front, Some("key: value"));
        assert_eq!(body, "body");
    }

    // Normal: PermissionMode round-trips through serde for all variants.
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
        // Every new field at once — confirms `effort`/`maxTurns`/`memory`/
        // `mcpServers`/`hooks` all land via the existing parse path. v126
        // schema reference: cli.js:225207-225281.
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
        // v126 emits `xhigh` as one token, not `x_high` like serde's
        // default kebab-from-PascalCase rename would produce. Pin the
        // explicit `#[serde(rename = "xhigh")]` so a future cleanup
        // doesn't regress to the snake-cased form.
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
        // Only `name` set — every new field defaults to None / empty.
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
        // A typo'd effort like `ultra` (not in the enum) shouldn't
        // crash the loader — `parse_agent` returns None for the
        // whole file when its frontmatter fails to parse, so the
        // bad agent is silently skipped rather than poisoning the
        // registry.
        let raw = "---\nname: bad\neffort: ultra\n---\nbody";
        let result = parse_agent(Path::new("/x/bad.md"), raw);
        assert!(result.is_none());
    }
}
