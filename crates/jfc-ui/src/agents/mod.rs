//! v126 agent + skill loaders — thin re-export from `jfc-agents` crate.

pub(crate) mod lifecycle {
    pub use jfc_agents::{
        build_agent_system_prompt, render_dispatch_section, render_skills_section,
    };
}

pub(crate) mod registry {
    pub use jfc_agents::{find_skill_by_name, load_agents, load_skills};
}

// Public items used via `crate::agents::` by callers outside this module.
pub use jfc_agents::AgentDef;
pub(crate) use lifecycle::{
    build_agent_system_prompt, render_dispatch_section, render_skills_section,
};
pub use registry::{find_skill_by_name, load_agents, load_skills};
