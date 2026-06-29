mod advice;
mod discovery;
mod memory;
mod orchestration;
mod skills;

use jfc_provider::ToolDef;

pub fn agent_tool_defs() -> Vec<ToolDef> {
    let mut defs = Vec::with_capacity(16);
    defs.extend(skills::skill_invocation_tool_defs());
    defs.extend(discovery::discovery_tool_defs());
    defs.extend(orchestration::task_tool_defs());
    defs.extend(memory::memory_tool_defs());
    defs.extend(orchestration::team_tool_defs());
    defs.extend(advice::advice_tool_defs());
    defs.extend(skills::skill_authoring_tool_defs());
    defs
}
