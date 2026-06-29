use jfc_plugin_host::{
    PluginDescriptorIssue, PluginDescriptorIssueActionability, PluginDescriptorIssueKind,
    PluginDescriptorIssueSeverity, PluginDescriptorKind, PluginDescriptorTargetKind,
    PluginHealthSummary, PluginReloadReport,
};
use jfc_plugin_sdk::{
    ExtensionSlot, MetricDescriptor, MetricSurface, MetricUnit, RuntimeActionDescriptor,
    RuntimeActionKind, UiSlotDescriptor,
};
use std::collections::HashMap;

pub(super) fn has_status_line_slot(slots: &[UiSlotDescriptor], id: &str) -> bool {
    slots
        .iter()
        .any(|slot| slot.slot == ExtensionSlot::StatusLine && slot.id == id)
}

pub(super) fn has_metric_surface(
    metrics: &[MetricDescriptor],
    id: &str,
    surface: MetricSurface,
) -> bool {
    metrics
        .iter()
        .any(|metric| metric.id == id && metric.surfaces.contains(&surface))
}

pub(super) fn cache_hit_badge(
    metrics: &[MetricDescriptor],
    usage_by_model: &HashMap<String, jfc_core::ModelUsage>,
) -> Option<String> {
    if !has_metric_surface(
        metrics,
        jfc_plugin_host::BUILTIN_CACHE_HIT_METRIC_ID,
        MetricSurface::StatusLine,
    ) {
        return None;
    }
    Some(format!(
        "cache {:.0}%",
        cache_hit_pct(usage_by_model)?.round()
    ))
}

pub(super) fn plugin_health_badge(summary: &PluginHealthSummary) -> Option<String> {
    if summary.total == 0 {
        return None;
    }
    if summary.failed > 0 {
        return Some(format!("plugins {} failed", summary.failed));
    }
    if summary.error_count > 0 {
        return Some(format!("plugins {} errors", summary.error_count));
    }
    if summary.disabled > 0 {
        return Some(format!("plugins {}/{} on", summary.active, summary.total));
    }
    if summary.registered > 0 {
        return Some(format!("plugins {} pending", summary.registered));
    }
    Some(format!("plugins {} ok", summary.active))
}

pub(super) const fn plugin_health_is_alert(summary: &PluginHealthSummary) -> bool {
    summary.failed > 0 || summary.error_count > 0
}

pub(super) const fn plugin_health_is_warning(summary: &PluginHealthSummary) -> bool {
    summary.disabled > 0 || summary.registered > 0
}

