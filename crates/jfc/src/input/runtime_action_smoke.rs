use crate::app::App;
use jfc_plugin_sdk::RuntimeActionDescriptor;

pub(super) async fn execute_plugin_smoke_action(app: &mut App, action: &RuntimeActionDescriptor) {
    let plugin_name = plugin_smoke_target(action);
    match crate::plugin_smoke::smoke_plugin(&plugin_name).await {
        Ok(report) => {
            tracing::info!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                report,
                "plugin smoke runtime action passed"
            );
            push_plugin_smoke_toast(
                app,
                jfc_engine::toast::ToastKind::Success,
                &plugin_name,
                None,
            );
        }
        Err(error) => {
            tracing::warn!(
                target: "jfc::palette",
                plugin = action.plugin_id.as_str(),
                action = action.id.as_str(),
                error = %error,
                "plugin smoke runtime action failed"
            );
            push_plugin_smoke_toast(
                app,
                jfc_engine::toast::ToastKind::Error,
                &plugin_name,
                Some(error.to_string()),
            );
        }
    }
}

pub(super) fn plugin_smoke_target(action: &RuntimeActionDescriptor) -> String {
    action
        .plugin_smoke_target()
        .unwrap_or_else(|_| action.plugin_id.as_str())
        .to_owned()
}

fn push_plugin_smoke_toast(
    app: &mut App,
    kind: jfc_engine::toast::ToastKind,
    plugin_name: &str,
    error: Option<String>,
) {
    let text = match error {
        Some(error) => format!("Plugin smoke failed for {plugin_name}: {error}"),
        None => format!("Plugin smoke passed for {plugin_name}"),
    };
    jfc_engine::toast::push_with_cap(
        &mut app.engine.toasts,
        jfc_engine::toast::Toast::new(kind, text),
    );
}
