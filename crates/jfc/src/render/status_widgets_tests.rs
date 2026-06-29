use super::status_widgets::ui_widget_panel_rows;
use super::status_widgets::{session_sidebar_widget_rows, task_panel_widget_rows};
use crate::app::{
    UiWidgetRefreshStatus, UiWidgetRefreshStatuses, UiWidgetSnapshot, UiWidgetSnapshots,
};

#[test]
fn ui_widget_panel_rows_marks_focused_info_sidebar_widget_normal() {
    let widgets = vec![
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
    let focused = crate::app::FocusedUiWidget {
        plugin_id: "demo".to_owned(),
        widget_id: "high".to_owned(),
    };

    assert_eq!(
        ui_widget_panel_rows(
            &widgets,
            &UiWidgetSnapshots::default(),
            &UiWidgetRefreshStatuses::default(),
            Some(&focused),
        ),
        vec!["> High · action demo.refresh · demo:high".to_owned()]
    );
}

#[test]
fn widget_rows_filter_task_and_session_scopes_normal() {
    let widgets = vec![
        jfc_plugin_sdk::UiWidgetDescriptor::new(
            jfc_plugin_sdk::PluginId::new("demo"),
            jfc_plugin_sdk::UiMutationScope::TaskPanel,
            "task",
            "Task Widget",
            jfc_plugin_sdk::UiWidgetKind::Text,
        )
        .with_body("task body")
        .with_priority(20),
        jfc_plugin_sdk::UiWidgetDescriptor::new(
            jfc_plugin_sdk::PluginId::new("demo"),
            jfc_plugin_sdk::UiMutationScope::SessionSidebar,
            "session",
            "Session Widget",
            jfc_plugin_sdk::UiWidgetKind::Action,
        )
        .with_runtime_action("session.open")
        .with_priority(10),
    ];

    assert_eq!(
        task_panel_widget_rows(
            &widgets,
            &UiWidgetSnapshots::default(),
            &UiWidgetRefreshStatuses::default()
        ),
        vec!["Task Widget · task body · demo:task".to_owned()]
    );
    assert_eq!(
        session_sidebar_widget_rows(
            &widgets,
            &UiWidgetSnapshots::default(),
            &UiWidgetRefreshStatuses::default()
        ),
        vec!["Session Widget · action session.open · demo:session".to_owned()]
    );
}

#[test]
fn widget_rows_prefer_host_snapshot_body_over_manifest_body_normal() {
    let plugin_id = jfc_plugin_sdk::PluginId::new("demo");
    let widgets = vec![
        jfc_plugin_sdk::UiWidgetDescriptor::new(
            plugin_id.clone(),
            jfc_plugin_sdk::UiMutationScope::InfoSidebar,
            "reviews",
            "Reviews",
            jfc_plugin_sdk::UiWidgetKind::Text,
        )
        .with_body("stale manifest body"),
    ];
    let mut snapshots = UiWidgetSnapshots::default();
    snapshots.insert(
        "demo\0info_sidebar\0reviews".to_owned(),
        UiWidgetSnapshot {
            body: Some("fresh bridge body".to_owned()),
            state: None,
        },
    );

    assert_eq!(
        ui_widget_panel_rows(
            &widgets,
            &snapshots,
            &UiWidgetRefreshStatuses::default(),
            None
        ),
        vec!["Reviews · fresh bridge body · demo:reviews".to_owned()]
    );
}

#[test]
fn widget_rows_include_refresh_status_for_refreshable_widgets_normal() {
    let plugin_id = jfc_plugin_sdk::PluginId::new("demo");
    let widget = jfc_plugin_sdk::UiWidgetDescriptor::new(
        plugin_id,
        jfc_plugin_sdk::UiMutationScope::InfoSidebar,
        "reviews",
        "Reviews",
        jfc_plugin_sdk::UiWidgetKind::Text,
    )
    .with_body("stale")
    .with_refresh(jfc_plugin_sdk::UiWidgetRefreshDescriptor::process_bridge(
        "/bin/reviews-widget",
    ));
    let mut status = UiWidgetRefreshStatuses::default();
    status.insert(
        "demo\0info_sidebar\0reviews".to_owned(),
        UiWidgetRefreshStatus {
            last_success_at: Some(std::time::Instant::now()),
            ..UiWidgetRefreshStatus::default()
        },
    );

    assert_eq!(
        ui_widget_panel_rows(&[widget], &UiWidgetSnapshots::default(), &status, None),
        vec!["Reviews · stale · refresh ok just now · demo:reviews".to_owned()]
    );
}
