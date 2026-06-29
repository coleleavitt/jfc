//! Agent and skill loading, parsing, and prompt construction.
//!
//! Extracted from `jfc/src/agents/` to allow independent reuse
//! by tooling, daemon workers, and test harnesses without pulling in
//! the full TUI dependency tree.

mod builtins;
pub mod evals;
mod lifecycle;
mod plugin_resources;
mod registry;
mod state;

// Public items used by consumers (jfc, jfc-tools, etc.)
pub use lifecycle::{
    build_agent_system_prompt, build_agent_system_prompt_with_context, render_dispatch_section,
    render_skills_section,
};
pub use registry::{
    SkillWriteError, built_in_agents, built_in_skills, find_skill_by_name, load_agents,
    load_skills, write_agent_skill,
};
pub use state::{
    Skill, SkillContext, SkillFile, SkillOrigin, SkillRenderContext, parse_agent, parse_skill,
    render_skill_invocation, split_frontmatter,
};

// Re-export core types for convenience
pub use jfc_core::{AgentCost, AgentDef, Effort, MemoryScope, PermissionMode};
