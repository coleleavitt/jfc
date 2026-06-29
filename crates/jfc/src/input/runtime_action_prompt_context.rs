use crate::app::App;
use jfc_plugin_sdk::RuntimeActionDescriptor;

pub(super) fn execute_refresh_prompt_context_action(
    app: &mut App,
    action: &RuntimeActionDescriptor,
) {
    let _ = app.reload_plugin_status_fresh();
    tracing::info!(
        target: "jfc::palette",
        plugin = action.plugin_id.as_str(),
        action = action.id.as_str(),
        "prompt-context runtime action refreshed plugin descriptors for next request"
    );
}
