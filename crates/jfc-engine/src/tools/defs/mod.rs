mod agents;
mod daemon;
mod design;
mod economy;
mod filesystem;
mod hcom;
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
    defs.extend(hcom::hcom_tool_defs());
    defs.extend(interaction::interaction_tool_defs());
    defs.extend(review::review_tool_defs());
    defs.extend(daemon::daemon_tool_defs());
    defs
}

pub fn model_tool_defs() -> Vec<ToolDef> {
    all_tool_defs()
        .into_iter()
        .filter(|tool| !is_model_hidden_builtin_tool_name(&tool.name))
        .collect()
}

pub fn is_model_hidden_builtin_tool_name(name: &str) -> bool {
    name.eq_ignore_ascii_case("BashOutput")
}
