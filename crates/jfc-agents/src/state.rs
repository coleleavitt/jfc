//! Agent and skill data types, serde frontmatter structs, and low-level
//! parsing helpers (`parse_skill`, `parse_agent`, `split_frontmatter`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub use jfc_core::{AgentCost, AgentDef, Effort, MemoryScope, PermissionMode};

/// How a skill should be executed when it is invoked directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillContext {
    /// Inline the rendered skill body into the current turn.
    #[default]
    Inline,
    /// Run the rendered skill body in a forked subagent.
    Fork,
}

impl SkillContext {
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(str::trim).map(str::to_ascii_lowercase).as_deref() {
            Some("fork" | "subagent" | "background") => Self::Fork,
            _ => Self::Inline,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Inline => "inline",
            Self::Fork => "fork",
        }
    }

    pub fn is_fork(self) -> bool {
        matches!(self, Self::Fork)
    }
}

/// A non-`SKILL.md` file that belongs to the same skill package.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillFile {
    /// Path relative to [`Skill::package_root`].
    pub relative_path: String,
    /// Absolute or process-relative path on disk.
    pub path: PathBuf,
    /// File size from metadata, in bytes.
    pub bytes: u64,
}

/// A loaded skill package: frontmatter metadata, markdown body, and attached
/// package files. The rendered body becomes a tool result or subagent prompt
/// when the skill is invoked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub source: PathBuf,
    pub description: Option<String>,
    pub body: String,
    pub user_invocable: bool,
    pub context: SkillContext,
    pub package_root: PathBuf,
    pub files: Vec<SkillFile>,
    pub allowed_tools: Vec<String>,
    pub disallowed_tools: Vec<String>,
    pub mcp_servers: Vec<String>,
    pub input_schema: Option<serde_json::Value>,
    pub schedule: Option<String>,
    /// `argument-hint` frontmatter — a short usage hint shown in the command
    /// palette (e.g. `<file> [--flag]`). Display-only.
    pub argument_hint: Option<String>,
    /// `model` frontmatter — the skill's preferred model id. When set, invoking
    /// the skill suggests switching to this model.
    pub model: Option<String>,
    /// `effort` frontmatter — the skill's preferred reasoning effort
    /// (low/medium/high/xhigh/max), applied for the skill's turn.
    pub effort: Option<String>,
    /// `disable-model-invocation` frontmatter — when true the model may NOT
    /// auto-invoke this skill via the Skill tool; only the user can run it.
    pub disable_model_invocation: bool,
    /// `created-by` frontmatter — `"agent"` for skills the agent authored from
    /// experience (curation-eligible), `"user"` (default) otherwise. Lets the
    /// skill curator know which skills it owns and may auto-archive.
    #[serde(default)]
    pub created_by: SkillOrigin,
}

/// Provenance of a skill — who authored it. Only [`SkillOrigin::Agent`] skills
/// are eligible for automatic curation (stale/archive).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum SkillOrigin {
    /// Authored by the user / shipped with the project. Never auto-curated.
    #[default]
    User,
    /// Written by the agent from a successful trajectory. Curation-eligible.
    Agent,
}

impl Skill {
    pub fn new(name: String, source: PathBuf, description: Option<String>, body: String) -> Self {
        let package_root = source
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            name,
            source,
            description,
            body,
            user_invocable: true,
            context: SkillContext::Inline,
            package_root,
            files: Vec::new(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            mcp_servers: Vec::new(),
            input_schema: None,
            schedule: None,
            argument_hint: None,
            model: None,
            effort: None,
            disable_model_invocation: false,
            created_by: SkillOrigin::User,
        }
    }

    pub fn is_user_invocable(&self) -> bool {
        self.user_invocable
    }

    /// Whether the *model* may auto-invoke this skill via the Skill tool.
    /// `disable-model-invocation: true` makes a skill user-only — it stays in
    /// the command palette but is hidden from the model's skill catalog.
    pub fn is_model_invocable(&self) -> bool {
        !self.disable_model_invocation
    }

    pub fn is_system_skill(&self) -> bool {
        let source = self.source.to_string_lossy();
        source.contains("/.codex/skills/.system/") || source.contains("/.agents/skills/.system/")
    }

    pub fn is_discoverable(&self) -> bool {
        let name = self.name.trim();
        !name.is_empty()
            && !name.starts_with("superpowers:")
            && self.is_user_invocable()
            && !self.is_system_skill()
    }
}

