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
    tracing::info!(target: "jfc::agents", project_root = %project_root.display(), "loading skills");
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
    tracing::debug!(
        target: "jfc::agents",
        count = out.len(),
        names = ?out.iter().map(|s| &s.name).collect::<Vec<_>>(),
        "skills loaded"
    );
    out
}

/// Render the loaded skills as a Markdown listing for injection into the
/// system prompt. The model needs to know skills exist before it can ask to
/// invoke them — this is the discovery surface.
///
/// Format (matches v126's cli.js:48850 listing, lighter cap):
///
/// ```text
/// ## Available skills
///
/// - `skill-name` — short description
/// - `another-skill` — …
/// ```
///
/// Description is capped at 200 chars (with `…` ellipsis on overflow) to
/// keep per-turn token cost low — we re-inject on every stream call so
/// every char compounds. v126 uses 1536 because their listing is cached;
/// jfc's isn't yet.
///
/// Returns `""` when `skills` is empty so callers can unconditionally
/// `push_str` the result.
pub(crate) fn render_skills_section(skills: &[Skill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    const MAX_DESC_CHARS: usize = 200;
    let mut out = String::from("\n\n## Available skills\n\n");
    for skill in skills {
        match &skill.description {
            Some(desc) if !desc.is_empty() => {
                // Char-aware truncation — UTF-8 boundaries matter, and
                // .len() would count bytes not characters.
                let trimmed: String = if desc.chars().count() > MAX_DESC_CHARS {
                    let mut s: String = desc.chars().take(MAX_DESC_CHARS).collect();
                    s.push('…');
                    s
                } else {
                    desc.clone()
                };
                out.push_str(&format!("- `{}` — {}\n", skill.name, trimmed));
            }
            _ => {
                out.push_str(&format!("- `{}`\n", skill.name));
            }
        }
    }
    out
}

/// Look up a skill by `name` in a slice. Returns the first match or `None`.
/// Used by the agent dispatcher to resolve `agent.skills` entries before
/// concatenating their bodies into the agent's system prompt, and by the
/// `Skill` tool / slash dispatcher to resolve a user-typed name.
pub fn find_skill_by_name<'a>(all_skills: &'a [Skill], name: &str) -> Option<&'a Skill> {
    // Case-insensitive — `/Explain` should hit the same skill as `/explain`.
    let result = all_skills.iter().find(|s| s.name.eq_ignore_ascii_case(name));
    tracing::trace!(
        target: "jfc::agents",
        name,
        found = result.is_some(),
        "find_skill_by_name"
    );
    result
}

/// Build the effective system prompt for an agent: its own `system_prompt`
/// followed by each resolved skill body, separated by `## Skill: <name>`
/// headers. Unknown skill names are skipped (with a `tracing::warn!`).
pub(crate) fn build_agent_system_prompt(agent: &AgentDef, all_skills: &[Skill]) -> String {
    if agent.skills.is_empty() {
        return agent.system_prompt.clone();
    }
    let mut out = agent.system_prompt.clone();
    for name in &agent.skills {
        match find_skill_by_name(all_skills, name) {
            Some(skill) => {
                out.push_str("\n\n## Skill: ");
                out.push_str(&skill.name);
                out.push_str("\n\n");
                out.push_str(&skill.body);
            }
            None => {
                tracing::warn!(
                    target: "jfc::agents",
                    agent = %agent.name,
                    skill = %name,
                    "agent references unknown skill; skipping",
                );
            }
        }
    }
    out
}

