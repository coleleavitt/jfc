use std::path::Path;

use jfc_plugin_host::{
    PluginDiscoveryOptions, PluginDiscoverySearchRoot, PluginReloadReport,
    builtin_plugin_management_plugin_host, reload_cached_discovered_resource_plugin_state,
};
use jfc_plugin_sdk::{
    MetricDescriptor, ProviderDescriptor, RuntimeActionDescriptor, RuntimeExtensionDescriptor,
    ServiceDescriptor, ToolDescriptor, UiPanelDescriptor, UiWidgetDescriptor,
};

use super::descriptor_rows::{provider_rows, tool_rows};
use super::doctor_issue_rows::descriptor_issue_rows;
use super::doctor_rows::{metric_rows, service_rows, ui_panel_rows, ui_widget_rows};
use super::doctor_runtime_rows::{runtime_action_rows, runtime_extension_rows};
use super::store::plugins_root;

pub(super) fn plugin_doctor(previous_digest: Option<&str>) -> anyhow::Result<String> {
    let root = plugins_root()?;
    plugin_doctor_in(&root, previous_digest)
}

fn plugin_doctor_in(root: &Path, previous_digest: Option<&str>) -> anyhow::Result<String> {
    let state = reload_cached_discovered_resource_plugin_state(
        PluginDiscoveryOptions::new()
            .with_search_root(PluginDiscoverySearchRoot::global_plugins_dir(root)),
        previous_digest,
    )?;
    let services = plugin_management_services();
    let tools = state.host.tool_descriptors();
    let providers = state.host.provider_descriptors();
    let metrics = state.host.metric_descriptors();
    let ui_panels = state.host.ui_panel_descriptors();
    let ui_widgets = state.host.ui_widget_descriptors();
    let runtime_actions = state.host.runtime_action_descriptors();
    let runtime_extensions = state.host.runtime_extension_descriptors();
    Ok(render_plugin_doctor(
        root,
        &state.report,
        &services,
        &tools,
        &providers,
        &metrics,
        &ui_panels,
        &ui_widgets,
        &runtime_actions,
        &runtime_extensions,
    ))
}

fn plugin_management_services() -> Vec<ServiceDescriptor> {
    builtin_plugin_management_plugin_host().service_descriptors()
}

fn render_plugin_doctor(
    root: &Path,
    report: &PluginReloadReport,
    services: &[ServiceDescriptor],
    tools: &[ToolDescriptor],
    providers: &[ProviderDescriptor],
    metrics: &[MetricDescriptor],
    ui_panels: &[UiPanelDescriptor],
    ui_widgets: &[UiWidgetDescriptor],
    runtime_actions: &[RuntimeActionDescriptor],
    runtime_extensions: &[RuntimeExtensionDescriptor],
) -> String {
    let diagnostics = &report.diagnostics;
    let counts = &diagnostics.counts;
    let mut out = String::new();
    out.push_str(&format!("plugins: {}\n", root.display()));
    out.push_str(&format!("reload: {}\n", reload_status(report)));
    out.push_str(&format!(
        "descriptor_digest: {}\n",
        diagnostics.descriptor_digest
    ));
    if let Some(previous) = &report.previous_descriptor_digest {
        out.push_str(&format!("previous_descriptor_digest: {previous}\n"));
    }
    out.push_str(&format!(
        "health: total={} active={} disabled={} failed={} errors={}\n",
        diagnostics.health.total,
        diagnostics.health.active,
        diagnostics.health.disabled,
        diagnostics.health.failed,
        diagnostics.health.error_count
    ));
    out.push_str(&format!(
        "descriptors: resources={} commands={} tools={} providers={} services={} ui_slots={} ui_panels={} ui_widgets={} runtime_actions={} runtime_extensions={} agent_launches={} metrics={} hooks={}\n",
        counts.resources,
        counts.commands,
        counts.tools,
        counts.providers,
        counts.services,
        counts.ui_slots,
        counts.ui_panels,
        counts.ui_widgets,
        counts.runtime_actions,
        counts.runtime_extensions,
        counts.agent_launches,
        counts.metrics,
        counts.hooks
    ));
    if !services.is_empty() {
        out.push_str("services:\n");
        for row in service_rows(services) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if !tools.is_empty() {
        out.push_str("tools:\n");
        for row in tool_rows(tools) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if !providers.is_empty() {
        out.push_str("providers:\n");
        for row in provider_rows(providers) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if !metrics.is_empty() {
        out.push_str("metrics:\n");
        for row in metric_rows(metrics) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if !ui_panels.is_empty() {
        out.push_str("ui_panels:\n");
        for row in ui_panel_rows(ui_panels) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if !ui_widgets.is_empty() {
        out.push_str("ui_widgets:\n");
        for row in ui_widget_rows(ui_widgets) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if !runtime_actions.is_empty() {
        out.push_str("runtime_actions:\n");
        for row in runtime_action_rows(runtime_actions) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if !runtime_extensions.is_empty() {
        out.push_str("runtime_extensions:\n");
        for row in runtime_extension_rows(runtime_extensions) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if !diagnostics.descriptor_issues.is_empty() {
        out.push_str("descriptor_issues:\n");
        for row in descriptor_issue_rows(&diagnostics.descriptor_issues) {
            out.push_str(&format!("- {row}\n"));
        }
    }
    if diagnostics.active_plugins.is_empty() {
        out.push_str("active_plugins: (none)\n");
    } else {
        out.push_str("active_plugins:\n");
        for plugin in &diagnostics.active_plugins {
            out.push_str(&format!("- {}\n", plugin.as_str()));
        }
    }
    if !diagnostics.failed_plugins.is_empty() {
        out.push_str("failed_plugins:\n");
        for plugin in &diagnostics.failed_plugins {
            out.push_str(&format!("- {}\n", plugin.as_str()));
        }
    }
    out
}

fn reload_status(report: &PluginReloadReport) -> &'static str {
    match report.changed {
        Some(true) => "changed",
        Some(false) => "unchanged",
        None => "fresh",
    }
}

#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod diagnostics_tests;