/// Runtime values available while rendering a skill body.
#[derive(Debug, Clone, Copy, Default)]
pub struct SkillRenderContext<'a> {
    pub project_root: Option<&'a Path>,
    pub memory_root: Option<&'a Path>,
}

impl<'a> SkillRenderContext<'a> {
    pub fn new(project_root: Option<&'a Path>, memory_root: Option<&'a Path>) -> Self {
        Self {
            project_root,
            memory_root,
        }
    }
}

/// Render a skill body for invocation: expand known placeholders, surface
/// package attachments as readable paths, and append caller arguments.
pub fn render_skill_invocation(
    skill: &Skill,
    context: SkillRenderContext<'_>,
    args: Option<&str>,
) -> String {
    let mut out = expand_skill_placeholders(skill, &skill.body, context, args);
    append_skill_files(skill, &mut out);
    if let Some(args) = args.map(str::trim).filter(|s| !s.is_empty()) {
        if out.ends_with('\n') {
            out.push('\n');
        } else {
            out.push_str("\n\n");
        }
        out.push_str("# Args\n");
        out.push_str(args);
    }
    out
}

fn expand_skill_placeholders(
    skill: &Skill,
    body: &str,
    context: SkillRenderContext<'_>,
    args: Option<&str>,
) -> String {
    let mut out = body.to_owned();
    replace_placeholder(&mut out, "SKILL_NAME", &skill.name);
    replace_placeholder(
        &mut out,
        "SKILL_ROOT",
        &skill.package_root.to_string_lossy(),
    );
    if let Some(project_root) = context.project_root {
        replace_placeholder(&mut out, "PROJECT_ROOT", &project_root.to_string_lossy());
    }
    if let Some(memory_root) = context.memory_root {
        replace_placeholder(&mut out, "MEMORY_ROOT", &memory_root.to_string_lossy());
    }
    replace_placeholder(&mut out, "ARGS", args.unwrap_or_default());
    out
}

fn replace_placeholder(out: &mut String, key: &str, value: &str) {
    let placeholder = format!("{{{{{key}}}}}");
    if out.contains(&placeholder) {
        *out = out.replace(&placeholder, value);
    }
}

fn append_skill_files(skill: &Skill, out: &mut String) {
    if skill.files.is_empty() {
        return;
    }
    if out.ends_with('\n') {
        out.push('\n');
    } else {
        out.push_str("\n\n");
    }
    out.push_str("# Skill Package Files\n");
    out.push_str("These files ship with the skill package and can be read from disk if needed:\n");
    for file in &skill.files {
        out.push_str(&format!(
            "- `{}` — `{}` ({} bytes)\n",
            file.relative_path,
            file.path.display(),
            file.bytes
        ));
    }
}

