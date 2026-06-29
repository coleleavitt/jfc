use super::status_panels::info_sidebar_panel_sections;
use crate::app::{UiPanelRefreshStatus, UiPanelRefreshStatuses, UiPanelSnapshot, UiPanelSnapshots};
use jfc_plugin_sdk::{PluginId, UiMutationScope, UiPanelDescriptor};

#[test]
fn info_sidebar_panel_sections_sort_and_format_panels_normal() {
    let panels = vec![
        UiPanelDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::TaskPanel,
            "hidden",
            "Hidden",
        )
        .with_body("not shown")
        .with_priority(100),
        UiPanelDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::InfoSidebar,
            "low",
            "Low",
        )
        .with_body("low body")
        .with_priority(1),
        UiPanelDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::InfoSidebar,
            "high",
            "High",
        )
        .with_body("first line\n\n second line ")
        .with_runtime_action("demo.open")
        .with_priority(10),
    ];

    assert_eq!(
        info_sidebar_panel_sections(
            &panels,
            &UiPanelSnapshots::default(),
            &UiPanelRefreshStatuses::default(),
            None,
        ),
        vec![
            super::status_panels::PluginPanelSection {
                title: "High · demo:high".to_owned(),
                rows: vec![
                    "first line".to_owned(),
                    "second line".to_owned(),
                    "action demo.open".to_owned(),
                ],
            },
            super::status_panels::PluginPanelSection {
                title: "Low · demo:low".to_owned(),
                rows: vec!["low body".to_owned()],
            },
        ]
    );
}

#[test]
fn info_sidebar_panel_sections_marks_focused_panel_normal() {
    let panels = vec![
        UiPanelDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::InfoSidebar,
            "high",
            "High",
        )
        .with_body("body"),
    ];
    let focused = crate::app::FocusedUiPanel {
        plugin_id: "demo".to_owned(),
        panel_id: "high".to_owned(),
    };

    assert_eq!(
        info_sidebar_panel_sections(
            &panels,
            &UiPanelSnapshots::default(),
            &UiPanelRefreshStatuses::default(),
            Some(&focused),
        ),
        vec![super::status_panels::PluginPanelSection {
            title: "> High · demo:high".to_owned(),
            rows: vec!["body".to_owned()],
        }]
    );
}

#[test]
fn panel_rows_prefer_host_snapshot_body_over_manifest_body_normal() {
    let plugin_id = PluginId::new("demo");
    let panels = vec![
        UiPanelDescriptor::new(
            plugin_id.clone(),
            UiMutationScope::InfoSidebar,
            "reviews",
            "Reviews",
        )
        .with_body("stale manifest body"),
    ];
    let mut snapshots = UiPanelSnapshots::default();
    snapshots.insert(
        "demo\0info_sidebar\0reviews".to_owned(),
        UiPanelSnapshot {
            body: Some("fresh bridge body\nsecond line".to_owned()),
            state: None,
        },
    );

    assert_eq!(
        info_sidebar_panel_sections(
            &panels,
            &snapshots,
            &UiPanelRefreshStatuses::default(),
            None
        ),
        vec![super::status_panels::PluginPanelSection {
            title: "Reviews · demo:reviews".to_owned(),
            rows: vec!["fresh bridge body".to_owned(), "second line".to_owned()],
        }]
    );
}

#[test]
fn panel_rows_include_refresh_status_for_refreshable_panels_normal() {
    let plugin_id = PluginId::new("demo");
    let panel = UiPanelDescriptor::new(
        plugin_id,
        UiMutationScope::InfoSidebar,
        "reviews",
        "Reviews",
    )
    .with_body("stale")
    .with_refresh(jfc_plugin_sdk::UiPanelRefreshDescriptor::process_bridge(
        "/bin/reviews-panel",
    ));
    let mut status = UiPanelRefreshStatuses::default();
    status.insert(
        "demo\0info_sidebar\0reviews".to_owned(),
        UiPanelRefreshStatus {
            last_success_at: Some(std::time::Instant::now()),
            ..UiPanelRefreshStatus::default()
        },
    );

    assert_eq!(
        info_sidebar_panel_sections(&[panel], &UiPanelSnapshots::default(), &status, None),
        vec![super::status_panels::PluginPanelSection {
            title: "Reviews · demo:reviews".to_owned(),
            rows: vec!["stale".to_owned(), "refresh ok just now".to_owned()],
        }]
    );
}
