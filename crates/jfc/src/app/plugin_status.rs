use std::collections::HashSet;
use std::path::Path;
use std::time::Instant;

use jfc_plugin_host::{
    PluginHealthSummary, PluginReloadReport, cached_discovered_resource_plugin_state,
    reload_cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{
    MetricDescriptor, RuntimeActionDescriptor, RuntimeExtensionDescriptor, UiPanelDescriptor,
    UiSlotDescriptor, UiWidgetDescriptor,
};

use super::plugin_panel_state::append_ui_panel_descriptors;
use super::plugin_runtime_extension_state::{
    append_runtime_extension_descriptors, builtin_runtime_extension_descriptors,
};
use super::{UiPanelRefreshStatuses, UiPanelSnapshots, UiWidgetRefreshStatuses, UiWidgetSnapshots};

#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct PluginUiState {
    pub(crate) health: PluginHealthSummary,
    pub(crate) ui_slots: Vec<UiSlotDescriptor>,
    pub(crate) ui_panel_descriptors: Vec<UiPanelDescriptor>,
    pub(crate) ui_panel_snapshots: UiPanelSnapshots,
    pub(crate) ui_panel_refresh_status: UiPanelRefreshStatuses,
    pub(crate) ui_widget_descriptors: Vec<UiWidgetDescriptor>,
    pub(crate) ui_widget_snapshots: UiWidgetSnapshots,
    pub(crate) ui_widget_refresh_status: UiWidgetRefreshStatuses,
    pub(crate) metric_descriptors: Vec<MetricDescriptor>,
    pub(crate) runtime_action_descriptors: Vec<RuntimeActionDescriptor>,
    pub(crate) runtime_extension_descriptors: Vec<RuntimeExtensionDescriptor>,
    pub(crate) reload_report: Option<PluginReloadReport>,
    pub(crate) last_refresh_at: Option<Instant>,
}

pub(crate) fn initial_ui_state(project_root: &Path) -> PluginUiState {
    let mut state = builtin_ui_state();
    state.ui_panel_snapshots = super::plugin_panel_state::load_ui_panel_snapshots(project_root);
    state.ui_widget_snapshots = super::plugin_widget_state::load_ui_widget_snapshots(project_root);
    if let Some(discovered) = discovered_ui_state(project_root, None) {
        append_ui_slots(&mut state.ui_slots, discovered.ui_slots);
        append_ui_panel_descriptors(&mut state.ui_panel_descriptors, discovered.ui_panels);
        append_ui_widget_descriptors(
            &mut state.ui_widget_descriptors,
            discovered.ui_widget_descriptors,
        );
        append_metric_descriptors(&mut state.metric_descriptors, discovered.metrics);
        append_runtime_action_descriptors(
            &mut state.runtime_action_descriptors,
            discovered.runtime_actions,
        );
        append_runtime_extension_descriptors(
            &mut state.runtime_extension_descriptors,
            discovered.runtime_extensions,
        );
        state.reload_report = Some(discovered.report);
    }
    state
}

pub(crate) fn refresh_ui_state(
    project_root: &Path,
    previous_report: Option<&PluginReloadReport>,
) -> Option<PluginUiState> {
    let previous_digest =
        previous_report.map(|report| report.diagnostics.descriptor_digest.as_str());
    let discovered = discovered_ui_state(project_root, previous_digest)?;
    let mut state = builtin_ui_state();
    append_ui_slots(&mut state.ui_slots, discovered.ui_slots);
    append_ui_panel_descriptors(&mut state.ui_panel_descriptors, discovered.ui_panels);
    append_ui_widget_descriptors(
        &mut state.ui_widget_descriptors,
        discovered.ui_widget_descriptors,
    );
    append_metric_descriptors(&mut state.metric_descriptors, discovered.metrics);
    append_runtime_action_descriptors(
        &mut state.runtime_action_descriptors,
        discovered.runtime_actions,
    );
    append_runtime_extension_descriptors(
        &mut state.runtime_extension_descriptors,
        discovered.runtime_extensions,
    );
    state.reload_report = Some(discovered.report);
    Some(state)
}

fn builtin_ui_state() -> PluginUiState {
    match jfc_plugin_host::builtin_status_line_plugin_host() {
        Ok(host) => {
            let snapshot = host.status_snapshot();
            PluginUiState {
                health: snapshot.health_summary(),
                ui_slots: host.ui_slot_descriptors(),
                ui_panel_descriptors: host.ui_panel_descriptors(),
                ui_panel_snapshots: UiPanelSnapshots::default(),
                ui_panel_refresh_status: UiPanelRefreshStatuses::default(),
                ui_widget_descriptors: host.ui_widget_descriptors(),
                ui_widget_snapshots: UiWidgetSnapshots::default(),
                ui_widget_refresh_status: UiWidgetRefreshStatuses::default(),
                metric_descriptors: builtin_metric_descriptors(),
                runtime_action_descriptors: host.runtime_action_descriptors(),
                runtime_extension_descriptors: builtin_runtime_extension_descriptors(&host),
                reload_report: None,
                last_refresh_at: None,
            }
        }
        Err(error) => {
            tracing::warn!(
                target: "jfc::plugins",
                error = %error,
                "failed to activate built-in status-line plugin"
            );
            PluginUiState::default()
        }
    }
}

