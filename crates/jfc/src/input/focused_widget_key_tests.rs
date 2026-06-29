use std::sync::Arc;

use super::handle_key;
use crate::app::App;
use crate::runtime::EngineEvent;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use jfc_plugin_sdk::{
    PluginId, ProcessBridgeCommand, RuntimeActionDescriptor, RuntimeActionKind, UiMutationScope,
    UiWidgetDescriptor, UiWidgetKind, UiWidgetRefreshDescriptor,
};
use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

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
async fn alt_right_and_left_move_info_sidebar_widget_focus_normal() {
    let mut app = test_app();
    app.info_sidebar.visible = true;
    app.plugins.ui_widget_descriptors = vec![
        widget("low", "Low", 1),
        widget("high", "High", 10),
        UiWidgetDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::TaskPanel,
            "hidden",
            "Hidden",
            UiWidgetKind::Text,
        )
        .with_priority(100),
    ];
    let tx = channel();

    handle_key(&mut app, key_mod(KeyCode::Right, KeyModifiers::ALT), &tx)
        .await
        .unwrap();
    let focus = app.info_sidebar.focused_widget.as_ref().unwrap();
    assert_eq!(focus.plugin_id, "demo");
    assert_eq!(focus.widget_id, "high");

    handle_key(&mut app, key_mod(KeyCode::Right, KeyModifiers::ALT), &tx)
        .await
        .unwrap();
    assert_eq!(
        app.info_sidebar.focused_widget.as_ref().unwrap().widget_id,
        "low"
    );

    handle_key(&mut app, key_mod(KeyCode::Left, KeyModifiers::ALT), &tx)
        .await
        .unwrap();
    assert_eq!(
        app.info_sidebar.focused_widget.as_ref().unwrap().widget_id,
        "high"
    );
}

#[tokio::test]
async fn alt_enter_invokes_focused_info_sidebar_widget_action_normal() {
    let mut app = test_app();
    app.info_sidebar.visible = true;
    let plugin_id = PluginId::new("demo");
    app.plugins.ui_widget_descriptors = vec![
        UiWidgetDescriptor::new(
            plugin_id.clone(),
            UiMutationScope::InfoSidebar,
            "toggle",
            "Toggle",
            UiWidgetKind::Action,
        )
        .with_runtime_action("toggle.info"),
    ];
    app.plugins.runtime_action_descriptors = vec![
        RuntimeActionDescriptor::new(
            plugin_id.clone(),
            "toggle.info",
            "Toggle Info",
            "Toggle info sidebar from focused widget",
            RuntimeActionKind::HostAction,
        )
        .with_payload(serde_json::json!({ "action": "toggle_info_sidebar" })),
    ];
    app.info_sidebar.focus_widget(plugin_id.as_str(), "toggle");
    let tx = channel();

    handle_key(&mut app, key_mod(KeyCode::Enter, KeyModifiers::ALT), &tx)
        .await
        .unwrap();

    assert!(!app.info_sidebar.visible);
}

#[tokio::test]
async fn alt_enter_refreshes_focused_info_sidebar_widget_snapshot_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut app = test_app();
    app.engine.cwd = tmp.path().to_string_lossy().into_owned();
    app.info_sidebar.visible = true;
    let plugin_id = PluginId::new("demo");
    app.plugins.ui_widget_descriptors = vec![
        UiWidgetDescriptor::new(
            plugin_id.clone(),
            UiMutationScope::InfoSidebar,
            "reviews",
            "Reviews",
            UiWidgetKind::Text,
        )
        .with_refresh(UiWidgetRefreshDescriptor::process_bridge(
            bridge_refresh_handler("fresh bridge body"),
        )),
    ];
    app.info_sidebar.focus_widget(plugin_id.as_str(), "reviews");
    let tx = channel();

    handle_key(&mut app, key_mod(KeyCode::Enter, KeyModifiers::ALT), &tx)
        .await
        .unwrap();

    let snapshot = app
        .plugins
        .ui_widget_snapshots
        .get("demo\0info_sidebar\0reviews")
        .expect("widget snapshot");
    assert_eq!(snapshot.body.as_deref(), Some("fresh bridge body"));
    assert_eq!(snapshot.state, Some(serde_json::json!({ "seen": 1 })));
}

fn test_app() -> App {
    let mut app = App::new(Arc::new(TestProvider), "test-model");
    app.engine.task_store = jfc_session::TaskStore::in_memory();
    app
}

fn channel() -> tokio::sync::mpsc::Sender<EngineEvent> {
    let (tx, _rx) = tokio::sync::mpsc::channel(4);
    tx
}

fn key_mod(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

fn widget(id: &'static str, label: &'static str, priority: i32) -> UiWidgetDescriptor {
    UiWidgetDescriptor::new(
        PluginId::new("demo"),
        UiMutationScope::InfoSidebar,
        id,
        label,
        UiWidgetKind::Action,
    )
    .with_priority(priority)
}

fn bridge_refresh_handler(body: &str) -> String {
    let script = format!(
        "read line\nid=$(printf '%s\\n' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nprintf '{{\"type\":\"response\",\"id\":\"%s\",\"response\":{{\"kind\":\"ui_widget_refresh\",\"result\":{{\"body\":\"{body}\",\"state\":{{\"seen\":1}}}}}}}}\\n' \"$id\"\n"
    );
    let command = ProcessBridgeCommand::new("/bin/sh").with_args(["-c", script.as_str()]);
    serde_json::to_string(&command).expect("bridge handler json")
}
