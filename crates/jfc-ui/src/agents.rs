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
    pub system_prompt: String,
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
}
