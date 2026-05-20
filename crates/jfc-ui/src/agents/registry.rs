//! Agent and skill registry: filesystem loaders, built-in agent definitions,
//! and the `find_skill_by_name` lookup helper.

use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

use super::state::{Skill, parse_agent, parse_skill};
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
            // For directory-based skills, the inner file is named
            // `SKILL.md` so `parse_skill`'s `file_stem()` would return
            // "SKILL" — useless as a skill name. Override with the
            // directory name unless the SKILL frontmatter explicitly
            // set a `name:` (in which case parse_skill already used it
            // and we don't second-guess).
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

#[derive(Debug)]
struct SkillRoot {
    path: PathBuf,
    namespace: Option<String>,
}

#[derive(Debug)]
pub(super) struct SkillCandidate {
    pub md_path: PathBuf,
    pub fallback_name: String,
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
/// Used by the agent dispatcher to resolve `agent.skills` entries before
/// concatenating their bodies into the agent's system prompt, and by the
/// `Skill` tool / slash dispatcher to resolve a user-typed name.
pub fn find_skill_by_name<'a>(all_skills: &'a [Skill], name: &str) -> Option<&'a Skill> {
    // Case-insensitive — `/Explain` should hit the same skill as `/explain`.
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

/// Returns the built-in agent definitions that ship with jfc.
/// These mirror v126's built-in agents: general-purpose, Explore, Plan, verification.
pub fn built_in_agents() -> Vec<AgentDef> {
    vec![
        AgentDef {
            name: "general-purpose".into(),
            source: PathBuf::from("built-in"),
            model: None,
            isolation: None,
            skills: Vec::new(),
            allowed_tools: Vec::new(), // "*" — all tools
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
            key_trigger: Some("ambiguous / multi-step user request → general-purpose handles when no specialist fits".into()),
            use_when: vec![
                "request spans multiple unrelated concerns".into(),
                "user prompt doesn't match a more specific agent's domain".into(),
            ],
            avoid_when: vec![
                "the request is read-only exploration → fire Explore instead".into(),
                "the request is plan-only design → fire Plan instead".into(),
            ],
            cost: Some(AgentCost::Expensive),
            system_prompt: "You are an agent for Claude Code. Given the user's message, you should use the tools available to complete the task. Complete the task fully—don't gold-plate, but don't leave it half-done.\n\nYour strengths:\n- Searching for code, configurations, and patterns across large codebases\n- Analyzing multiple files to understand system architecture\n- Investigating complex questions that require exploring many files\n- Performing multi-step research tasks\n\nGuidelines:\n- For file searches: search broadly when you don't know where something lives. Use Read when you know the specific file path.\n- For analysis: Start broad and narrow down. Use multiple search strategies if the first doesn't yield results.\n- Be thorough: Check multiple locations, consider different naming conventions, look for related files.\n- NEVER create files unless they're absolutely necessary for achieving your goal. ALWAYS prefer editing an existing file to creating a new one.\n- NEVER proactively create documentation files (*.md) or README files. Only create documentation files if explicitly requested.\n\nWhen you complete the task, respond with a concise report covering what was done and any key findings — the caller will relay this to the user, so it only needs the essentials.".into(),
        },
        AgentDef {
            name: "Explore".into(),
            source: PathBuf::from("built-in"),
            model: Some("haiku".into()),
            isolation: None,
            skills: Vec::new(),
            allowed_tools: vec![
                "Read".into(), "Glob".into(), "Grep".into(), "Bash".into(),
            ],
            disallowed_tools: vec![
                "Task".into(), "Edit".into(), "Write".into(), "ApplyPatch".into(),
            ],
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
            key_trigger: Some("broad codebase exploration / 2+ modules / unfamiliar structure → fire Explore in background".into()),
            use_when: vec![
                "user asks 'how does X work', 'find Y', 'where is Z', 'look into …'".into(),
                "request spans 2+ files or modules".into(),
                "you don't know exact file locations and the search would take >3 grep calls".into(),
                "multiple search angles would strengthen understanding".into(),
            ],
            avoid_when: vec![
                "you already know the exact file (Read directly)".into(),
                "single-keyword grep would suffice (Grep directly)".into(),
                "Explore is already running on the same question (Delegation Trust Rule)".into(),
            ],
            cost: Some(AgentCost::Cheap),
            system_prompt: "You are a file search specialist. You excel at thoroughly navigating and exploring codebases.\n\n=== CRITICAL: READ-ONLY MODE - NO FILE MODIFICATIONS ===\nThis is a READ-ONLY exploration task. You are STRICTLY PROHIBITED from:\n- Creating new files\n- Modifying existing files\n- Deleting files\n- Running ANY commands that change system state\n\nYour role is EXCLUSIVELY to search and analyze existing code.\n\nYour strengths:\n- Rapidly finding files using glob patterns\n- Searching code and text with powerful regex patterns\n- Reading and analyzing file contents\n\nGuidelines:\n- Use Glob for broad file pattern matching\n- Use Grep for searching file contents with regex\n- Use Read when you know the specific file path you need to read\n- Use Bash ONLY for read-only operations (ls, git status, git log, git diff, find, cat, head, tail)\n- Adapt your search approach based on the thoroughness level specified by the caller\n- Wherever possible spawn multiple parallel tool calls for grepping and reading files\n\nComplete the user's search request efficiently and report your findings clearly.".into(),
        },
        AgentDef {
            name: "Plan".into(),
            source: PathBuf::from("built-in"),
            model: None, // inherit
            isolation: None,
            skills: Vec::new(),
            allowed_tools: vec![
                "Read".into(), "Glob".into(), "Grep".into(), "Bash".into(),
            ],
            disallowed_tools: vec![
                "Task".into(), "Edit".into(), "Write".into(), "ApplyPatch".into(),
            ],
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
            key_trigger: Some("multi-step / risky / cross-cutting change → fire Plan before any destructive edit".into()),
            use_when: vec![
                "user asks 'how should I implement X', 'design Y', 'plan the Z refactor'".into(),
                "the change touches 3+ files / 2+ modules and you don't have a clear approach".into(),
                "the change is irreversible (schema migration, public API change, large refactor)".into(),
            ],
            avoid_when: vec![
                "the change is a one-liner with obvious scope".into(),
                "the user already gave a step-by-step plan".into(),
            ],
            cost: Some(AgentCost::Expensive),
            system_prompt: "You are a software architect and planning specialist. Your role is to explore the codebase and design implementation plans.\n\n=== CRITICAL: READ-ONLY MODE - NO FILE MODIFICATIONS ===\nThis is a READ-ONLY planning task. You are STRICTLY PROHIBITED from modifying files.\n\nYour Process:\n1. Understand Requirements: Focus on the requirements provided.\n2. Explore Thoroughly: Read files, find existing patterns, understand architecture, identify similar features.\n3. Design Solution: Create implementation approach, consider trade-offs.\n4. Detail the Plan: Step-by-step strategy, dependencies, potential challenges.\n\nGuidelines:\n- Use Glob, Grep, and Read to explore the codebase\n- Use Bash ONLY for read-only operations\n- NEVER modify, create, or delete files\n\nEnd your response with:\n### Critical Files for Implementation\nList 3-5 files most critical for implementing this plan.".into(),
        },
        AgentDef {
            name: "verification".into(),
            source: PathBuf::from("built-in"),
            model: None, // inherit
            isolation: None,
            skills: Vec::new(),
            // verification is the one read-only specialist that legitimately
            // needs the task lifecycle tools: when it's dispatched against a
            // queued todo it must be able to mark the task done (PASS) or
            // failed (FAIL). Explore/Plan stay strictly read-only — they
            // produce findings/plans, they don't own queue entries.
            allowed_tools: vec![
                "Read".into(), "Glob".into(), "Grep".into(), "Bash".into(),
                "TaskList".into(), "TaskGet".into(), "TaskUpdate".into(), "TaskDone".into(),
            ],
            disallowed_tools: vec![
                "Task".into(), "Edit".into(), "Write".into(), "ApplyPatch".into(),
            ],
            permission_mode: None,
            forks_parent_context: None,
            background: Some(true),
            color: Some("red".into()),
            effort: None,
            max_turns: None,
            max_input_tokens: None,
            memory: None,
            mcp_servers: Vec::new(),
            hooks: std::collections::HashMap::new(),
            key_trigger: Some("after every non-trivial edit → fire verification in background to actually run + test".into()),
            use_when: vec![
                "you just finished a feature, fix, or refactor and the user wants confidence".into(),
                "the change touches a runtime path (server / CLI / build pipeline)".into(),
                "tests exist and the user expects you to run them".into(),
            ],
            avoid_when: vec![
                "the change was a doc / comment edit only".into(),
                "the user asked you NOT to run tests this turn".into(),
            ],
            cost: Some(AgentCost::Cheap),
            system_prompt: "You are a verification specialist. Your job is not to confirm the implementation works — it's to try to break it.\n\n=== CRITICAL: DO NOT MODIFY THE PROJECT ===\nYou are STRICTLY PROHIBITED from creating, modifying, or deleting any files IN THE PROJECT DIRECTORY.\n\nYou MAY write ephemeral test scripts to /tmp via Bash when inline commands aren't sufficient.\n\n=== VERIFICATION STRATEGY ===\nAdapt based on what changed:\n- Frontend: Start dev server, curl endpoints, run frontend tests\n- Backend/API: Start server, curl/fetch endpoints, verify responses, test error handling\n- CLI: Run with representative inputs, verify stdout/stderr/exit codes\n- Bug fixes: Reproduce original bug, verify fix, run regression tests\n\n=== REQUIRED STEPS ===\n1. Read CLAUDE.md/README for build/test commands\n2. Run the build (broken build = automatic FAIL)\n3. Run the test suite (failing tests = automatic FAIL)\n4. Run linters/type-checkers if configured\n5. Check for regressions\n\n=== OUTPUT FORMAT ===\nEvery check must follow:\n```\n### Check: [what you're verifying]\n**Command run:** [exact command]\n**Output observed:** [actual output]\n**Result: PASS** (or FAIL with Expected vs Actual)\n```\n\nEnd with exactly: VERDICT: PASS or VERDICT: FAIL or VERDICT: PARTIAL".into(),
        },
        AgentDef {
            name: "orchestrator".into(),
            source: PathBuf::from("built-in"),
            model: None,
            isolation: None,
            skills: Vec::new(),
            allowed_tools: vec![
                "Read".into(), "Glob".into(), "Grep".into(), "Bash".into(),
                "TaskCreate".into(), "TaskList".into(), "TaskGet".into(),
                "TaskUpdate".into(), "TaskDone".into(), "TaskValidate".into(),
                "AskUserQuestion".into(),
                // The orchestrator builds plans first, then surfaces
                // them for authorization — that's literally what
                // EnterPlanMode/ExitPlanMode model. Letting the agent
                // call these closes the loop with the leader's
                // permission-mode state.
                "EnterPlanMode".into(), "ExitPlanMode".into(),
            ],
            disallowed_tools: vec![
                "Edit".into(), "Write".into(), "ApplyPatch".into(),
            ],
            permission_mode: None,
            forks_parent_context: None,
            background: None,
            color: Some("magenta".into()),
            effort: None,
            max_turns: Some(8),
            max_input_tokens: None,
            memory: None,
            mcp_servers: Vec::new(),
            hooks: std::collections::HashMap::new(),
            key_trigger: Some("vague multi-area request → fire orchestrator to decompose into N concrete subtasks before authorizing work".into()),
            use_when: vec![
                "user request is broad: 'fix all the auth bugs', 'modernize the build', 'audit security'".into(),
                "you can't tell what 'done' looks like without scoping".into(),
                "the work would touch >5 files across multiple subsystems".into(),
            ],
            avoid_when: vec![
                "user already gave a numbered plan".into(),
                "the request is concrete (Edit / Write / single bug fix)".into(),
                "Plan agent fits better — Plan designs the *how* for one task; orchestrator decomposes a wide request into many tasks".into(),
            ],
            cost: Some(AgentCost::Cheap),
            system_prompt: "You are an orchestrator. Your job is to decompose a vague, wide-scope user request into a numbered plan of concrete subtasks the leader can dispatch.\n\n=== READ-ONLY ===\nDo NOT edit code. Do NOT run destructive commands. Use Read / Grep / Glob / Bash (read-only) to scope the work, then output the plan.\n\n=== PERMISSION POSTURE ===\nCall `EnterPlanMode` at the start of your scoping work to make the read-only contract enforceable session-wide. When you've finished decomposing, call `ExitPlanMode` with the finalized plan as the body — that surfaces the plan to the user, transitions the session out of plan mode, and hands authorization back to the leader. Treat ExitPlanMode as your terminal action: do not continue executing after it.\n\n=== WORKFLOW ===\n1. **Scope**: enumerate the surface area touched. Use Glob + Grep to find every file/module/test the request implicates.\n2. **Cluster**: group findings into independent units of work (\"refactor auth middleware\", \"update auth tests\", \"migrate session storage\", etc.). Each unit should be assignable to a single agent run.\n3. **Sequence**: identify dependencies between units. Mark units that can run in parallel.\n4. **Estimate**: per-unit, predict roughly how many tool calls and which agents fit (`general-purpose` for code change, `Explore` for investigation, `verification` after each).\n5. **Surface for authorization**: output a numbered plan and STOP. The leader will decide which units to dispatch.\n\n=== OUTPUT FORMAT ===\n```\n## Plan: <one-line summary>\n\n### Surface scope\n- file/path:line — observation\n- file/path:line — observation\n\n### Subtasks\n1. **<title>** — <one-line scope>\n   - Files: ...\n   - Agent: <general-purpose | Plan | Explore | verification>\n   - Parallel-safe: yes/no\n   - Verification: <command to confirm done>\n2. **<title>** — ...\n\n### Dependency graph\n- 2 depends on 1\n- 3 and 4 are parallel\n```\n\nDo NOT proceed past the plan. The leader fires the actual work.".into(),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::lifecycle::{
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

    // Normal: `load_skills` against an empty root produces an empty list
    // (when ~/.claude/skills is also empty or missing). Doesn't panic.
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
        // Don't crash; we only assert it returns a Vec without panic.
        let _ = load_skills(&tmp);
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // Robust: directory-based skills resolve via `<dir>/SKILL.md` and use
    // the directory name as the skill name. Regression for the
    // user-visible "Unknown skill: do-178b" failure when the loader only
    // walked flat `.md` files.
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

    // Robust: project-level skill files override user-level ones with the
    // same name. We can't easily mock `~/.claude/skills`, but we can verify
    // the dedup happens with a single project file.
    #[test]
    fn load_skills_project_skill_loads_robust() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_skills_proj_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skills_dir = tmp.join(".claude/skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        std::fs::write(
            skills_dir.join("alpha.md"),
            "---\nname: alpha\ndescription: project alpha\n---\nbody",
        )
        .unwrap();
        let skills = load_skills(&tmp);
        let alpha = skills
            .iter()
            .find(|s| s.name == "alpha")
            .expect("project skill should be loaded");
        assert_eq!(alpha.description.as_deref(), Some("project alpha"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_skills_finds_codex_and_agents_roots_normal() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_skills_codex_agents_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let codex_skill = tmp.join(".codex/skills/codex-skill");
        let agents_skill = tmp.join(".agents/skills/agents-skill");
        std::fs::create_dir_all(&codex_skill).unwrap();
        std::fs::create_dir_all(&agents_skill).unwrap();
        std::fs::write(
            codex_skill.join("SKILL.md"),
            "---\ndescription: codex root\n---\ncodex body",
        )
        .unwrap();
        std::fs::write(
            agents_skill.join("SKILL.md"),
            "---\ndescription: agents root\n---\nagents body",
        )
        .unwrap();

        let skills = load_skills(&tmp);
        let names: Vec<&str> = skills.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"codex-skill"), "names: {names:?}");
        assert!(names.contains(&"agents-skill"), "names: {names:?}");
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn load_skills_namespaces_plugin_skills_normal() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_skills_plugin_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let skill_dir = tmp.join(".agents/plugins/github/skills/review-pr");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(skill_dir.join("SKILL.md"), "---\n---\nreview body").unwrap();

        let skills = load_skills(&tmp);
        assert!(
            skills.iter().any(|s| s.name == "github:review-pr"),
            "skills: {:?}",
            skills.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // Normal: `built_in_agents` ships at least the four canonical agents
    // every jfc session can discover.
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
        // The Explore agent ships with a haiku-pinned model and is
        // restricted to read-only tools.
        let explore = agents.iter().find(|a| a.name == "Explore").unwrap();
        assert_eq!(explore.model.as_deref(), Some("haiku"));
        assert!(explore.allowed_tools.iter().any(|t| t == "Read"));
        assert!(explore.disallowed_tools.iter().any(|t| t == "Edit"));
        assert!(!explore.system_prompt.is_empty());
    }

    // Normal: `load_agents` against an empty project root falls back to
    // built-in agents.
    #[test]
    fn load_agents_empty_root_returns_builtins_normal() {
        // Use a tempdir we know has no `.claude/agents` to ensure the
        // result == built-in set (modulo any user-level ~/.claude content).
        let tmp = std::env::temp_dir().join(format!(
            "jfc_agents_empty_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&tmp).unwrap();
        let agents = load_agents(&tmp);
        let names: Vec<&str> = agents.iter().map(|a| a.name.as_str()).collect();
        // Built-ins always show up.
        for needed in ["general-purpose", "Explore", "Plan", "verification"] {
            assert!(names.contains(&needed), "missing {needed}: {names:?}");
        }
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // Robust: a project-defined agent with the same name as a built-in
    // overrides the built-in (project precedence wins).
    #[test]
    fn load_agents_project_overrides_builtin_robust() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_agents_override_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let agents_dir = tmp.join(".claude/agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        // Override `Explore` with a non-haiku model — confirms the loader
        // treats the project file as authoritative.
        std::fs::write(
            agents_dir.join("Explore.md"),
            "---\nname: Explore\nmodel: opus\n---\nCustom explorer body.",
        )
        .unwrap();

        let agents = load_agents(&tmp);
        let explore = agents
            .iter()
            .find(|a| a.name == "Explore")
            .expect("Explore must be present after override");
        assert_eq!(
            explore.model.as_deref(),
            Some("opus"),
            "project file should override built-in Explore"
        );
        assert!(explore.system_prompt.contains("Custom explorer body"));
        // built-ins for other names still surface.
        assert!(agents.iter().any(|a| a.name == "Plan"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // Robust: a malformed agent file in the project directory is silently
    // skipped — the rest of the registry still loads.
    #[test]
    fn load_agents_skips_malformed_files_robust() {
        let tmp = std::env::temp_dir().join(format!(
            "jfc_agents_malformed_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let agents_dir = tmp.join(".claude/agents");
        std::fs::create_dir_all(&agents_dir).unwrap();
        // No frontmatter at all — `parse_agent` returns None.
        std::fs::write(agents_dir.join("broken.md"), "Just a body, no frontmatter").unwrap();
        // Frontmatter with bad YAML.
        std::fs::write(
            agents_dir.join("yaml-bad.md"),
            "---\nname: [unterminated\n---\nbody",
        )
        .unwrap();
        // A valid one mixed in.
        std::fs::write(
            agents_dir.join("ok.md"),
            "---\nname: ok-agent\n---\nValid body.",
        )
        .unwrap();
        // Non-md file should be ignored.
        std::fs::write(agents_dir.join("README.txt"), "ignored").unwrap();
        let agents = load_agents(&tmp);
        assert!(agents.iter().any(|a| a.name == "ok-agent"));
        assert!(!agents.iter().any(|a| a.name == "broken"));
        let _ = std::fs::remove_dir_all(&tmp);
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
            first_lines
                .iter()
                .any(|l| l.contains("## Available skills")),
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

    /// Normal: dispatch section is empty when no agent has a key_trigger.
    /// Existing user agents (no metadata) shouldn't create noise in the
    /// system prompt.
    #[test]
    fn render_dispatch_section_empty_when_no_triggers_normal() {
        let agents = vec![make_agent("plain", "system", vec![])];
        let out = render_dispatch_section(&agents);
        assert_eq!(out, "");
    }

    /// Normal: dispatch section lists every trigger-bearing agent with
    /// its keyTrigger line. Pin the v132-style "Default Bias: DELEGATE"
    /// language so future refactors can't accidentally weaken it.
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
        assert!(out.contains("broad codebase exploration"), "{out}");
        assert!(out.contains("how does X work"), "{out}");
        assert!(out.contains("Delegation Trust Rule"), "{out}");
        assert!(out.contains("Intent → dispatch routing"), "{out}");
    }

    /// Robust: built-in agents already have triggers populated, so a
    /// fresh `built_in_agents()` always yields a non-empty section.
    /// This test pins that contract — if someone removes a key_trigger
    /// the system prompt loses the dispatch nudge silently otherwise.
    #[test]
    fn built_in_agents_have_dispatch_triggers_robust() {
        let agents = built_in_agents();
        let with_triggers: Vec<&str> = agents
            .iter()
            .filter(|a| a.key_trigger.is_some())
            .map(|a| a.name.as_str())
            .collect();
        // All four built-ins should advertise auto-dispatch.
        for expected in ["general-purpose", "Explore", "Plan", "verification"] {
            assert!(
                with_triggers.contains(&expected),
                "{expected} must have a keyTrigger; got {with_triggers:?}"
            );
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
        let skills = vec![make_skill("b", "BBBB body."), make_skill("a", "AAAA body.")];
        let out = build_agent_system_prompt(&agent, &skills);
        let pos_a = out.find("AAAA body.").expect("a present");
        let pos_b = out.find("BBBB body.").expect("b present");
        assert!(pos_a < pos_b, "skill 'a' must appear before skill 'b'");
    }
}
