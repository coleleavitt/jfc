use jfc_plugin_sdk::{
    MetricDescriptor, MetricSurface, MetricUnit, ServiceDescriptor, UiMutationScope,
    UiPanelDescriptor, UiPanelRefreshKind, UiWidgetDescriptor, UiWidgetKind, UiWidgetRefreshKind,
};

pub(super) fn service_rows(services: &[ServiceDescriptor]) -> Vec<String> {
    let mut rows = services
        .iter()
        .map(|descriptor| {
            format!(
                "{} {} {} [{}]",
                descriptor.plugin_id.as_str(),
                descriptor.kind.as_str(),
                descriptor.namespace,
                descriptor.status.as_str()
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

pub(super) fn metric_rows(metrics: &[MetricDescriptor]) -> Vec<String> {
    let mut rows = metrics
        .iter()
        .map(|descriptor| {
            format!(
                "{} {} {} [{}; surfaces: {}; priority={}]",
                descriptor.plugin_id.as_str(),
                descriptor.id,
                descriptor.label,
                metric_unit_label(descriptor.unit),
                metric_surface_labels(&descriptor.surfaces),
                descriptor.priority
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

pub(super) fn ui_widget_rows(widgets: &[UiWidgetDescriptor]) -> Vec<String> {
    let mut rows = widgets
        .iter()
        .map(|descriptor| {
            let mut fields = vec![
                ui_widget_kind_label(descriptor.kind).to_owned(),
                format!("scope: {}", ui_mutation_scope_label(descriptor.scope)),
                format!("priority={}", descriptor.priority),
            ];
            fields.extend(ui_widget_refresh_fields(descriptor));
            format!(
                "{} {} {} [{}]",
                descriptor.plugin_id.as_str(),
                descriptor.id,
                descriptor.label,
                fields.join("; ")
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

pub(super) fn ui_panel_rows(panels: &[UiPanelDescriptor]) -> Vec<String> {
    let mut rows = panels
        .iter()
        .map(|descriptor| {
            let mut fields = vec![format!(
                "scope: {}; priority={}",
                ui_mutation_scope_label(descriptor.scope),
                descriptor.priority
            )];
            fields.extend(ui_panel_refresh_fields(descriptor));
            format!(
                "{} {} {} [{}]",
                descriptor.plugin_id.as_str(),
                descriptor.id,
                descriptor.title,
                fields.join("; ")
            )
        })
        .collect::<Vec<_>>();
    rows.sort();
    rows
}

fn ui_panel_refresh_fields(descriptor: &UiPanelDescriptor) -> Vec<String> {
    let Some(refresh) = descriptor.refresh.as_ref() else {
        return Vec::new();
    };
    let mut fields = vec![format!(
        "refresh={}",
        ui_panel_refresh_kind_label(refresh.kind)
    )];
    if let Some(min_interval_ms) = refresh.min_interval_ms {
        fields.push(format!("min_interval_ms={min_interval_ms}"));
    }
    if let Some(auto_refresh_ms) = refresh.auto_refresh_ms {
        fields.push(format!("auto_refresh_ms={auto_refresh_ms}"));
    }
    fields
}

fn ui_widget_refresh_fields(descriptor: &UiWidgetDescriptor) -> Vec<String> {
    let Some(refresh) = descriptor.refresh.as_ref() else {
        return Vec::new();
    };
    let mut fields = vec![format!(
        "refresh={}",
        ui_widget_refresh_kind_label(refresh.kind)
    )];
    if let Some(min_interval_ms) = refresh.min_interval_ms {
        fields.push(format!("min_interval_ms={min_interval_ms}"));
    }
    if let Some(auto_refresh_ms) = refresh.auto_refresh_ms {
        fields.push(format!("auto_refresh_ms={auto_refresh_ms}"));
    }
    fields
}

fn metric_surface_labels(surfaces: &[MetricSurface]) -> String {
    if surfaces.is_empty() {
        return "none".to_owned();
    }
    surfaces
        .iter()
        .map(|surface| match surface {
            MetricSurface::StatusLine => "status_line",
            MetricSurface::Sidebar => "sidebar",
            MetricSurface::Panel => "panel",
        })
        .collect::<Vec<_>>()
        .join(",")
}

const fn metric_unit_label(unit: MetricUnit) -> &'static str {
    match unit {
        MetricUnit::Count => "count",
        MetricUnit::Percent => "percent",
        MetricUnit::Tokens => "tokens",
        MetricUnit::Usd => "usd",
        MetricUnit::Digest => "digest",
        MetricUnit::Text => "text",
    }
}

const fn ui_mutation_scope_label(scope: UiMutationScope) -> &'static str {
    match scope {
        UiMutationScope::InfoSidebar => "info_sidebar",
        UiMutationScope::TaskPanel => "task_panel",
        UiMutationScope::SessionSidebar => "session_sidebar",
    }
}

const fn ui_widget_kind_label(kind: UiWidgetKind) -> &'static str {
    match kind {
        UiWidgetKind::Text => "text",
        UiWidgetKind::Action => "action",
    }
}

const fn ui_widget_refresh_kind_label(kind: UiWidgetRefreshKind) -> &'static str {
    match kind {
        UiWidgetRefreshKind::ProcessBridge => "process_bridge",
    }
}

const fn ui_panel_refresh_kind_label(kind: UiPanelRefreshKind) -> &'static str {
    match kind {
        UiPanelRefreshKind::ProcessBridge => "process_bridge",
    }
}