/// Same precedence rules as `load_skills`, but for agent definitions.
pub fn load_agents(project_root: &Path) -> Vec<AgentDef> {
    tracing::info!(target: "jfc::agents", project_root = %project_root.display(), "loading agents");
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
    tracing::debug!(
        target: "jfc::agents",
        count = out.len(),
        names = ?out.iter().map(|a| &a.name).collect::<Vec<_>>(),
        "agents loaded"
    );
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

    // Helper: build a Skill with a fixed name/description, no body, dummy
    // source path. Keeps test bodies tight.
    fn skill(name: &str, description: Option<&str>) -> Skill {
        Skill {
            name: name.to_owned(),
            source: PathBuf::from("/x/skills/x.md"),
            description: description.map(str::to_owned),
            body: String::new(),
        }
    }

    // Normal: an empty slice yields the empty string so callers can
    // unconditionally `push_str` the result without polluting the prompt
    // with a header that has no items beneath it.
    #[test]
    fn render_skills_section_empty_returns_empty_normal() {
        assert_eq!(render_skills_section(&[]), "");
    }

    // Normal: each skill renders as a single bullet line containing the
    // backticked name, an em-dash separator, and the description.
    #[test]
    fn render_skills_section_renders_each_skill_normal() {
        let skills = vec![
            skill("first", Some("does the first thing")),
            skill("second", Some("does the second thing")),
        ];
        let out = render_skills_section(&skills);
        assert!(out.contains("- `first` — does the first thing\n"));
        assert!(out.contains("- `second` — does the second thing\n"));
        // Two lines for the two skills, plus header lines.
        assert_eq!(out.matches("\n- `").count(), 2);
    }

    // Normal: the rendered block leads with the `## Available skills`
    // header so the model can find it by section name.
    #[test]
    fn render_skills_section_starts_with_header_normal() {
        let out = render_skills_section(&[skill("only", Some("only one"))]);
        let first_lines: Vec<&str> = out.lines().take(4).collect();
        assert!(
            first_lines.iter().any(|l| l.contains("## Available skills")),
            "header missing from first 4 lines: {first_lines:?}"
        );
    }

    // Robust: a 500-char description is truncated to 200 chars + a single
    // ellipsis. The cap is char-based, not byte-based.
    #[test]
    fn render_skills_section_truncates_long_description_robust() {
        let long: String = "a".repeat(500);
        let out = render_skills_section(&[skill("big", Some(&long))]);
        // Find the line for our skill.
        let line = out
            .lines()
            .find(|l| l.starts_with("- `big`"))
            .expect("line for `big` skill");
        // Strip the leading `- \`big\` — ` prefix to isolate the description.
        let desc = line.strip_prefix("- `big` — ").expect("desc prefix");
        assert!(
            desc.ends_with('…'),
            "expected ellipsis suffix, got {desc:?}"
        );
        // 200 a's plus the ellipsis = 201 chars.
        assert_eq!(desc.chars().count(), 201);
    }

    // Robust: a skill with `description: None` renders as a bare bullet —
    // no em-dash, no trailing whitespace, no panic.
    #[test]
    fn render_skills_section_handles_no_description_robust() {
        let out = render_skills_section(&[skill("naked", None)]);
        assert!(out.contains("- `naked`\n"));
        // Must NOT contain the em-dash separator for this entry.
        assert!(
            !out.contains("- `naked` —"),
            "naked skill should not have a dash: {out:?}"
        );
    }

    fn make_agent(name: &str, system_prompt: &str, skills: Vec<String>) -> AgentDef {
        AgentDef {
            name: name.to_owned(),
            source: PathBuf::from(format!("/x/agents/{name}.md")),
            model: None,
            isolation: None,
            skills,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            permission_mode: None,
            forks_parent_context: None,
            background: None,
            color: None,
            system_prompt: system_prompt.to_owned(),
            effort: None,
            max_turns: None,
            memory: None,
            mcp_servers: Vec::new(),
            hooks: std::collections::HashMap::new(),
        }
    }

    fn make_skill(name: &str, body: &str) -> Skill {
        Skill {
            name: name.to_owned(),
            source: PathBuf::from(format!("/x/skills/{name}.md")),
            description: None,
            body: body.to_owned(),
        }
    }

    // Normal: an agent with no skills returns its base `system_prompt`
    // verbatim — no header, no trailing whitespace.
    #[test]
    fn build_agent_system_prompt_no_skills_returns_base_normal() {
        let agent = make_agent("a", "You are an agent.", Vec::new());
        let out = build_agent_system_prompt(&agent, &[]);
        assert_eq!(out, "You are an agent.");
    }

    // Normal: when an agent lists two skills that both resolve, both bodies
    // appear in the output, each preceded by a `## Skill: <name>` header.
    #[test]
    fn build_agent_system_prompt_appends_resolved_skills_normal() {
        let agent = make_agent(
            "impl",
            "Base prompt.",
            vec!["one".to_owned(), "two".to_owned()],
        );
        let skills = vec![
            make_skill("one", "Body of skill one."),
            make_skill("two", "Body of skill two."),
        ];
        let out = build_agent_system_prompt(&agent, &skills);
        assert!(out.starts_with("Base prompt."));
        assert!(out.contains("## Skill: one"));
        assert!(out.contains("## Skill: two"));
        assert!(out.contains("Body of skill one."));
        assert!(out.contains("Body of skill two."));
    }

    // Robust: a skill name that doesn't resolve in `all_skills` is silently
    // skipped — no crash, no placeholder. Other resolved skills still appear.
    #[test]
    fn build_agent_system_prompt_skips_unknown_skill_robust() {
        let agent = make_agent(
            "x",
            "Base.",
            vec!["missing-skill".to_owned(), "real".to_owned()],
        );
        let skills = vec![make_skill("real", "Real body.")];
        let out = build_agent_system_prompt(&agent, &skills);
        assert!(!out.contains("missing-skill"));
        assert!(out.contains("## Skill: real"));
        assert!(out.contains("Real body."));
    }

    // Robust: skill bodies appear in the order listed in `agent.skills`,
    // not the order of `all_skills`. Order matters for prompt composition.
    #[test]
    fn build_agent_system_prompt_preserves_order_robust() {
        let agent = make_agent("x", "Base.", vec!["a".to_owned(), "b".to_owned()]);
        // Pass `all_skills` in reverse to ensure the agent's order wins.
        let skills = vec![
            make_skill("b", "BBBB body."),
            make_skill("a", "AAAA body."),
        ];
        let out = build_agent_system_prompt(&agent, &skills);
        let pos_a = out.find("AAAA body.").expect("a present");
        let pos_b = out.find("BBBB body.").expect("b present");
        assert!(pos_a < pos_b, "skill 'a' must appear before skill 'b'");
    }

    // Normal: an exact lowercase match returns the matching skill.
    #[test]
    fn find_skill_by_name_exact_normal() {
        let skills = vec![make_skill("explain", ""), make_skill("review", "")];
        let hit = find_skill_by_name(&skills, "explain").expect("found");
        assert_eq!(hit.name, "explain");
    }

    // Robust: lookup is case-insensitive — "EXPLAIN" still finds "explain".
    #[test]
    fn find_skill_by_name_case_insensitive_robust() {
        let skills = vec![make_skill("explain", "")];
        let hit = find_skill_by_name(&skills, "EXPLAIN").expect("found");
        assert_eq!(hit.name, "explain");
    }

    // Robust: a name that doesn't match any loaded skill returns None rather
    // than a misleading partial hit.
    #[test]
    fn find_skill_by_name_unknown_returns_none_robust() {
        let skills = vec![make_skill("explain", "")];
        assert!(find_skill_by_name(&skills, "unknown-skill").is_none());
    }

    // Robust: an empty skills list returns None (no panic, no out-of-bounds).
    #[test]
    fn find_skill_by_name_empty_list_returns_none_robust() {
        assert!(find_skill_by_name(&[], "anything").is_none());
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
