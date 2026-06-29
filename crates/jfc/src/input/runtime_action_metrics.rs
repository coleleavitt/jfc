use crate::app::App;
use jfc_plugin_sdk::RuntimeActionDescriptor;

pub(super) fn execute_refresh_metrics_action(app: &mut App, _action: &RuntimeActionDescriptor) {
    let _ = app.reload_plugin_status_fresh();
}