pub(super) fn plugin_detail_health<'a>(
    summary: &'a PluginHealthSummary,
    reload_report: Option<&'a PluginReloadReport>,
) -> &'a PluginHealthSummary {
    reload_report
        .map(|report| &report.diagnostics.health)
        .filter(|health| health.total > 0)
        .unwrap_or(summary)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PluginDetailRowTone {
    Health,
    Muted,
    Error,
    Warning,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PluginDetailRow {
    pub(super) text: String,
    pub(super) tone: PluginDetailRowTone,
}

impl PluginDetailRow {
    fn health(text: String) -> Self {
        Self {
            text,
            tone: PluginDetailRowTone::Health,
        }
    }

    fn muted(text: String) -> Self {
        Self {
            text,
            tone: PluginDetailRowTone::Muted,
        }
    }

    fn issue(issue: &PluginDescriptorIssue) -> Self {
        Self {
            text: descriptor_issue_detail_row(issue),
            tone: descriptor_issue_tone(issue.severity),
        }
    }
}

pub(super) fn plugin_health_detail_render_rows(
    summary: &PluginHealthSummary,
    reload_report: Option<&PluginReloadReport>,
    runtime_actions: &[RuntimeActionDescriptor],
) -> Vec<PluginDetailRow> {
    let health = plugin_detail_health(summary, reload_report);
    if health.total == 0 {
        return Vec::new();
    }
    let mut rows = vec![PluginDetailRow::health(format!(
        "{}/{} active",
        health.active, health.total
    ))];
    if health.failed > 0 {
        rows.push(PluginDetailRow::muted(format!("{} failed", health.failed)));
    }
    if health.error_count > 0 {
        rows.push(PluginDetailRow::muted(format!(
            "{} errors",
            health.error_count
        )));
    }
    if health.disabled > 0 {
        rows.push(PluginDetailRow::muted(format!(
            "{} disabled",
            health.disabled
        )));
    }
    if health.registered > 0 {
        rows.push(PluginDetailRow::muted(format!(
            "{} pending activation",
            health.registered
        )));
    }
    if let Some(report) = reload_report {
        rows.push(PluginDetailRow::muted(match report.changed {
            Some(true) => "descriptors changed".to_owned(),
            Some(false) => "descriptors unchanged".to_owned(),
            None => "descriptors fresh".to_owned(),
        }));
        rows.push(PluginDetailRow::muted(format!(
            "digest {}",
            report.diagnostics.descriptor_digest.as_str()
        )));
        rows.push(PluginDetailRow::muted(format!(
            "tools {} · resources {} · ui slots {} · actions {} · runtime ext {} · agent launches {}",
            report.diagnostics.counts.tools,
            report.diagnostics.counts.resources,
            report.diagnostics.counts.ui_slots,
            report.diagnostics.counts.runtime_actions,
            report.diagnostics.counts.runtime_extensions,
            report.diagnostics.counts.agent_launches
        )));
        if report.diagnostics.counts.ui_widgets > 0 {
            rows.push(PluginDetailRow::muted(format!(
                "widgets {}",
                report.diagnostics.counts.ui_widgets
            )));
        }
        if report.diagnostics.counts.ui_panels > 0 {
            rows.push(PluginDetailRow::muted(format!(
                "panels {}",
                report.diagnostics.counts.ui_panels
            )));
        }
        if report.diagnostics.counts.metrics > 0 {
            rows.push(PluginDetailRow::muted(format!(
                "metrics {}",
                report.diagnostics.counts.metrics
            )));
        }
        if !report.diagnostics.descriptor_issues.is_empty() {
            rows.push(PluginDetailRow::muted(format!(
                "descriptor issues {}",
                report.diagnostics.descriptor_issues.len()
            )));
            for issue in &report.diagnostics.descriptor_issues {
                rows.push(PluginDetailRow::issue(issue));
                rows.push(PluginDetailRow::muted(format!("fix {}", issue.repair_hint)));
            }
        }
    }
    push_plugin_diagnostics_rows(&mut rows, runtime_actions);
    rows
}

fn push_plugin_diagnostics_rows(
    rows: &mut Vec<PluginDetailRow>,
    runtime_actions: &[RuntimeActionDescriptor],
) {
    let Some(action) = runtime_actions
        .iter()
        .find(|action| action.kind == RuntimeActionKind::PluginDiagnostics)
    else {
        return;
    };
    rows.push(PluginDetailRow::muted(format!(
        "diagnostics action {}",
        action.label
    )));
    let smoke_checks = runtime_actions
        .iter()
        .filter(|action| action.kind == RuntimeActionKind::PluginSmoke)
        .count();
    if smoke_checks > 0 {
        rows.push(PluginDetailRow::muted(format!(
            "diagnostics smoke checks {smoke_checks}"
        )));
    }
}

fn descriptor_issue_detail_row(issue: &PluginDescriptorIssue) -> String {
    format!(
        "{} {} {}: {}:{}:{} -> {}:{}:{}",
        issue_severity_label(issue.severity),
        issue_actionability_label(issue.actionability),
        issue_kind_label(issue.kind),
        descriptor_kind_label(issue.descriptor_kind),
        issue.plugin_id.as_str(),
        issue.descriptor_id.as_str(),
        target_kind_label(issue.target_kind),
        issue.target_plugin_id.as_str(),
        issue.target_id.as_str()
    )
}

const fn descriptor_issue_tone(severity: PluginDescriptorIssueSeverity) -> PluginDetailRowTone {
    match severity {
        PluginDescriptorIssueSeverity::Error => PluginDetailRowTone::Error,
        PluginDescriptorIssueSeverity::Warning => PluginDetailRowTone::Warning,
    }
}

const fn issue_kind_label(kind: PluginDescriptorIssueKind) -> &'static str {
    match kind {
        PluginDescriptorIssueKind::MissingRuntimeAction => "missing_runtime_action",
        PluginDescriptorIssueKind::MissingUiPanel => "missing_ui_panel",
        PluginDescriptorIssueKind::MissingUiWidget => "missing_ui_widget",
    }
}

const fn issue_severity_label(severity: PluginDescriptorIssueSeverity) -> &'static str {
    match severity {
        PluginDescriptorIssueSeverity::Error => "error",
        PluginDescriptorIssueSeverity::Warning => "warning",
    }
}

const fn issue_actionability_label(
    actionability: PluginDescriptorIssueActionability,
) -> &'static str {
    match actionability {
        PluginDescriptorIssueActionability::AddRuntimeAction => "add_runtime_action",
        PluginDescriptorIssueActionability::AddUiPanel => "add_ui_panel",
        PluginDescriptorIssueActionability::AddUiWidget => "add_ui_widget",
        PluginDescriptorIssueActionability::FixReference => "fix_reference",
    }
}

const fn descriptor_kind_label(kind: PluginDescriptorKind) -> &'static str {
    match kind {
        PluginDescriptorKind::RuntimeAction => "runtime_action",
        PluginDescriptorKind::UiPanel => "ui_panel",
        PluginDescriptorKind::UiSlot => "ui_slot",
        PluginDescriptorKind::UiWidget => "ui_widget",
    }
}

