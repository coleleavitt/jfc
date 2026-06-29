use jfc_provider::ModelId;

use crate::runtime::StreamRequestOverrides;

#[derive(Clone, Copy)]
pub(super) struct BehavioralPromptState {
    pub(super) effective_brief_mode: bool,
    pub(super) pewter_owl_header: bool,
    pub(super) pewter_owl_tool: bool,
    pub(super) interaction_mode: crate::interaction_mode::InteractionMode,
}

pub(super) fn resolve_behavioral_prompt_state(
    overrides: &StreamRequestOverrides,
    model: &ModelId,
) -> BehavioralPromptState {
    let pewter_owl_header = crate::feature_gates::pewter_owl_header_enabled(model.as_str(), false);
    let pewter_owl_tool = crate::feature_gates::pewter_owl_tool_enabled(model.as_str(), false);
    let pewter_owl_brief = crate::feature_gates::pewter_owl_brief_enabled(model.as_str(), false);
    let effective_brief_mode = overrides.brief_mode || pewter_owl_brief;
    BehavioralPromptState {
        effective_brief_mode,
        pewter_owl_header,
        pewter_owl_tool,
        interaction_mode: overrides.interaction_mode,
    }
}
