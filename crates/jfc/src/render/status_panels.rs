use crate::app::FocusedUiPanel;
use crate::app::{UiPanelRefreshStatuses, UiPanelSnapshots, ui_panel_snapshot_key};
use jfc_plugin_sdk::{UiMutationScope, UiPanelDescriptor};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PluginPanelSection {
    pub(super) title: String,
    pub(super) rows: Vec<String>,
}

pub(super) fn info_sidebar_panel_sections(
    panels: &[UiPanelDescriptor],
    snapshots: &UiPanelSnapshots,
    refresh_status: &UiPanelRefreshStatuses,
    focused: Option<&FocusedUiPanel>,
) -> Vec<PluginPanelSection> {
    panel_sections_for_scope(
        panels,
        snapshots,
        refresh_status,
        UiMutationScope::InfoSidebar,
        focused,
    )
}

fn panel_sections_for_scope(
    panels: &[UiPanelDescriptor],
    snapshots: &UiPanelSnapshots,
    refresh_status: &UiPanelRefreshStatuses,
    scope: UiMutationScope,
    focused: Option<&FocusedUiPanel>,
) -> Vec<PluginPanelSection> {
    let mut scoped = panels
        .iter()
        .filter(|panel| panel.scope == scope)
        .collect::<Vec<_>>();
    scoped.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.as_str().cmp(right.plugin_id.as_str()))
            .then_with(|| left.id.cmp(&right.id))
    });
    scoped
        .into_iter()
        .map(|panel| panel_section(panel, snapshots, refresh_status, focused))
        .collect()
}

fn panel_section(
    panel: &UiPanelDescriptor,
    snapshots: &UiPanelSnapshots,
    refresh_status: &UiPanelRefreshStatuses,
    focused: Option<&FocusedUiPanel>,
) -> PluginPanelSection {
    let mut rows = panel_body_rows(panel, snapshots);
    if let Some(action_id) = &panel.runtime_action_id {
        rows.push(format!("action {action_id}"));
    }
    if let Some(label) = ui_panel_refresh_label(panel, refresh_status) {
        rows.push(label);
    }
    let marker = if is_focused_panel(panel, focused) {
        "> "
    } else {
        ""
    };
    PluginPanelSection {
        title: format!(
            "{marker}{} · {}:{}",
            panel.title,
            panel.plugin_id.as_str(),
            panel.id
        ),
        rows,
    }
}

fn panel_body_rows(panel: &UiPanelDescriptor, snapshots: &UiPanelSnapshots) -> Vec<String> {
    if let Some(snapshot) = snapshots.get(&ui_panel_snapshot_key(panel))
        && let Some(body) = &snapshot.body
    {
        return body_rows(body);
    }
    panel.body.as_deref().map(body_rows).unwrap_or_default()
}

fn body_rows(body: &str) -> Vec<String> {
    body.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect()
}

fn ui_panel_refresh_label(
    panel: &UiPanelDescriptor,
    refresh_status: &UiPanelRefreshStatuses,
) -> Option<String> {
    panel.refresh.as_ref()?;
    let status = refresh_status.get(&ui_panel_snapshot_key(panel));
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

fn relative_duration_label(elapsed: std::time::Duration) -> String {
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

fn is_focused_panel(panel: &UiPanelDescriptor, focused: Option<&FocusedUiPanel>) -> bool {
    focused.is_some_and(|focused| {
        focused.plugin_id.as_str() == panel.plugin_id.as_str()
            && focused.panel_id.as_str() == panel.id.as_str()
    })
}
