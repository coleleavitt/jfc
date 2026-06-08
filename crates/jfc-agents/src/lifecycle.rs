//! Agent prompt-construction helpers: rendering the skills section, the
//! auto-dispatch section, and building an agent's full system prompt by
//! splicing in resolved skill bodies.

use crate::registry::find_skill_by_name;
use crate::state::{Skill, SkillRenderContext, render_skill_invocation};
use jfc_core::AgentDef;

/// Render the loaded skills as a Markdown listing for injection into the
/// system prompt. The model needs to know skills exist before it can ask to
/// invoke them — this is the discovery surface.
///
/// Description is capped at 200 chars (with `…` ellipsis on overflow) to
/// keep per-turn token cost low — we re-inject on every stream call so
/// every char compounds.
///
/// Returns `""` when `skills` is empty so callers can unconditionally
/// `push_str` the result.
pub fn render_skills_section(skills: &[Skill]) -> String {
    let visible: Vec<&Skill> = skills
        .iter()
        .filter(|skill| prompt_visible_skill(skill))
        .collect();
    if visible.is_empty() {
        return String::new();
    }
    const MAX_DESC_CHARS: usize = 200;
    let mut out = String::from("\n\n## Available skills\n\n");
    for skill in visible {
        let mut tags = Vec::new();
        if skill.context.is_fork() {
            tags.push("fork".to_owned());
        }
        if !skill.files.is_empty() {
            tags.push(format!("{} files", skill.files.len()));
        }
        if let Some(schedule) = skill.schedule.as_deref().filter(|s| !s.trim().is_empty()) {
            tags.push(format!("schedule {schedule}"));
        }
        let tag_text = if tags.is_empty() {
            String::new()
        } else {
            format!(" [{}]", tags.join(", "))
        };
        match &skill.description {
            Some(desc) if !desc.is_empty() => {
                let trimmed: String = if desc.chars().count() > MAX_DESC_CHARS {
                    let mut s: String = desc.chars().take(MAX_DESC_CHARS).collect();
                    s.push('…');
                    s
                } else {
                    desc.clone()
                };
                out.push_str(&format!("- `{}`{} — {}\n", skill.name, tag_text, trimmed));
            }
            _ => {
                out.push_str(&format!("- `{}`{}\n", skill.name, tag_text));
            }
        }
    }
    out
}

fn prompt_visible_skill(skill: &Skill) -> bool {
    skill.is_discoverable()
}

/// Build the effective system prompt for an agent: its own `system_prompt`
/// followed by each resolved skill body, separated by `## Skill: <name>`
/// headers. Unknown skill names are skipped (with a `tracing::warn!`).
pub fn build_agent_system_prompt(agent: &AgentDef, all_skills: &[Skill]) -> String {
    build_agent_system_prompt_with_context(agent, all_skills, SkillRenderContext::default())
}

/// Build the effective system prompt for an agent with runtime placeholder
/// values available to any resolved skills.
pub fn build_agent_system_prompt_with_context(
    agent: &AgentDef,
    all_skills: &[Skill],
    context: SkillRenderContext<'_>,
) -> String {
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
                out.push_str(&render_skill_invocation(skill, context, None));
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

/// Render the auto-dispatch section that gets injected into the
/// leader's system prompt. Only renders when at least one agent has a
/// `key_trigger` populated. Returns `""` otherwise so callers can
/// unconditionally `push_str`.
pub fn render_dispatch_section(agents: &[AgentDef]) -> String {
    let triggers: Vec<&AgentDef> = agents.iter().filter(|a| a.key_trigger.is_some()).collect();
    if triggers.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n\n## Delegation — fire agents proactively\n\n\
         **Default Bias: DELEGATE.** Work yourself only when the task is \
         trivially small (one-line edit, single grep, single read of a known \
         file). Otherwise dispatch the matching specialist via the Task tool. \
         Mirrors v132's `subagent_type=Explore` nudge for any research that \
         would take more than 3 direct queries.\n\n\
         ### Key triggers (check BEFORE acting yourself)\n",
    );
    for a in &triggers {
        if let Some(t) = &a.key_trigger {
            out.push_str(&format!("- `{}` — {}\n", a.name, t));
        }
    }
    out.push_str("\n### Use vs avoid\n");
    for a in &triggers {
        if a.use_when.is_empty() && a.avoid_when.is_empty() {
            continue;
        }
        out.push_str(&format!("\n**`{}`**\n", a.name));
        if !a.use_when.is_empty() {
            out.push_str("  Use when:\n");
            for line in &a.use_when {
                out.push_str(&format!("  - {line}\n"));
            }
        }
        if !a.avoid_when.is_empty() {
            out.push_str("  Avoid when:\n");
            for line in &a.avoid_when {
                out.push_str(&format!("  - {line}\n"));
            }
        }
    }
    out.push_str(
        "\n### Delegation Trust Rule\n\
         Once you fire an agent for a question, do NOT manually grep / read \
         the same files yourself in parallel. Wait for the agent's result. \
         If you fire multiple agents, fire them in a single message via \
         multiple Task tool_use blocks (parallel dispatch) — never sequence \
         independent investigations.\n\n\
         ### Parallel fan-out\n\
         When a question has 2+ independent angles (e.g. \"how is X handled in \
         the frontend AND backend\", \"find every callsite of A, B, and C\", \
         \"audit the test coverage of these 5 modules\"), fan out **one Task \
         per angle in a single tool-use block**. Each agent runs concurrently \
         and returns to you in any order; the more independent the angles, \
         the higher the parallelism payoff. Cap at ~5 simultaneous agents per \
         turn so you can synthesize without losing track.\n\n\
         ### Result synthesis\n\
         After agents return, do not just paste their output. Synthesize:\n\
         - **Deduplicate**: same file mentioned twice → one entry, citing both agents.\n\
         - **Reconcile contradictions**: if agent A and agent B disagree, name \
           the conflict explicitly and either resolve it (re-read the source) or \
           flag it for the user.\n\
         - **Cite sources**: every claim should reference a `file_path:line_number` \
           the agent surfaced, not just \"the agent said so\".\n\
         - **Filter for relevance**: drop content that doesn't move the user's task \
           forward, even if the agent reported it.\n\n\
         ### Intent → dispatch routing (fast lookup)\n\
         | User says… | Default action |\n\
         | --- | --- |\n\
         | \"how does X work\" / \"explain Y\" / \"find Z\" | Fire `Explore` in background |\n\
         | \"plan the refactor\" / \"design Y\" / \"implement big-thing\" | Fire `Plan`, surface plan via ExitPlanMode |\n\
         | \"does this still work\" / \"run the tests\" / after a non-trivial edit | Fire `verification` in background |\n\
         | multi-angle audit (frontend+backend, N modules, N callers) | Fire N `Explore` agents in parallel, then synthesize |\n\
         | one-liner edit, exact-known file, single keyword grep | Use direct tools, no agent needed |\n",
    );
    out
}
