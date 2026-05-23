//! Agent and skill loading, parsing, and prompt construction.
//!
//! Extracted from `jfc-ui/src/agents/` to allow independent reuse
//! by tooling, daemon workers, and test harnesses without pulling in
//! the full TUI dependency tree.

mod lifecycle;
mod registry;
mod state;

// Public items used by consumers (jfc-ui, jfc-tools, etc.)
pub use lifecycle::{build_agent_system_prompt, render_dispatch_section, render_skills_section};
pub use registry::{built_in_agents, find_skill_by_name, load_agents, load_skills};
pub use state::{Skill, parse_agent, parse_skill, split_frontmatter};

// Re-export core types for convenience
pub use jfc_core::{AgentCost, AgentDef, Effort, MemoryScope, PermissionMode};
