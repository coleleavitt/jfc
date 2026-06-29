use std::collections::HashMap;

use super::status_plugins::*;
use super::status_widgets::*;

#[test]
fn plugin_health_badge_reports_failures_first_normal() {
    let summary = jfc_plugin_host::PluginHealthSummary {
        total: 4,
        active: 2,
        disabled: 1,
        failed: 1,
        registered: 0,
        error_count: 0,
    };

    assert_eq!(
        plugin_health_badge(&summary),
        Some("plugins 1 failed".to_owned())
    );
}

#[test]
fn empty_plugin_health_stays_hidden_normal() {
    assert_eq!(
        plugin_health_badge(&jfc_plugin_host::PluginHealthSummary::default()),
        None
    );
}

#[test]
fn plugin_health_detail_rows_without_reload_report_stays_health_only_normal() {
    let summary = jfc_plugin_host::PluginHealthSummary {
        total: 1,
        active: 1,
        disabled: 0,
        failed: 0,
        registered: 0,
        error_count: 0,
    };

    assert_eq!(
        row_texts(plugin_health_detail_render_rows(&summary, None, &[])),
        vec!["1/1 active".to_owned()]
    );
}

#[test]
fn plugin_health_detail_rows_surface_reload_state_normal() {
    let summary = jfc_plugin_host::PluginHealthSummary {
        total: 4,
        active: 2,
        disabled: 1,
        failed: 1,
        registered: 0,
        error_count: 3,
    };
    let report = jfc_plugin_host::PluginReloadReport {
        diagnostics: jfc_plugin_host::PluginHostDiagnostics {
            health: summary,
            counts: jfc_plugin_host::PluginDescriptorCounts {
                plugins: 4,
                active_plugins: 2,
                failed_plugins: 1,
                hooks: 0,
                services: 0,
                tools: 1,
                providers: 0,
                resources: 2,
                commands: 0,
                ui_slots: 3,
                ui_panels: 1,
                ui_widgets: 1,
                runtime_actions: 6,
                runtime_extensions: 4,
                agent_launches: 5,
                metrics: 6,
                errors: 3,
            },
            descriptor_digest: "bbbbbbbbbbbbbbbb".to_owned(),
            descriptor_issues: Vec::new(),
            active_plugins: Vec::new(),
            failed_plugins: Vec::new(),
        },
        previous_descriptor_digest: Some("aaaaaaaaaaaaaaaa".to_owned()),
        changed: Some(true),
    };

    let rows = row_texts(plugin_health_detail_render_rows(
        &summary,
        Some(&report),
        &[],
    ));

    assert_eq!(
        rows,
        vec![
            "2/4 active".to_owned(),
            "1 failed".to_owned(),
            "3 errors".to_owned(),
            "1 disabled".to_owned(),
            "descriptors changed".to_owned(),
            "digest bbbbbbbbbbbbbbbb".to_owned(),
            "tools 1 · resources 2 · ui slots 3 · actions 6 · runtime ext 4 · agent launches 5"
                .to_owned(),
            "widgets 1".to_owned(),
            "panels 1".to_owned(),
            "metrics 6".to_owned(),
        ]
    );
}

fn row_texts(rows: Vec<PluginDetailRow>) -> Vec<String> {
    rows.into_iter().map(|row| row.text).collect()
}

