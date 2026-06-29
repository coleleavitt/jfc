use std::time::Duration;

use crate::app::{
    FocusedUiWidget, UiWidgetRefreshStatuses, UiWidgetSnapshots, ui_widget_snapshot_key,
};
use jfc_plugin_sdk::{UiMutationScope, UiWidgetDescriptor, UiWidgetKind};

pub(super) fn ui_widget_panel_rows(
    widgets: &[UiWidgetDescriptor],
    snapshots: &UiWidgetSnapshots,
    refresh_status: &UiWidgetRefreshStatuses,
    focused: Option<&FocusedUiWidget>,
) -> Vec<String> {
    ui_widget_rows_for_scope(
        widgets,
        snapshots,
        refresh_status,
        UiMutationScope::InfoSidebar,
        focused,
    )
}

pub(super) fn task_panel_widget_rows(
    widgets: &[UiWidgetDescriptor],
    snapshots: &UiWidgetSnapshots,
    refresh_status: &UiWidgetRefreshStatuses,
) -> Vec<String> {
    ui_widget_rows_for_scope(
        widgets,
        snapshots,
        refresh_status,
        UiMutationScope::TaskPanel,
        None,
    )
}

pub(super) fn session_sidebar_widget_rows(
    widgets: &[UiWidgetDescriptor],
    snapshots: &UiWidgetSnapshots,
    refresh_status: &UiWidgetRefreshStatuses,
) -> Vec<String> {
    ui_widget_rows_for_scope(
        widgets,
        snapshots,
        refresh_status,
        UiMutationScope::SessionSidebar,
        None,
    )
}

fn ui_widget_rows_for_scope(
    widgets: &[UiWidgetDescriptor],
    snapshots: &UiWidgetSnapshots,
    refresh_status: &UiWidgetRefreshStatuses,
    scope: UiMutationScope,
    focused: Option<&FocusedUiWidget>,
) -> Vec<String> {
    let mut panel_widgets = widgets
        .iter()
        .filter(|widget| widget.scope == scope)
        .collect::<Vec<_>>();
    panel_widgets.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.as_str().cmp(right.plugin_id.as_str()))
            .then_with(|| left.id.cmp(&right.id))
    });
    panel_widgets
        .into_iter()
        .map(|widget| ui_widget_row(widget, snapshots, refresh_status, focused))
        .collect()
}

fn ui_widget_row(
    widget: &UiWidgetDescriptor,
    snapshots: &UiWidgetSnapshots,
    refresh_status: &UiWidgetRefreshStatuses,
    focused: Option<&FocusedUiWidget>,
) -> String {
    let marker = if is_focused_widget(widget, focused) {
        "> "
    } else {
        ""
    };
    let mut parts = vec![
        widget.label.clone(),
        ui_widget_payload_label(widget, snapshots),
    ];
    if let Some(label) = ui_widget_refresh_label(widget, refresh_status) {
        parts.push(label);
    }
    parts.push(format!("{}:{}", widget.plugin_id.as_str(), widget.id));
    format!("{marker}{}", parts.join(" · "))
}

fn ui_widget_payload_label(widget: &UiWidgetDescriptor, snapshots: &UiWidgetSnapshots) -> String {
    if let Some(snapshot) = snapshots.get(&ui_widget_snapshot_key(widget))
        && let Some(body) = &snapshot.body
    {
        return body.clone();
    }
    if let Some(body) = &widget.body {
        return body.clone();
    }
    if let Some(action_id) = &widget.runtime_action_id {
        return format!("action {action_id}");
    }
    ui_widget_kind_label(widget.kind).to_owned()
}

fn ui_widget_refresh_label(
    widget: &UiWidgetDescriptor,
    refresh_status: &UiWidgetRefreshStatuses,
) -> Option<String> {
    widget.refresh.as_ref()?;
    let status = refresh_status.get(&ui_widget_snapshot_key(widget));
    if let Some(reason) = status.and_then(|status| status.last_skip_reason.as_ref()) {
        return Some(format!("refresh skipped {}", compact_status_text(reason)));
    }
    if let Some(error) = status.and_then(|status| status.last_error.as_ref()) {
        return Some(format!("refresh error {}", compact_status_text(error)));
    }
    if let Some(success_at) = status.and_then(|status| status.last_success_at) {
        return Some(format!(
            "refresh ok {}",
            relative_duration_label(success_at.elapsed())
        ));
    }
    if let Some(attempt_at) = status.and_then(|status| status.last_attempt_at) {
        return Some(format!(
            "refresh tried {}",
            relative_duration_label(attempt_at.elapsed())
        ));
    }
    Some("refreshable".to_owned())
}

fn compact_status_text(value: &str) -> String {
    let value = value.replace(['\n', '\r'], " ");
    if value.chars().count() <= 80 {
        return value;
    }
    let mut truncated = value.chars().take(77).collect::<String>();
    truncated.push_str("...");
    truncated
}

fn relative_duration_label(elapsed: Duration) -> String {
    let seconds = elapsed.as_secs();
    if seconds == 0 {
        return "just now".to_owned();
    }
    if seconds < 60 {
        return format!("{seconds}s ago");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes}m ago");
    }
    format!("{}h ago", minutes / 60)
}

fn is_focused_widget(widget: &UiWidgetDescriptor, focused: Option<&FocusedUiWidget>) -> bool {
    focused.is_some_and(|focused| {
        focused.plugin_id.as_str() == widget.plugin_id.as_str()
            && focused.widget_id.as_str() == widget.id.as_str()
    })
}

const fn ui_widget_kind_label(kind: UiWidgetKind) -> &'static str {
    match kind {
        UiWidgetKind::Text => "text",
        UiWidgetKind::Action => "action",
    }
}
