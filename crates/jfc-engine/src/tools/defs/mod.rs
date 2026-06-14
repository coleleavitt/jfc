mod agents;
mod daemon;
mod design;
mod economy;
mod filesystem;
mod interaction;
mod plan;
mod review;
mod tasks;

use jfc_provider::ToolDef;

pub fn all_tool_defs() -> Vec<ToolDef> {
    let mut defs = Vec::with_capacity(64);
    defs.extend(filesystem::filesystem_tool_defs());
    defs.extend(tasks::task_tool_defs());
    defs.extend(plan::plan_tool_defs());
    defs.extend(agents::agent_tool_defs());
    defs.extend(economy::economy_tool_defs());
    defs.extend(design::design_tool_defs());
    defs.extend(interaction::interaction_tool_defs());
    defs.extend(review::review_tool_defs());
    defs.extend(daemon::daemon_tool_defs());
    defs
}
