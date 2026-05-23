mod agents;
mod daemon;
mod economy;
mod filesystem;
mod graph;
mod interaction;
mod tasks;

use jfc_provider::ToolDef;

pub fn all_tool_defs() -> Vec<ToolDef> {
    let mut defs = Vec::with_capacity(64);
    defs.extend(filesystem::filesystem_tool_defs());
    defs.extend(tasks::task_tool_defs());
    defs.extend(agents::agent_tool_defs());
    defs.extend(graph::graph_tool_defs());
    defs.extend(economy::economy_tool_defs());
    defs.extend(interaction::interaction_tool_defs());
    defs.extend(daemon::daemon_tool_defs());
    defs
}
