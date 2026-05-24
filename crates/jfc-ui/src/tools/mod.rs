mod bash;
mod daemon;
mod defs;
mod dispatch;
mod dispatch_heavy;
mod economy;
mod filesystem;
mod learn;
mod lsp;
mod memory;
mod notebook;
mod notifications;
pub mod plans;
mod registry;
mod safe_tools;
mod scratchpad;
mod search;
pub(crate) mod structured_output;
mod subagent;
mod swarm;
mod tasks;
#[cfg(test)]
mod tests;
mod worktree;

// ---------------------------------------------------------------------------
// Re-exports: public API surface consumed by the rest of the crate
// ---------------------------------------------------------------------------

// runtime types (ExecutionResult, etc.)
pub use crate::runtime::{ExecutionResult, ToolProvenance, ToolSource};

// main dispatcher
pub use dispatch::execute_tool;

// tool definitions (for advertised tool list)
pub(crate) use defs::all_tool_defs;
pub use safe_tools::all_tool_defs_with_mcp;

// economy
pub(crate) use economy::market_report_string;
// Used by the test suite (tools/tests.rs is #[path]-included into the test
// module below), not by the non-test build — hence the cfg guard.
#[cfg(test)]
pub(crate) use economy::{EconomyAgentInvoker, EconomySwarmProvider, apply_winning_solution};

// subagent
pub(crate) use subagent::{execute_task, selected_subagent_model};

// tasks / skills
#[cfg(test)]
pub(crate) use tasks::execute_skill;

// swarm
pub(crate) use swarm::CURRENT_AGENT_NAME;

// registry
pub(crate) use registry::get_or_build_graph_session;
#[cfg(test)]
pub(crate) use registry::invalidate_graph_session_cache;
#[cfg(test)]
pub(crate) use registry::record_edited_file;
pub use registry::{
    graph_history_snapshot, pop_undo_entry, push_undo_entry, register_active_provider,
    register_event_sender, register_mcp_registry, render_pending_auto_context, restore_undo_entry,
};
pub(crate) use registry::{snapshot_active_provider, snapshot_event_sender, snapshot_mcp_registry};

// slop guard sentinel
pub(crate) use safe_tools::SLOP_GUARD_MARKER;
