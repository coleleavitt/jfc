//! Agent and skill registry: filesystem loaders, built-in agent definitions,
//! and the `find_skill_by_name` lookup helper.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use crate::builtins;
use crate::state::{Skill, SkillFile, parse_agent, parse_skill};
pub use jfc_core::{AgentCost, AgentDef};

#[derive(Debug, Clone)]
struct PluginRoot {
    path: PathBuf,
    namespace: Option<String>,
}

fn plugin_roots(project_root: &Path) -> Vec<PluginRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    let settings = jfc_config::claude_settings::load_merged(project_root);
    let mut push_root = |path: PathBuf, namespace: Option<String>| {
        if let Some(plugin) = namespace.as_deref()
            && !settings.plugin_enabled(plugin)
        {
            tracing::debug!(
                target: "jfc::agents",
                plugin,
                path = %path.display(),
                "plugin disabled by enabledPlugins setting"
            );
            return;
        }
        if seen.insert((path.clone(), namespace.clone())) {
            roots.push(PluginRoot { path, namespace });
        }
    };

    if let Some(home) = dirs::home_dir() {
        push_plugin_roots_in(&home.join(".claude/plugins"), &mut push_root);
    }
    if let Some(config) = dirs::config_dir() {
        push_plugin_roots_in(&config.join("jfc/plugins"), &mut push_root);
    }

    push_plugin_roots_in(&project_root.join(".claude/plugins"), &mut push_root);
    push_plugin_roots_in(&project_root.join("plugins"), &mut push_root);
    push_plugin_roots_in(&project_root.join(".agents/plugins"), &mut push_root);
    push_plugin_roots_in(&project_root.join(".codex/plugins"), &mut push_root);

    roots
}

fn push_plugin_roots_in(plugins_dir: &Path, push_root: &mut impl FnMut(PathBuf, Option<String>)) {
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
            .map(str::to_owned)
        else {
            continue;
        };
        push_root(path, Some(plugin));
    }
}

// ─── Skill loading ────────────────────────────────────────────────────────────

/// Load every skill discoverable from project + user roots. Project skills
/// override user skills with the same name.
pub fn load_skills(project_root: &Path) -> Vec<Skill> {
    tracing::info!(target: "jfc::agents", project_root = %project_root.display(), "loading skills");
    let mut out: Vec<Skill> = built_in_skills();
    for root in skill_roots(project_root) {
        for candidate in skill_candidates(&root.path) {
            let SkillCandidate {
                md_path,
                fallback_name,
                package_root,
            } = candidate;
            let Ok(raw) = std::fs::read_to_string(&md_path) else {
                continue;
            };
            let Some(mut skill) = parse_skill(&md_path, &raw) else {
                continue;
            };
            if let Some(package_root) = package_root {
                skill.package_root = package_root;
                skill.files = collect_skill_files(&skill.package_root, &skill.source);
            }
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

/// Returns the built-in skill definitions that ship with jfc.
pub fn built_in_skills() -> Vec<Skill> {
    builtins::built_in_skills()
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
    package_root: Option<PathBuf>,
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
    for plugin in plugin_roots(project_root) {
        push_root(plugin.path.join("skills"), plugin.namespace);
    }

    roots
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
                let package_root = path.parent().map(Path::to_path_buf);
                let fallback_name = path
                    .parent()
                    .and_then(|p| p.file_name())
                    .and_then(|s| s.to_str())
                    .unwrap_or("unnamed")
                    .to_owned();
                out.push(SkillCandidate {
                    md_path: path,
                    fallback_name,
                    package_root,
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
                    package_root: None,
                });
            }
        }
    }

    out
}