/// Parse a skill .md file: optional YAML frontmatter (between `---` lines)
/// followed by a markdown body. If `name` is missing, falls back to the
/// filename stem.
pub fn parse_skill(path: &Path, raw: &str) -> Option<Skill> {
    let (front, body) = split_frontmatter(raw);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unnamed");
    let mut name = stem.to_owned();
    let mut description = None;
    let mut user_invocable = true;
    let mut context = SkillContext::Inline;
    let mut allowed_tools = Vec::new();
    let mut disallowed_tools = Vec::new();
    let mut mcp_servers = Vec::new();
    let mut input_schema = None;
    let mut schedule = None;
    let mut argument_hint = None;
    let mut model = None;
    let mut effort = None;
    let mut disable_model_invocation = false;
    let mut created_by = SkillOrigin::User;
    if let Some(yaml) = front
        && let Ok(parsed) = serde_yaml::from_str::<SkillFront>(yaml)
    {
        if let Some(n) = parsed.name {
            name = n;
        }
        description = parsed.description;
        if let Some(v) = parsed.user_invocable {
            user_invocable = v;
        }
        context = SkillContext::parse(parsed.context.as_deref());
        allowed_tools = parsed.allowed_tools.unwrap_or_default();
        disallowed_tools = parsed.disallowed_tools.unwrap_or_default();
        mcp_servers = parsed.mcp_servers.unwrap_or_default();
        input_schema = parsed.input_schema;
        schedule = parsed.schedule;
        argument_hint = parsed.argument_hint;
        model = parsed.model;
        effort = parsed.effort;
        disable_model_invocation = parsed.disable_model_invocation.unwrap_or(false);
        if matches!(parsed.created_by.as_deref(), Some("agent")) {
            created_by = SkillOrigin::Agent;
        }
    }
    let mut skill = Skill::new(
        name,
        path.to_path_buf(),
        description,
        body.trim().to_owned(),
    );
    skill.user_invocable = user_invocable;
    skill.context = context;
    skill.allowed_tools = allowed_tools;
    skill.disallowed_tools = disallowed_tools;
    skill.mcp_servers = mcp_servers;
    skill.input_schema = input_schema;
    skill.schedule = schedule;
    skill.argument_hint = argument_hint;
    skill.model = model;
    skill.effort = effort;
    skill.disable_model_invocation = disable_model_invocation;
    skill.created_by = created_by;
    Some(skill)
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
    #[serde(default, rename = "user-invocable", alias = "userInvocable")]
    pub user_invocable: Option<bool>,
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default, rename = "allowed-tools", alias = "allowedTools")]
    pub allowed_tools: Option<Vec<String>>,
    #[serde(default, rename = "disallowed-tools", alias = "disallowedTools")]
    pub disallowed_tools: Option<Vec<String>>,
    #[serde(default, rename = "mcp-servers", alias = "mcpServers")]
    pub mcp_servers: Option<Vec<String>>,
    #[serde(default, rename = "input-schema", alias = "inputSchema")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default)]
    pub schedule: Option<String>,
    #[serde(default, rename = "argument-hint", alias = "argumentHint")]
    pub argument_hint: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub effort: Option<String>,
    #[serde(
        default,
        rename = "disable-model-invocation",
        alias = "disableModelInvocation"
    )]
    pub disable_model_invocation: Option<bool>,
    #[serde(default, rename = "created-by", alias = "createdBy")]
    pub created_by: Option<String>,
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
        let raw = "---\nname: my-skill\ndescription: A test skill\nuser-invocable: false\ncontext: fork\nallowed-tools:\n  - Read\nmcp-servers:\n  - github\nschedule: '@daily'\n---\n# Body\n\nDo the thing.";
        let s = parse_skill(Path::new("/x/skills/my.md"), raw).expect("parsed");
        assert_eq!(s.name, "my-skill");
        assert_eq!(s.description.as_deref(), Some("A test skill"));
        assert!(!s.user_invocable);
        assert_eq!(s.context, SkillContext::Fork);
        assert_eq!(s.allowed_tools, vec!["Read"]);
        assert_eq!(s.mcp_servers, vec!["github"]);
        assert_eq!(s.schedule.as_deref(), Some("@daily"));
        assert!(s.body.contains("Do the thing"));
    }

    #[test]
    fn parse_skill_no_frontmatter_uses_filename_stem_normal() {
        let s = parse_skill(Path::new("/x/skills/snake.md"), "Just a body").expect("parsed");
        assert_eq!(s.name, "snake");
        assert_eq!(s.description, None);
        assert_eq!(s.body, "Just a body");
        assert!(s.user_invocable);
        assert_eq!(s.context, SkillContext::Inline);
        // Defaults for the richer fields.
        assert_eq!(s.argument_hint, None);
        assert_eq!(s.model, None);
        assert_eq!(s.effort, None);
        assert!(!s.disable_model_invocation);
        assert!(s.is_model_invocable());
    }

    #[test]
    fn parse_skill_richer_frontmatter_normal() {
        let raw = "---\nname: deploy\ndescription: Ship it\nargument-hint: \"<env> [--dry-run]\"\nmodel: claude-opus-4-8\neffort: high\ndisable-model-invocation: true\n---\n# Deploy\n\nRun the deploy.";
        let s = parse_skill(Path::new("/x/skills/deploy.md"), raw).expect("parsed");
        assert_eq!(s.argument_hint.as_deref(), Some("<env> [--dry-run]"));
        assert_eq!(s.model.as_deref(), Some("claude-opus-4-8"));
        assert_eq!(s.effort.as_deref(), Some("high"));
        assert!(s.disable_model_invocation);
        // disable-model-invocation hides it from the model but not the user.
        assert!(!s.is_model_invocable());
        assert!(s.is_user_invocable());
    }

    #[test]
    fn parse_skill_camelcase_aliases_robust() {
        let raw = "---\nname: x\nargumentHint: \"<arg>\"\ndisableModelInvocation: true\n---\nbody";
        let s = parse_skill(Path::new("/x/skills/x.md"), raw).expect("parsed");
        assert_eq!(s.argument_hint.as_deref(), Some("<arg>"));
        assert!(s.disable_model_invocation);
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
