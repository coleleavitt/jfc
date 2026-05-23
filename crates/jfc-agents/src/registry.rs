//! Agent and skill registry: filesystem loaders, built-in agent definitions,
//! and the `find_skill_by_name` lookup helper.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::state::{Skill, parse_agent, parse_skill};
pub use jfc_core::{AgentCost, AgentDef};

// ─── Skill loading ────────────────────────────────────────────────────────────

/// Load every skill discoverable from project + user roots. Project skills
/// override user skills with the same name.
pub fn load_skills(project_root: &Path) -> Vec<Skill> {
    tracing::info!(target: "jfc::agents", project_root = %project_root.display(), "loading skills");
    let mut out: Vec<Skill> = Vec::new();
    for root in skill_roots(project_root) {
        for candidate in skill_candidates(&root.path) {
            let SkillCandidate {
                md_path,
                fallback_name,
            } = candidate;
            let Ok(raw) = std::fs::read_to_string(&md_path) else {
                continue;
            };
            let Some(mut skill) = parse_skill(&md_path, &raw) else {
                continue;
            };
            let frontmatter_set_name = !skill.name.is_empty()
                && skill.name != "unnamed"
                && skill.name != "SKILL"
                && skill.name != "Skill"
                && skill.name != "skill";
            if !frontmatter_set_name {
                skill.name = fallback_name;
            }
            if let Some(namespace) = &root.namespace
                && !skill.name.contains(':')
            {
                skill.name = format!("{namespace}:{}", skill.name);
            }
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

#[derive(Debug)]
struct SkillRoot {
    path: PathBuf,
    namespace: Option<String>,
}

#[derive(Debug)]
struct SkillCandidate {
    md_path: PathBuf,
    fallback_name: String,
}

fn skill_roots(project_root: &Path) -> Vec<SkillRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    let mut push_root = |path: PathBuf, namespace: Option<String>| {
        if seen.insert((path.clone(), namespace.clone())) {
            roots.push(SkillRoot { path, namespace });
        }
    };

    if let Some(home) = dirs::home_dir() {
        push_root(home.join(".claude/skills"), None);
        push_root(home.join(".codex/skills"), None);
        push_root(home.join(".agents/skills"), None);
    }

    push_root(project_root.join(".claude/skills"), None);
    push_root(project_root.join(".codex/skills"), None);
    push_root(project_root.join(".agents/skills"), None);
    push_plugin_skill_roots(project_root, ".agents", &mut push_root);
    push_plugin_skill_roots(project_root, ".codex", &mut push_root);

    roots
}

fn push_plugin_skill_roots(
    project_root: &Path,
    config_dir: &str,
    push_root: &mut impl FnMut(PathBuf, Option<String>),
) {
    let plugins_dir = project_root.join(config_dir).join("plugins");
    let Ok(entries) = std::fs::read_dir(plugins_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_dir() {
            continue;
        }
        let Some(plugin) = path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.starts_with('.'))
        else {
            continue;
        };
        push_root(path.join("skills"), Some(plugin.to_owned()));
    }
}

fn skill_candidates(root: &Path) -> Vec<SkillCandidate> {
    const MAX_SCAN_DEPTH: usize = 8;
    const MAX_DIRS: usize = 512;

    if !root.is_dir() {
        return Vec::new();
    }

    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut seen_dirs = HashSet::new();
    if let Ok(canon) = root.canonicalize() {
        seen_dirs.insert(canon);
    }

    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if file_name.starts_with('.') {
                continue;
            }

            if path.is_dir() {
                if depth >= MAX_SCAN_DEPTH || seen_dirs.len() >= MAX_DIRS {
                    continue;
                }
                if let Ok(canon) = path.canonicalize()
                    && seen_dirs.insert(canon)
                {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }

            if !path.is_file() {
                continue;
            }

            if file_name.eq_ignore_ascii_case("SKILL.md") {
                let fallback_name = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("unnamed")
                    .to_owned();
                out.push(SkillCandidate {
                    md_path: path,
                    fallback_name,
                });
            } else if depth == 0 && path.extension().and_then(|s| s.to_str()) == Some("md") {
                let fallback_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unnamed")
                    .to_owned();
                out.push(SkillCandidate {
                    md_path: path,
                    fallback_name,
                });
            }
        }
    }

    out
}

// ─── Agent loading ────────────────────────────────────────────────────────────

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
    // Prepend built-in agents (user-defined agents with same name override them)
    for builtin in built_in_agents() {
        if !out.iter().any(|a| a.name == builtin.name) {
            out.push(builtin);
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

/// Look up a skill by `name` in a slice. Returns the first match or `None`.
pub fn find_skill_by_name<'a>(all_skills: &'a [Skill], name: &str) -> Option<&'a Skill> {
    let result = all_skills
        .iter()
        .find(|s| s.name.eq_ignore_ascii_case(name));
    tracing::trace!(
        target: "jfc::agents",
        name,
        found = result.is_some(),
        "find_skill_by_name"
    );
    result
}