fn collect_skill_files(package_root: &Path, skill_md_path: &Path) -> Vec<SkillFile> {
    const MAX_SCAN_DEPTH: usize = 8;
    const MAX_DIRS: usize = 512;
    const MAX_FILES: usize = 256;

    if !package_root.is_dir() {
        return Vec::new();
    }

    let canonical_skill = skill_md_path.canonicalize().ok();
    let mut out = Vec::new();
    let mut queue = std::collections::VecDeque::from([(package_root.to_path_buf(), 0usize)]);
    let mut seen_dirs = HashSet::new();
    if let Ok(canon) = package_root.canonicalize() {
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
            if canonical_skill
                .as_ref()
                .is_some_and(|skill| path.canonicalize().ok().as_ref() == Some(skill))
            {
                continue;
            }

            let Ok(metadata) = std::fs::metadata(&path) else {
                continue;
            };
            let relative_path = path
                .strip_prefix(package_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.push(SkillFile {
                relative_path,
                path,
                bytes: metadata.len(),
            });
            if out.len() >= MAX_FILES {
                break;
            }
        }
        if out.len() >= MAX_FILES {
            break;
        }
    }

    out.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    out
}

// ─── Agent loading ────────────────────────────────────────────────────────────

#[derive(Debug)]
struct AgentRoot {
    path: PathBuf,
    namespace: Option<String>,
}

fn agent_roots(project_root: &Path) -> Vec<AgentRoot> {
    let mut roots = Vec::new();
    let mut seen = HashSet::new();
    let mut push_root = |path: PathBuf, namespace: Option<String>| {
        if seen.insert((path.clone(), namespace.clone())) {
            roots.push(AgentRoot { path, namespace });
        }
    };

    if let Some(home) = dirs::home_dir() {
        push_root(home.join(".claude/agents"), None);
    }
    push_root(project_root.join(".claude/agents"), None);
    for plugin in plugin_roots(project_root) {
        push_root(plugin.path.join("agents"), plugin.namespace);
    }

    roots
}