#[test]
fn plugin_health_detail_render_rows_surface_descriptor_issues_normal() {
    let summary = jfc_plugin_host::PluginHealthSummary {
        total: 1,
        active: 1,
        disabled: 0,
        failed: 0,
        registered: 0,
        error_count: 0,
    };
    let plugin_id = jfc_plugin_sdk::PluginId::new("acme");
    let report = jfc_plugin_host::PluginReloadReport {
        diagnostics: jfc_plugin_host::PluginHostDiagnostics {
            health: summary,
            counts: jfc_plugin_host::PluginDescriptorCounts::default(),
            descriptor_digest: "bbbbbbbbbbbbbbbb".to_owned(),
            descriptor_issues: vec![jfc_plugin_host::PluginDescriptorIssue {
                kind: jfc_plugin_host::PluginDescriptorIssueKind::MissingRuntimeAction,
                severity: jfc_plugin_host::PluginDescriptorIssueSeverity::Error,
                actionability:
                    jfc_plugin_host::PluginDescriptorIssueActionability::AddRuntimeAction,
                plugin_id: plugin_id.clone(),
                descriptor_kind: jfc_plugin_host::PluginDescriptorKind::UiWidget,
                descriptor_id: "queue".to_owned(),
                target_plugin_id: plugin_id.clone(),
                target_kind: jfc_plugin_host::PluginDescriptorTargetKind::RuntimeAction,
                target_id: "queue.run".to_owned(),
                message: "descriptor references a missing runtime action".to_owned(),
                repair_action: jfc_plugin_host::PluginDescriptorRepairAction::AddRuntimeAction {
                    plugin_id: plugin_id.clone(),
                    action_id: "queue.run".to_owned(),
                },
                repair_hint: "Add runtime action 'queue.run' to plugin 'acme', or point UI widget 'queue' in plugin 'acme' at an existing runtime action.".to_owned(),
            }],
            active_plugins: Vec::new(),
            failed_plugins: Vec::new(),
        },
        previous_descriptor_digest: None,
        changed: None,
    };

    let rows = plugin_health_detail_render_rows(&summary, Some(&report), &[]);

    assert!(rows.iter().any(|row| row.text == "descriptor issues 1"));
    let issue_row = rows
        .iter()
        .find(|row| row.text.contains("missing_runtime_action"))
        .expect("descriptor issue row");
    assert_eq!(issue_row.tone, PluginDetailRowTone::Error);
    assert!(
        rows.iter()
            .any(|row| row.text.contains("fix Add runtime action 'queue.run'"))
    );
}

#[test]
fn plugin_health_detail_render_rows_surface_diagnostics_action_normal() {
    let summary = jfc_plugin_host::PluginHealthSummary {
        total: 1,
        active: 1,
        disabled: 0,
        failed: 0,
        registered: 0,
        error_count: 0,
    };
    let plugin_id = jfc_plugin_sdk::PluginId::new("builtin.jfc-ux");
    let actions = vec![
        jfc_plugin_sdk::RuntimeActionDescriptor::new(
            plugin_id.clone(),
            "command_palette.plugin_diagnostics",
            "Run Plugin Diagnostics",
            "Run plugin diagnostics",
            jfc_plugin_sdk::RuntimeActionKind::PluginDiagnostics,
        ),
        jfc_plugin_sdk::RuntimeActionDescriptor::new(
            plugin_id,
            "plugin.smoke",
            "Smoke Plugin",
            "Run smoke checks",
            jfc_plugin_sdk::RuntimeActionKind::PluginSmoke,
        ),
    ];

    let rows = row_texts(plugin_health_detail_render_rows(&summary, None, &actions));

    assert!(rows.contains(&"diagnostics action Run Plugin Diagnostics".to_owned()));
    assert!(rows.contains(&"diagnostics smoke checks 1".to_owned()));
}

#[test]
fn cache_hit_badge_is_metric_descriptor_gated_normal() {
    let metrics = vec![
        jfc_plugin_sdk::MetricDescriptor::new(
            jfc_plugin_sdk::PluginId::new("test"),
            jfc_plugin_host::BUILTIN_CACHE_HIT_METRIC_ID,
            "Cache hit",
            "Cache hit",
            jfc_plugin_sdk::MetricUnit::Percent,
        )
        .with_surface(jfc_plugin_sdk::MetricSurface::StatusLine),
    ];
    let mut usage_by_model = HashMap::new();
    usage_by_model.insert(
        "model".to_owned(),
        jfc_core::ModelUsage {
            input_tokens: 100,
            cache_read_tokens: 40,
            ..Default::default()
        },
    );

    assert_eq!(
        cache_hit_badge(&metrics, &usage_by_model),
        Some("cache 40%".to_owned())
    );
    assert_eq!(cache_hit_badge(&[], &usage_by_model), None);
}