fn builtin_metric_descriptors() -> Vec<MetricDescriptor> {
    match jfc_plugin_host::builtin_observability_plugin_host() {
        Ok(host) => host.metric_descriptors(),
        Err(error) => {
            tracing::warn!(
                target: "jfc::plugins",
                error = %error,
                "failed to activate built-in observability plugin"
            );
            Vec::new()
        }
    }
}

struct DiscoveredUiState {
    report: PluginReloadReport,
    ui_slots: Vec<UiSlotDescriptor>,
    ui_panels: Vec<UiPanelDescriptor>,
    ui_widget_descriptors: Vec<UiWidgetDescriptor>,
    metrics: Vec<MetricDescriptor>,
    runtime_actions: Vec<RuntimeActionDescriptor>,
    runtime_extensions: Vec<RuntimeExtensionDescriptor>,
}

fn discovered_ui_state(
    project_root: &Path,
    previous_digest: Option<&str>,
) -> Option<DiscoveredUiState> {
    let options = jfc_engine::workflows::registry::plugin_discovery_options_for(project_root);
    let state = if previous_digest.is_some() {
        reload_cached_discovered_resource_plugin_state(options, previous_digest)
    } else {
        cached_discovered_resource_plugin_state(options)
    };
    match state {
        Ok(state) => Some(DiscoveredUiState {
            report: state.report.clone(),
            ui_slots: state.host.ui_slot_descriptors(),
            ui_panels: state.host.ui_panel_descriptors(),
            ui_widget_descriptors: state.host.ui_widget_descriptors(),
            metrics: state.host.metric_descriptors(),
            runtime_actions: state.host.runtime_action_descriptors(),
            runtime_extensions: state.host.runtime_extension_descriptors(),
        }),
        Err(error) => {
            tracing::warn!(
                target: "jfc::plugins",
                error = %error,
                "failed to reload discovered plugin descriptors"
            );
            None
        }
    }
}

fn append_ui_slots(slots: &mut Vec<UiSlotDescriptor>, extra: Vec<UiSlotDescriptor>) {
    let mut seen =
        slots
            .iter()
            .map(slot_key)
            .collect::<HashSet<(String, jfc_plugin_sdk::ExtensionSlot, String)>>();
    for slot in extra {
        if seen.insert(slot_key(&slot)) {
            slots.push(slot);
        }
    }
}

fn slot_key(slot: &UiSlotDescriptor) -> (String, jfc_plugin_sdk::ExtensionSlot, String) {
    (
        slot.plugin_id.as_str().to_owned(),
        slot.slot,
        slot.id.clone(),
    )
}

fn append_ui_widget_descriptors(
    widgets: &mut Vec<UiWidgetDescriptor>,
    extra: Vec<UiWidgetDescriptor>,
) {
    let mut seen =
        widgets
            .iter()
            .map(ui_widget_key)
            .collect::<HashSet<(String, jfc_plugin_sdk::UiMutationScope, String)>>();
    for widget in extra {
        if seen.insert(ui_widget_key(&widget)) {
            widgets.push(widget);
        }
    }
}

fn ui_widget_key(widget: &UiWidgetDescriptor) -> (String, jfc_plugin_sdk::UiMutationScope, String) {
    (
        widget.plugin_id.as_str().to_owned(),
        widget.scope,
        widget.id.clone(),
    )
}

fn append_metric_descriptors(metrics: &mut Vec<MetricDescriptor>, extra: Vec<MetricDescriptor>) {
    let mut seen = metrics
        .iter()
        .map(metric_key)
        .collect::<HashSet<(String, String)>>();
    for metric in extra {
        if seen.insert(metric_key(&metric)) {
            metrics.push(metric);
        }
    }
}

fn metric_key(metric: &MetricDescriptor) -> (String, String) {
    (metric.plugin_id.as_str().to_owned(), metric.id.clone())
}

fn append_runtime_action_descriptors(
    actions: &mut Vec<RuntimeActionDescriptor>,
    extra: Vec<RuntimeActionDescriptor>,
) {
    let mut seen = actions
        .iter()
        .map(runtime_action_key)
        .collect::<HashSet<(String, String)>>();
    for action in extra {
        if seen.insert(runtime_action_key(&action)) {
            actions.push(action);
        }
    }
}

fn runtime_action_key(action: &RuntimeActionDescriptor) -> (String, String) {
    (action.plugin_id.as_str().to_owned(), action.id.clone())
}