/// Same precedence rules as `load_skills`, but for agent definitions.
pub fn load_agents(project_root: &Path) -> Vec<AgentDef> {
    tracing::info!(target: "jfc::agents", project_root = %project_root.display(), "loading agents");
    let mut out: Vec<AgentDef> = Vec::new();
    for root in agent_roots(project_root) {
        let dir = root.path;
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
            let Some(mut agent) = parse_agent(&path, &raw) else {
                continue;
            };
            if let Some(namespace) = &root.namespace
                && !agent.name.contains(':')
            {
                agent.name = format!("{namespace}:{}", agent.name);
            }
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
    // Read-only catalogue shared by Explore / Plan / verification.
    // Includes the native graph + code-index tools so subagents can use
    // the pre-built code graph instead of grep-looping through the tree
    // (without these, the subagent's tool filter in
    // `jfc/src/tools/subagent.rs::filter_tools_for_agent` drops the
    // graph tools from the advertised catalogue and the model gets
    // "unknown tool" if it tries to call them).
    let read_only_tools = strs(&[
        "Read",
        "Glob",
        "Grep",
        "Bash",
        "code_index",
        "graph_query",
        "graph_context",
        "graph_search",
        "graph_callers",
        "graph_callees",
        "graph_impact",
        "graph_node",
        "graph_explore",
        "graph_status",
        "graph_files",
    ]);
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
            a.allowed_tools = read_only_tools;
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
            a.skills = strs(&["verification-findings"]);
            a.allowed_tools = strs(&[
                "Read",
                "Glob",
                "Grep",
                "Bash",
                "code_index",
                "graph_query",
                "graph_context",
                "graph_search",
                "graph_callers",
                "graph_callees",
                "graph_impact",
                "graph_node",
                "graph_explore",
                "graph_status",
                "graph_files",
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
                "code_index",
                "graph_query",
                "graph_context",
                "graph_search",
                "graph_callers",
                "graph_callees",
                "graph_impact",
                "graph_node",
                "graph_explore",
                "graph_status",
                "graph_files",
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
        Skill::new(
            name.to_owned(),
            PathBuf::from(format!("/x/skills/{name}.md")),
            None,
            body.to_owned(),
        )
    }

    fn skill(name: &str, description: Option<&str>) -> Skill {
        Skill::new(
            name.to_owned(),
            PathBuf::from("/x/skills/x.md"),
            description.map(str::to_owned),
            String::new(),
        )
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
    fn load_skills_includes_built_in_167_pack_normal() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_builtin_skill_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let skills = load_skills(&tmp);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        for needed in [
            "catch-up",
            "dream",
            "morning-checkin",
            "pre-meeting-checkin",
            "run",
            "verify",
            "run-skill-generator",
            "cowork-plugin",
            "design-sync",
            "simplify",
        ] {
            assert!(names.contains(&needed), "missing {needed} from {names:?}");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn project_skill_overrides_built_in_normal() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_builtin_override_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let verify_dir = tmp.join(".claude/skills/verify");
        std::fs::create_dir_all(&verify_dir).unwrap();
        std::fs::write(
            verify_dir.join("SKILL.md"),
            "---\nname: verify\ndescription: project verify\n---\nproject-specific verify body",
        )
        .unwrap();

        let skills = load_skills(&tmp);
        let verify = skills.iter().find(|s| s.name == "verify").unwrap();
        assert_eq!(verify.description.as_deref(), Some("project verify"));
        assert!(verify.body.contains("project-specific"));
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
    fn load_skills_collects_package_files_normal() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_skill_pkg_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skill_dir = tmp.join(".agents/skills/run-app");
        std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\nname: run-app\n---\nbody").unwrap();
        std::fs::write(skill_dir.join("scripts/driver.mjs"), "console.log('ok')").unwrap();

        let skills = load_skills(&tmp);
        let s = skills.iter().find(|s| s.name == "run-app").unwrap();
        assert_eq!(s.files.len(), 1);
        assert_eq!(s.files[0].relative_path, "scripts/driver.mjs");
        assert!(s.files[0].path.ends_with("scripts/driver.mjs"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn plugin_skills_and_agents_load_with_namespace_normal() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_plugin_registry_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plugin_skill = tmp.join("plugins/sec/skills/audit");
        let plugin_agent = tmp.join("plugins/sec/agents");
        std::fs::create_dir_all(&plugin_skill).unwrap();
        std::fs::create_dir_all(&plugin_agent).unwrap();
        std::fs::write(plugin_skill.join("SKILL.md"), "---\nname: audit\n---\nbody").unwrap();
        std::fs::write(
            plugin_agent.join("reviewer.md"),
            "---\nname: reviewer\n---\nReview things.",
        )
        .unwrap();

        let skills = load_skills(&tmp);
        let agents = load_agents(&tmp);
        assert!(skills.iter().any(|skill| skill.name == "sec:audit"));
        assert!(agents.iter().any(|agent| agent.name == "sec:reviewer"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn enabled_plugins_false_disables_plugin_roots_normal() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_plugin_disabled_test_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let plugin_skill = tmp.join("plugins/sec/skills/audit");
        let plugin_agent = tmp.join("plugins/sec/agents");
        std::fs::create_dir_all(&plugin_skill).unwrap();
        std::fs::create_dir_all(&plugin_agent).unwrap();
        std::fs::create_dir_all(tmp.join(".claude")).unwrap();
        std::fs::write(plugin_skill.join("SKILL.md"), "---\nname: audit\n---\nbody").unwrap();
        std::fs::write(
            plugin_agent.join("reviewer.md"),
            "---\nname: reviewer\n---\nReview things.",
        )
        .unwrap();
        std::fs::write(
            tmp.join(".claude/settings.json"),
            r#"{ "enabledPlugins": { "sec@local": false } }"#,
        )
        .unwrap();

        let skills = load_skills(&tmp);
        let agents = load_agents(&tmp);
        assert!(!skills.iter().any(|skill| skill.name == "sec:audit"));
        assert!(!agents.iter().any(|agent| agent.name == "sec:reviewer"));
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
    fn render_skills_section_hides_internal_superpower_skills_robust() {
        let skills = vec![
            skill("vuln-researcher", Some("JS and vuln research")),
            skill(
                "superpowers:verification-before-completion",
                Some("internal"),
            ),
            Skill::new(
                "openai-docs".to_owned(),
                PathBuf::from("/home/me/.codex/skills/.system/openai-docs/SKILL.md"),
                Some("system skill".to_owned()),
                String::new(),
            ),
        ];
        let out = render_skills_section(&skills);
        assert!(out.contains("vuln-researcher"));
        assert!(!out.contains("superpowers:"));
        assert!(!out.contains("openai-docs"));
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
