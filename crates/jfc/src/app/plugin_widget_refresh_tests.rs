use std::sync::Arc;

use jfc_plugin_sdk::{
    PluginId, ProcessBridgeCommand, UiMutationScope, UiWidgetDescriptor, UiWidgetKind,
    UiWidgetRefreshDescriptor,
};
use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

use super::App;

struct TestProvider;

#[async_trait::async_trait]
impl Provider for TestProvider {
    fn name(&self) -> &str {
        "test"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(
        &self,
        _messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        Ok(Box::pin(futures::stream::empty()))
    }
}

impl jfc_provider::seal::Sealed for TestProvider {}

#[tokio::test]
async fn ui_widget_refresh_honors_min_interval_debounce_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut app = test_app(tmp.path());
    let first_widget = widget("first").with_refresh(
        UiWidgetRefreshDescriptor::process_bridge(bridge_refresh_handler("first"))
            .with_min_interval_ms(60_000),
    );

    assert!(
        app.refresh_ui_widget_snapshot(&first_widget)
            .await
            .expect("refresh")
    );
    let second_widget = widget("second").with_refresh(
        UiWidgetRefreshDescriptor::process_bridge(bridge_refresh_handler("second"))
            .with_min_interval_ms(60_000),
    );

    assert!(
        !app.refresh_ui_widget_snapshot(&second_widget)
            .await
            .expect("debounced refresh")
    );
    let snapshot = app
        .plugins
        .ui_widget_snapshots
        .get("demo\0info_sidebar\0reviews")
        .expect("snapshot");
    assert_eq!(snapshot.body.as_deref(), Some("first"));
    let status = app
        .plugins
        .ui_widget_refresh_status
        .get("demo\0info_sidebar\0reviews")
        .expect("refresh status");
    assert!(
        status
            .last_skip_reason
            .as_deref()
            .is_some_and(|reason| { reason.starts_with("debounced ") })
    );
}

#[tokio::test]
async fn due_ui_widget_auto_refresh_runs_on_declared_cadence_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut app = test_app(tmp.path());
    app.plugins.ui_widget_descriptors = vec![
        widget("auto").with_refresh(
            UiWidgetRefreshDescriptor::process_bridge(bridge_refresh_handler("auto"))
                .with_auto_refresh_ms(1),
        ),
    ];

    assert!(app.refresh_due_ui_widget_snapshots().await);
    assert!(!app.refresh_due_ui_widget_snapshots().await);
    let snapshot = app
        .plugins
        .ui_widget_snapshots
        .get("demo\0info_sidebar\0reviews")
        .expect("snapshot");
    assert_eq!(snapshot.body.as_deref(), Some("auto"));
}

fn test_app(project_root: &std::path::Path) -> App {
    let mut app = App::new(Arc::new(TestProvider), "test-model");
    app.engine.task_store = jfc_session::TaskStore::in_memory();
    app.engine.cwd = project_root.to_string_lossy().into_owned();
    app
}

fn widget(label: &str) -> UiWidgetDescriptor {
    UiWidgetDescriptor::new(
        PluginId::new("demo"),
        UiMutationScope::InfoSidebar,
        "reviews",
        label,
        UiWidgetKind::Text,
    )
}

fn bridge_refresh_handler(body: &str) -> String {
    let script = format!(
        "read line\nid=$(printf '%s\\n' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nprintf '{{\"type\":\"response\",\"id\":\"%s\",\"response\":{{\"kind\":\"ui_widget_refresh\",\"result\":{{\"body\":\"{body}\"}}}}}}\\n' \"$id\"\n"
    );
    let command = ProcessBridgeCommand::new("/bin/sh").with_args(["-c", script.as_str()]);
    serde_json::to_string(&command).expect("bridge handler json")
}
