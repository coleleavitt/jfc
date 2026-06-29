use crate::app::App;
use crate::runtime::EngineEvent;
use jfc_plugin_sdk::{RuntimeActionDescriptor, RuntimeActionKind};
use tokio::sync::mpsc;

pub(super) async fn execute_plugin_diagnostics_action(
    app: &mut App,
    action: &RuntimeActionDescriptor,
    _tx: &mpsc::Sender<EngineEvent>,
) {
    let changed = app.reload_plugin_status_fresh();
    let descriptor_issues = app
        .plugins
        .reload_report
        .as_ref()
        .map(|report| report.diagnostics.descriptor_issues.len())
        .unwrap_or_default();
    let smoke_actions = app
        .plugins
        .runtime_action_descriptors
        .iter()
        .filter(|descriptor| descriptor.kind == RuntimeActionKind::PluginSmoke)
        .cloned()
        .collect::<Vec<_>>();
    push_diagnostics_toast(app, descriptor_issues, smoke_actions.len(), changed);
    tracing::info!(
        target: "jfc::palette",
        plugin = action.plugin_id.as_str(),
        action = action.id.as_str(),
        descriptor_issues,
        smoke_checks = smoke_actions.len(),
        changed,
        "plugin diagnostics runtime action refreshed descriptors"
    );
    for smoke_action in smoke_actions {
        super::runtime_action_smoke::execute_plugin_smoke_action(app, &smoke_action).await;
    }
}

fn push_diagnostics_toast(
    app: &mut App,
    descriptor_issues: usize,
    smoke_checks: usize,
    changed: bool,
) {
    let kind = if descriptor_issues > 0 {
        jfc_engine::toast::ToastKind::Warning
    } else {
        jfc_engine::toast::ToastKind::Success
    };
    let change_label = if changed { "changed" } else { "fresh" };
    let issue_label = descriptor_issue_label(descriptor_issues);
    let smoke_label = smoke_check_label(smoke_checks);
    let text = format!(
        "Plugin diagnostics {change_label}: {descriptor_issues} {issue_label}, {smoke_checks} {smoke_label}"
    );
    jfc_engine::toast::push_with_cap(
        &mut app.engine.toasts,
        jfc_engine::toast::Toast::new(kind, text),
    );
}

const fn descriptor_issue_label(count: usize) -> &'static str {
    if count == 1 {
        "descriptor issue"
    } else {
        "descriptor issues"
    }
}

const fn smoke_check_label(count: usize) -> &'static str {
    if count == 1 {
        "smoke check"
    } else {
        "smoke checks"
    }
}
