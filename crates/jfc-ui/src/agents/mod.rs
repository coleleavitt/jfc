//! v126 agent + skill loaders.
//!
//! Submodules:
//! - `state`    — `Skill`, `AgentDef` re-exports, serde frontmatter structs,
//!                low-level `parse_*` / `split_frontmatter` helpers.
//! - `lifecycle` — prompt-construction helpers: `render_skills_section`,
//!                `render_dispatch_section`, `build_agent_system_prompt`.
//! - `registry` — filesystem loaders (`load_skills`, `load_agents`),
//!                `find_skill_by_name`, `built_in_agents`.

pub(crate) mod lifecycle;
pub(crate) mod registry;
pub(crate) mod state;

// Public items used via `crate::agents::` by callers outside this module.
pub(crate) use lifecycle::{
    build_agent_system_prompt, render_dispatch_section, render_skills_section,
};
pub use registry::{find_skill_by_name, load_agents, load_skills};
pub use state::AgentDef;
