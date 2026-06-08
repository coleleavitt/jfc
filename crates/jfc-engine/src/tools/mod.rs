mod bash;
mod catalog;
mod daemon;
mod defs;
mod design;
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
pub mod structured_output;
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
pub use bash::execute_bash_inner;
pub use dispatch::execute_tool;

// plan↔task linkage hook, shared by the manual TaskDone path and the
// subagent parent_task_id completion path so both advance linked plans.
pub use dispatch::advance_linked_plans;

// tool definitions (for advertised tool list)
pub use catalog::progressive_tool_defs;
pub use defs::all_tool_defs;
pub use safe_tools::all_tool_defs_with_mcp;

const PEWTER_OWL_SEND_USER_MESSAGE_PROMPT: &str = "Send a message the user will read verbatim. Use this for content they need to see exactly as written between tool calls — a generated code snippet, a specific value, a direct reply to something they asked mid-task. Don't use it for routine narration of what you're about to do, or for your final answer — normal text reaches them for those.\n\n`status`: 'normal' when replying to what they just asked; 'proactive' when you're surfacing something unprompted.";

pub fn apply_send_user_message_policy(
    tools: &mut Vec<jfc_provider::ToolDef>,
    brief_mode: bool,
    pewter_owl_tool: bool,
) {
    if !brief_mode && !pewter_owl_tool {
        tools.retain(|tool| tool.name != "SendUserMessage");
        return;
    }
    if pewter_owl_tool
        && !brief_mode
        && let Some(tool) = tools.iter_mut().find(|tool| tool.name == "SendUserMessage")
    {
        tool.description = PEWTER_OWL_SEND_USER_MESSAGE_PROMPT.to_owned();
    }
}

// economy
pub use economy::market_report_string;
// Used by the test suite (tools/tests.rs is #[path]-included into the test
// module below), not by the non-test build — hence the cfg guard.

// daemon
pub use daemon::execute_schedule_wakeup;

// subagent
pub use subagent::{execute_task, selected_subagent_model};

// tasks / skills

// swarm
pub use swarm::CURRENT_AGENT_NAME;

// registry
pub use registry::{
    pop_undo_entry, push_undo_entry, register_active_provider, register_event_sender,
    register_mcp_registry, restore_undo_entry,
};
pub use registry::{snapshot_active_provider, snapshot_event_sender, snapshot_mcp_registry};

// slop guard sentinel
pub use safe_tools::SLOP_GUARD_MARKER;
