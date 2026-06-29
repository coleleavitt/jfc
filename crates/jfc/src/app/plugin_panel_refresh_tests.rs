use std::sync::Arc;

use jfc_plugin_sdk::{
    PluginId, ProcessBridgeCommand, UiMutationScope, UiPanelDescriptor, UiPanelRefreshDescriptor,
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
async fn ui_panel_refresh_honors_min_interval_debounce_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut app = test_app(tmp.path());
    let first_panel = panel("first").with_refresh(
        UiPanelRefreshDescriptor::process_bridge(bridge_refresh_handler("first"))
            .with_min_interval_ms(60_000),
    );

    assert!(
        app.refresh_ui_panel_snapshot(&first_panel)
            .await
            .expect("refresh")
    );
    let second_panel = panel("second").with_refresh(
        UiPanelRefreshDescriptor::process_bridge(bridge_refresh_handler("second"))
            .with_min_interval_ms(60_000),
    );

    assert!(
        !app.refresh_ui_panel_snapshot(&second_panel)
            .await
            .expect("debounced refresh")
    );
    let snapshot = app
        .plugins
        .ui_panel_snapshots
        .get("demo\0info_sidebar\0reviews")
        .expect("snapshot");
    assert_eq!(snapshot.body.as_deref(), Some("first"));
    let status = app
        .plugins
        .ui_panel_refresh_status
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
async fn due_ui_panel_auto_refresh_runs_on_declared_cadence_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut app = test_app(tmp.path());
    app.plugins.ui_panel_descriptors = vec![
        panel("auto").with_refresh(
            UiPanelRefreshDescriptor::process_bridge(bridge_refresh_handler("auto"))
                .with_auto_refresh_ms(1),
        ),
    ];

    assert!(app.refresh_due_ui_panel_snapshots().await);
    assert!(!app.refresh_due_ui_panel_snapshots().await);
    let snapshot = app
        .plugins
        .ui_panel_snapshots
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

fn panel(title: &str) -> UiPanelDescriptor {
    UiPanelDescriptor::new(
        PluginId::new("demo"),
        UiMutationScope::InfoSidebar,
        "reviews",
        title,
    )
}

fn bridge_refresh_handler(body: &str) -> String {
    let script = format!(
        "read line\nid=$(printf '%s\\n' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nprintf '{{\"type\":\"response\",\"id\":\"%s\",\"response\":{{\"kind\":\"ui_panel_refresh\",\"result\":{{\"body\":\"{body}\"}}}}}}\\n' \"$id\"\n"
    );
    let command = ProcessBridgeCommand::new("/bin/sh").with_args(["-c", script.as_str()]);
    serde_json::to_string(&command).expect("bridge handler json")
}
