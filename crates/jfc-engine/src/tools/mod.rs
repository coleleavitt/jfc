mod bash;
mod bounty_learning;
mod bounty_tasks;
mod catalog;
mod code_navigation;
mod daemon;
mod defs;
mod descriptor_builtin_route_kind;
mod descriptor_builtin_routes;
mod descriptor_catalog;
#[cfg(test)]
mod descriptor_discovery_tests;
mod descriptor_external_routes;
#[cfg(test)]
mod descriptor_external_tests;
mod descriptor_filesystem_defs;
mod descriptor_filesystem_routes;
mod descriptor_process_bridge;
mod descriptor_router;
#[cfg(test)]
mod descriptor_router_tests;
mod descriptor_search_defs;
mod descriptor_shell_defs;
mod descriptor_shell_routes;
#[cfg(test)]
mod descriptor_shell_tests;
mod design;
mod discovery;
mod dispatch;
mod dispatch_heavy;
mod economy;
mod filesystem;
mod hcom;
mod learn;
mod lsp;
mod memory;
mod notebook;
mod notifications;
mod persistent_shell;
pub mod plans;
mod registry;
pub(crate) mod research;
mod safe_tools;
#[cfg(test)]
mod schema_tests;
mod scratchpad;
mod search;
mod ssrf;
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
// background-shell roster API (list/cancel/detach), surfaced in the TUI.
pub use bash::{
    BashTaskSnapshot, CancelOutcome, background_running_foreground_bash, bash_task_command,
    bash_task_is_running, cancel_bash_task, list_bash_tasks,
};
pub use descriptor_catalog::{
    ExternalToolDescriptorReload, register_discovered_plugin_tool_descriptors,
    register_external_tool_descriptors, reload_discovered_plugin_tool_descriptors,
};
pub use descriptor_router::builtin_tool_descriptors;
pub(crate) use descriptor_router::external_tool_policy;
pub use dispatch::{
    builtin_tool_runtime, execute_tool, execute_tool_with_runtime, execute_tool_with_runtime_id,
};

// plan↔task linkage hook, shared by the manual TaskDone path and the
// subagent parent_task_id completion path so both advance linked plans.
pub use dispatch::advance_linked_plans;

// tool definitions (for advertised tool list)
pub use catalog::progressive_tool_defs;
pub(crate) use code_navigation::is_code_navigation_tool_name;
pub use defs::{
    all_tool_defs, is_model_hidden_builtin_tool_name, model_tool_defs, sync_tool_definitions_to_db,
};
pub use discovery::all_tool_defs_with_mcp;
pub use hcom::{
    hcom_available, is_hcom_tool_name, system_prompt_section as hcom_system_prompt_section,
};

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
pub use economy::{market_report_string, market_report_string_for_cwd};
// Used by the test suite (tools/tests.rs is #[path]-included into the test
// module below), not by the non-test build — hence the cfg guard.

// daemon
pub use daemon::execute_schedule_wakeup;

// subagent
pub use subagent::{
    build_parent_context_seed, execute_task, selected_subagent_model,
    selected_subagent_provider_model,
};

// tasks / skills

// swarm
pub use swarm::CURRENT_AGENT_NAME;

// registry
pub use registry::{
    agent_registry, pop_undo_entry, push_undo_entry, register_active_provider,
    register_event_sender, register_mcp_registry, register_provider_registry, restore_undo_entry,
};
pub use registry::{
    snapshot_active_provider, snapshot_event_sender, snapshot_mcp_registry,
    snapshot_provider_registry,
};

// slop guard sentinel
pub use safe_tools::SLOP_GUARD_MARKER;