// ─── Built-in Agent Definitions ──────────────────────────────────────────────

/// Construct an `AgentDef` with built-in defaults; caller patches the fields that differ.
fn builtin(name: &str, prompt_file: &str) -> AgentDef {
    AgentDef {
        name: name.into(),
        source: PathBuf::from("built-in"),
        model: None,
        isolation: None,
        skills: Vec::new(),
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        permission_mode: None,
        forks_parent_context: None,
        background: None,
        color: None,
        effort: None,
        max_turns: None,
        max_input_tokens: None,
        memory: None,
        mcp_servers: Vec::new(),
        hooks: std::collections::HashMap::new(),
        key_trigger: None,
        use_when: Vec::new(),
        avoid_when: Vec::new(),
        cost: None,
        system_prompt: prompt_file.to_owned(),
    }
}

fn strs(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

/// Returns the built-in agent definitions that ship with jfc.
pub fn built_in_agents() -> Vec<AgentDef> {
    let read_only_tools = strs(&["Read", "Glob", "Grep", "Bash"]);
    let no_write_tools = strs(&["Task", "Edit", "Write", "ApplyPatch"]);
    let no_write_only = strs(&["Edit", "Write", "ApplyPatch"]);

    vec![
        {
            let mut a = builtin(
                "general-purpose",
                include_str!("builtin_prompts/general_purpose.txt"),
            );
            a.key_trigger = Some("ambiguous / multi-step user request → general-purpose handles when no specialist fits".into());
            a.use_when = strs(&[
                "request spans multiple unrelated concerns",
                "user prompt doesn't match a more specific agent's domain",
            ]);
            a.avoid_when = strs(&[
                "the request is read-only exploration → fire Explore instead",
                "the request is plan-only design → fire Plan instead",
            ]);
            a.cost = Some(AgentCost::Expensive);
            a
        },
        {
            let mut a = builtin("Explore", include_str!("builtin_prompts/explore.txt"));
            a.model = Some("haiku".into());
            a.allowed_tools = read_only_tools.clone();
            a.disallowed_tools = no_write_tools.clone();
            a.key_trigger = Some("broad codebase exploration / 2+ modules / unfamiliar structure → fire Explore in background".into());
            a.use_when = strs(&[
                "user asks 'how does X work', 'find Y', 'where is Z', 'look into …'",
                "request spans 2+ files or modules",
                "you don't know exact file locations and the search would take >3 grep calls",
                "multiple search angles would strengthen understanding",
            ]);
            a.avoid_when = strs(&[
                "you already know the exact file (Read directly)",
                "single-keyword grep would suffice (Grep directly)",
                "Explore is already running on the same question (Delegation Trust Rule)",
            ]);
            a.cost = Some(AgentCost::Cheap);
            a
        },
        {
            let mut a = builtin("Plan", include_str!("builtin_prompts/plan.txt"));
            a.allowed_tools = read_only_tools.clone();
            a.disallowed_tools = no_write_tools.clone();
            a.key_trigger = Some(
                "multi-step / risky / cross-cutting change → fire Plan before any destructive edit"
                    .into(),
            );
            a.use_when = strs(&[
                "user asks 'how should I implement X', 'design Y', 'plan the Z refactor'",
                "the change touches 3+ files / 2+ modules and you don't have a clear approach",
                "the change is irreversible (schema migration, public API change, large refactor)",
            ]);
            a.avoid_when = strs(&[
                "the change is a one-liner with obvious scope",
                "the user already gave a step-by-step plan",
            ]);
            a.cost = Some(AgentCost::Expensive);
            a
        },
        {
            let mut a = builtin(
                "verification",
                include_str!("builtin_prompts/verification.txt"),
            );
            a.allowed_tools = strs(&[
                "Read",
                "Glob",
                "Grep",
                "Bash",
                "TaskList",
                "TaskGet",
                "TaskUpdate",
                "TaskDone",
            ]);
            a.disallowed_tools = no_write_tools;
            a.background = Some(true);
            a.color = Some("red".into());
            a.key_trigger = Some("after every non-trivial edit → fire verification in background to actually run + test".into());
            a.use_when = strs(&[
                "you just finished a feature, fix, or refactor and the user wants confidence",
                "the change touches a runtime path (server / CLI / build pipeline)",
                "tests exist and the user expects you to run them",
            ]);
            a.avoid_when = strs(&[
                "the change was a doc / comment edit only",
                "the user asked you NOT to run tests this turn",
            ]);
            a.cost = Some(AgentCost::Cheap);
            a
        },
        {
            let mut a = builtin(
                "orchestrator",
                include_str!("builtin_prompts/orchestrator.txt"),
            );
            a.allowed_tools = strs(&[
                "Read",
                "Glob",
                "Grep",
                "Bash",
                "TaskCreate",
                "TaskList",
                "TaskGet",
                "TaskUpdate",
                "TaskDone",
                "TaskValidate",
                "AskUserQuestion",
                "EnterPlanMode",
                "ExitPlanMode",
            ]);
            a.disallowed_tools = no_write_only;
            a.color = Some("magenta".into());
            a.max_turns = Some(8);
            a.key_trigger = Some("vague multi-area request → fire orchestrator to decompose into N concrete subtasks before authorizing work".into());
            a.use_when = strs(&[
                "user request is broad: 'fix all the auth bugs', 'modernize the build', 'audit security'",
                "you can't tell what 'done' looks like without scoping",
                "the work would touch >5 files across multiple subsystems",
            ]);
            a.avoid_when = strs(&[
                "user already gave a numbered plan",
                "the request is concrete (Edit / Write / single bug fix)",
                "Plan agent fits better — Plan designs the *how* for one task; orchestrator decomposes a wide request into many tasks",
            ]);
            a.cost = Some(AgentCost::Cheap);
            a
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lifecycle::{
        build_agent_system_prompt, render_dispatch_section, render_skills_section,
    };

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
            max_input_tokens: None,
            memory: None,
            mcp_servers: Vec::new(),
            hooks: std::collections::HashMap::new(),
            key_trigger: None,
            use_when: Vec::new(),
            avoid_when: Vec::new(),
            cost: None,
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

    fn skill(name: &str, description: Option<&str>) -> Skill {
        Skill {
            name: name.to_owned(),
            source: PathBuf::from("/x/skills/x.md"),
            description: description.map(str::to_owned),
            body: String::new(),
        }
    }

    #[test]
    fn load_skills_empty_root_normal() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_skills_empty_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let _ = load_skills(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_skills_finds_directory_based_skill_robust() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_skill_dir_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skills_dir = tmp.join(".claude/skills");
        let do178b_dir = skills_dir.join("do-178b");
        std::fs::create_dir_all(&do178b_dir).unwrap();
        std::fs::write(
            do178b_dir.join("SKILL.md"),
            "---\ndescription: avionics certification reference\n---\n# DO-178B\n\nbody",
        )
        .unwrap();
        let skills = load_skills(&tmp);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"do-178b"),
            "expected `do-178b` in loaded skills, got {names:?}"
        );
        let s = skills.iter().find(|s| s.name == "do-178b").unwrap();
        assert_eq!(
            s.description.as_deref(),
            Some("avionics certification reference")
        );
        assert!(s.body.contains("body"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn built_in_agents_includes_canonical_set_normal() {
        let agents = built_in_agents();
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        for needed in ["general-purpose", "Explore", "Plan", "verification"] {
            assert!(
                names.contains(&needed),
                "built-in {needed} missing from {names:?}"
            );
        }
        let explore = agents.iter().find(|a| a.name == "Explore").unwrap();
        assert_eq!(explore.model.as_deref(), Some("haiku"));
        assert!(explore.allowed_tools.iter().any(|t| t == "Read"));
        assert!(explore.disallowed_tools.iter().any(|t| t == "Edit"));
        assert!(!explore.system_prompt.is_empty());
    }

    #[test]
    fn find_skill_by_name_exact_normal() {
        let skills = vec![make_skill("explain", ""), make_skill("review", "")];
        let hit = find_skill_by_name(&skills, "explain").expect("found");
        assert_eq!(hit.name, "explain");
    }

    #[test]
    fn find_skill_by_name_case_insensitive_robust() {
        let skills = vec![make_skill("explain", "")];
        let hit = find_skill_by_name(&skills, "EXPLAIN").expect("found");
        assert_eq!(hit.name, "explain");
    }

    #[test]
    fn render_skills_section_empty_returns_empty_normal() {
        assert_eq!(render_skills_section(&[]), "");
    }

    #[test]
    fn render_skills_section_renders_each_skill_normal() {
        let skills = vec![
            skill("first", Some("does the first thing")),
            skill("second", Some("does the second thing")),
        ];
        let out = render_skills_section(&skills);
        assert!(out.contains("- `first` — does the first thing\n"));
        assert!(out.contains("- `second` — does the second thing\n"));
    }

    #[test]
    fn render_dispatch_section_empty_when_no_triggers_normal() {
        let agents = vec![make_agent("plain", "system", vec![])];
        let out = render_dispatch_section(&agents);
        assert_eq!(out, "");
    }

    #[test]
    fn render_dispatch_section_includes_triggers_normal() {
        let mut a = make_agent("Explore", "...", vec![]);
        a.key_trigger = Some("broad codebase exploration → fire Explore".into());
        a.use_when = vec!["how does X work".into()];
        a.avoid_when = vec!["already running".into()];
        a.cost = Some(AgentCost::Cheap);
        let out = render_dispatch_section(&[a]);
        assert!(out.contains("Default Bias: DELEGATE"), "{out}");
        assert!(out.contains("Explore"), "{out}");
    }

    #[test]
    fn build_agent_system_prompt_no_skills_returns_base_normal() {
        let agent = make_agent("a", "You are an agent.", Vec::new());
        let out = build_agent_system_prompt(&agent, &[]);
        assert_eq!(out, "You are an agent.");
    }

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
    }
}