const fn target_kind_label(kind: PluginDescriptorTargetKind) -> &'static str {
    match kind {
        PluginDescriptorTargetKind::RuntimeAction => "runtime_action",
        PluginDescriptorTargetKind::UiPanel => "ui_panel",
        PluginDescriptorTargetKind::UiWidget => "ui_widget",
    }
}

pub(super) fn cache_metric_rows(
    metrics: &[MetricDescriptor],
    usage_by_model: &HashMap<String, jfc_core::ModelUsage>,
    reload_report: Option<&PluginReloadReport>,
) -> Vec<String> {
    let mut rows = Vec::new();
    if has_metric_surface(
        metrics,
        jfc_plugin_host::BUILTIN_CACHE_HIT_METRIC_ID,
        MetricSurface::Sidebar,
    ) && let Some(hit_pct) = cache_hit_pct(usage_by_model)
    {
        let total_cache_read = total_cache_read_tokens(usage_by_model);
        let total_input = total_input_tokens(usage_by_model);
        rows.push(format!(
            "cache hit {:.0}% ({} read / {} input)",
            hit_pct.round(),
            super::fmt_number(total_cache_read),
            super::fmt_number(total_input)
        ));
    }
    if has_metric_surface(
        metrics,
        jfc_plugin_host::BUILTIN_CACHE_DIGEST_METRIC_ID,
        MetricSurface::Sidebar,
    ) && let Some(report) = reload_report
    {
        rows.push(match report.changed {
            Some(true) => "descriptors changed".to_owned(),
            Some(false) => "descriptors unchanged".to_owned(),
            None => "descriptors fresh".to_owned(),
        });
        rows.push(format!(
            "descriptor digest {}",
            report.diagnostics.descriptor_digest
        ));
    }
    rows
}

pub(super) fn rsi_metric_rows(
    metrics: &[MetricDescriptor],
    metadata: Option<&jfc_engine::runtime::StreamRequestMetadata>,
) -> Vec<String> {
    let Some(metadata) = metadata else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    if has_metric_surface(
        metrics,
        jfc_plugin_host::BUILTIN_RSI_PROMPT_SECTIONS_METRIC_ID,
        MetricSurface::Sidebar,
    ) {
        rows.push(format!(
            "rsi prompt sections {}",
            metadata.rsi_prompt_sections
        ));
    }
    if has_metric_surface(
        metrics,
        jfc_plugin_host::BUILTIN_RSI_TOOL_VISIBILITY_METRIC_ID,
        MetricSurface::Sidebar,
    ) {
        rows.push(format!(
            "rsi tool visibility rules {}",
            metadata.rsi_tool_visibility_rules
        ));
    }
    rows
}

pub(super) fn metric_panel_descriptor_rows(metrics: &[MetricDescriptor]) -> Vec<String> {
    let mut panel_metrics = metrics
        .iter()
        .filter(|metric| metric.surfaces.contains(&MetricSurface::Panel))
        .collect::<Vec<_>>();
    panel_metrics.sort_by(|left, right| {
        right
            .priority
            .cmp(&left.priority)
            .then_with(|| left.plugin_id.as_str().cmp(right.plugin_id.as_str()))
            .then_with(|| left.id.cmp(&right.id))
    });
    panel_metrics
        .into_iter()
        .map(metric_descriptor_row)
        .collect()
}

pub(super) fn metric_descriptor_row(metric: &MetricDescriptor) -> String {
    format!(
        "{} · {} · {}:{}",
        metric.label,
        metric_unit_label(metric.unit),
        metric.plugin_id.as_str(),
        metric.id
    )
}

fn metric_unit_label(unit: MetricUnit) -> &'static str {
    match unit {
        MetricUnit::Count => "count",
        MetricUnit::Percent => "percent",
        MetricUnit::Tokens => "tokens",
        MetricUnit::Usd => "usd",
        MetricUnit::Digest => "digest",
        MetricUnit::Text => "text",
    }
}

fn cache_hit_pct(usage_by_model: &HashMap<String, jfc_core::ModelUsage>) -> Option<f64> {
    let total_cache_read = total_cache_read_tokens(usage_by_model);
    let total_input = total_input_tokens(usage_by_model);
    (total_cache_read > 0 && total_input > 0)
        .then(|| (total_cache_read as f64 / total_input as f64 * 100.0).min(100.0))
}

fn total_cache_read_tokens(usage_by_model: &HashMap<String, jfc_core::ModelUsage>) -> u64 {
    usage_by_model
        .values()
        .map(|usage| usage.cache_read_tokens)
        .sum()
}

fn total_input_tokens(usage_by_model: &HashMap<String, jfc_core::ModelUsage>) -> u64 {
    usage_by_model
        .values()
        .map(|usage| usage.input_tokens)
        .sum()
}
