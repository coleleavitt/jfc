use std::sync::Arc;

use super::handle_key;
use crate::app::App;
use crate::runtime::EngineEvent;
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use jfc_plugin_sdk::{
    PluginId, ProcessBridgeCommand, RuntimeActionDescriptor, RuntimeActionKind, UiMutationScope,
    UiPanelDescriptor, UiPanelRefreshDescriptor,
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
async fn alt_down_and_up_move_info_sidebar_panel_focus_normal() {
    let mut app = test_app();
    app.info_sidebar.visible = true;
    app.plugins.ui_panel_descriptors = vec![
        panel("low", "Low", 1),
        panel("high", "High", 10),
        UiPanelDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::TaskPanel,
            "hidden",
            "Hidden",
        )
        .with_priority(100),
    ];
    let tx = channel();

    handle_key(&mut app, key_mod(KeyCode::Down, KeyModifiers::ALT), &tx)
        .await
        .unwrap();
    let focus = app.info_sidebar.focused_panel.as_ref().unwrap();
    assert_eq!(focus.plugin_id, "demo");
    assert_eq!(focus.panel_id, "high");
    assert!(app.info_sidebar.focused_widget.is_none());

    handle_key(&mut app, key_mod(KeyCode::Down, KeyModifiers::ALT), &tx)
        .await
        .unwrap();
    assert_eq!(
        app.info_sidebar.focused_panel.as_ref().unwrap().panel_id,
        "low"
    );

    handle_key(&mut app, key_mod(KeyCode::Up, KeyModifiers::ALT), &tx)
        .await
        .unwrap();
    assert_eq!(
        app.info_sidebar.focused_panel.as_ref().unwrap().panel_id,
        "high"
    );
}

#[tokio::test]
async fn alt_enter_invokes_focused_info_sidebar_panel_action_normal() {
    let mut app = test_app();
    app.info_sidebar.visible = true;
    let plugin_id = PluginId::new("demo");
    app.plugins.ui_panel_descriptors = vec![
        UiPanelDescriptor::new(
            plugin_id.clone(),
            UiMutationScope::InfoSidebar,
            "toggle",
            "Toggle",
        )
        .with_runtime_action("toggle.info"),
    ];
    app.plugins.runtime_action_descriptors = vec![
        RuntimeActionDescriptor::new(
            plugin_id.clone(),
            "toggle.info",
            "Toggle Info",
            "Toggle info sidebar from focused panel",
            RuntimeActionKind::HostAction,
        )
        .with_payload(serde_json::json!({ "action": "toggle_info_sidebar" })),
    ];
    app.info_sidebar.focus_panel(plugin_id.as_str(), "toggle");
    let tx = channel();

    handle_key(&mut app, key_mod(KeyCode::Enter, KeyModifiers::ALT), &tx)
        .await
        .unwrap();

    assert!(!app.info_sidebar.visible);
}

#[tokio::test]
async fn alt_enter_refreshes_focused_info_sidebar_panel_snapshot_normal() {
    let tmp = tempfile::TempDir::new().expect("tempdir");
    let mut app = test_app();
    app.engine.cwd = tmp.path().to_string_lossy().into_owned();
    app.info_sidebar.visible = true;
    let plugin_id = PluginId::new("demo");
    app.plugins.ui_panel_descriptors = vec![
        UiPanelDescriptor::new(
            plugin_id.clone(),
            UiMutationScope::InfoSidebar,
            "reviews",
            "Reviews",
        )
        .with_refresh(UiPanelRefreshDescriptor::process_bridge(
            bridge_refresh_handler("fresh panel body"),
        )),
    ];
    app.info_sidebar.focus_panel(plugin_id.as_str(), "reviews");
    let tx = channel();

    handle_key(&mut app, key_mod(KeyCode::Enter, KeyModifiers::ALT), &tx)
        .await
        .unwrap();

    let snapshot = app
        .plugins
        .ui_panel_snapshots
        .get("demo\0info_sidebar\0reviews")
        .expect("panel snapshot");
    assert_eq!(snapshot.body.as_deref(), Some("fresh panel body"));
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

fn panel(id: &'static str, title: &'static str, priority: i32) -> UiPanelDescriptor {
    UiPanelDescriptor::new(
        PluginId::new("demo"),
        UiMutationScope::InfoSidebar,
        id,
        title,
    )
    .with_priority(priority)
}

fn bridge_refresh_handler(body: &str) -> String {
    let script = format!(
        "read line\nid=$(printf '%s\\n' \"$line\" | sed -n 's/.*\"id\":\"\\([^\"]*\\)\".*/\\1/p')\nprintf '{{\"type\":\"response\",\"id\":\"%s\",\"response\":{{\"kind\":\"ui_panel_refresh\",\"result\":{{\"body\":\"{body}\",\"state\":{{\"seen\":1}}}}}}}}\\n' \"$id\"\n"
    );
    let command = ProcessBridgeCommand::new("/bin/sh").with_args(["-c", script.as_str()]);
    serde_json::to_string(&command).expect("bridge handler json")
}