#[test]
fn rsi_metric_rows_read_latest_request_metadata_normal() {
    let metrics = vec![
        jfc_plugin_sdk::MetricDescriptor::new(
            jfc_plugin_sdk::PluginId::new("test"),
            jfc_plugin_host::BUILTIN_RSI_PROMPT_SECTIONS_METRIC_ID,
            "RSI prompt",
            "RSI prompt",
            jfc_plugin_sdk::MetricUnit::Count,
        )
        .with_surface(jfc_plugin_sdk::MetricSurface::Sidebar),
    ];
    let metadata = jfc_engine::runtime::StreamRequestMetadata {
        advertised_tool_count: 0,
        action_expected: false,
        tool_choice: jfc_engine::runtime::StreamToolChoice::Auto,
        resolved_model: None,
        context_budget: None,
        context_pressure_nudge: None,
        provider_history_archive_recall_ids: Vec::new(),
        rsi_prompt_sections: 3,
        rsi_tool_visibility_rules: 1,
    };

    assert_eq!(
        rsi_metric_rows(&metrics, Some(&metadata)),
        vec!["rsi prompt sections 3".to_owned()]
    );
}

#[test]
fn metric_panel_descriptor_rows_sort_and_format_panel_metrics_normal() {
    let metrics = vec![
        jfc_plugin_sdk::MetricDescriptor::new(
            jfc_plugin_sdk::PluginId::new("demo"),
            "low",
            "Low",
            "Low",
            jfc_plugin_sdk::MetricUnit::Text,
        )
        .with_surface(jfc_plugin_sdk::MetricSurface::Panel)
        .with_priority(1),
        jfc_plugin_sdk::MetricDescriptor::new(
            jfc_plugin_sdk::PluginId::new("demo"),
            "high",
            "High",
            "High",
            jfc_plugin_sdk::MetricUnit::Count,
        )
        .with_surface(jfc_plugin_sdk::MetricSurface::Panel)
        .with_priority(10),
        jfc_plugin_sdk::MetricDescriptor::new(
            jfc_plugin_sdk::PluginId::new("demo"),
            "sidebar",
            "Sidebar",
            "Sidebar",
            jfc_plugin_sdk::MetricUnit::Count,
        )
        .with_surface(jfc_plugin_sdk::MetricSurface::Sidebar),
    ];

    assert_eq!(
        metric_panel_descriptor_rows(&metrics),
        vec![
            "High · count · demo:high".to_owned(),
            "Low · text · demo:low".to_owned(),
        ]
    );
}

#[test]
fn ui_widget_panel_rows_sort_and_format_info_sidebar_widgets_normal() {
    let widgets = vec![
        jfc_plugin_sdk::UiWidgetDescriptor::new(
            jfc_plugin_sdk::PluginId::new("demo"),
            jfc_plugin_sdk::UiMutationScope::TaskPanel,
            "hidden",
            "Hidden",
            jfc_plugin_sdk::UiWidgetKind::Text,
        ),
        jfc_plugin_sdk::UiWidgetDescriptor::new(
            jfc_plugin_sdk::PluginId::new("demo"),
            jfc_plugin_sdk::UiMutationScope::InfoSidebar,
            "low",
            "Low",
            jfc_plugin_sdk::UiWidgetKind::Text,
        )
        .with_body("low body")
        .with_priority(1),
        jfc_plugin_sdk::UiWidgetDescriptor::new(
            jfc_plugin_sdk::PluginId::new("demo"),
            jfc_plugin_sdk::UiMutationScope::InfoSidebar,
            "high",
            "High",
            jfc_plugin_sdk::UiWidgetKind::Action,
        )
        .with_runtime_action("demo.refresh")
        .with_priority(10),
    ];

    assert_eq!(
        ui_widget_panel_rows(
            &widgets,
            &crate::app::UiWidgetSnapshots::default(),
            &crate::app::UiWidgetRefreshStatuses::default(),
            None
        ),
        vec![
            "High · action demo.refresh · demo:high".to_owned(),
            "Low · low body · demo:low".to_owned(),
        ]
    );
}
