use std::sync::Arc;

use super::runtime_action_router::execute_runtime_action_for_label;
use super::runtime_action_smoke::plugin_smoke_target;
use crate::app::App;
use crate::runtime::EngineEvent;
use jfc_plugin_sdk::{
    PluginId, RuntimeActionDescriptor, RuntimeActionKind, UiMutationScope, UiPanelDescriptor,
    UiWidgetDescriptor, UiWidgetKind,
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

fn test_app() -> App {
    let mut app = App::new(Arc::new(TestProvider), "test-model");
    app.engine.task_store = jfc_session::TaskStore::in_memory();
    app
}

fn channel() -> tokio::sync::mpsc::Sender<EngineEvent> {
    let (tx, _rx) = tokio::sync::mpsc::channel(4);
    tx
}

#[tokio::test]
async fn open_panel_runtime_action_focuses_info_sidebar_widget_normal() {
    let mut app = test_app();
    app.info_sidebar.visible = false;
    let plugin_id = PluginId::new("plugin.palette");
    app.plugins.ui_widget_descriptors =
        vec![review_queue_widget(plugin_id.clone()).with_runtime_action("widget.run")];
    app.plugins.runtime_action_descriptors =
        vec![open_widget_action(plugin_id, "Plugin Widget", false)];
    let tx = channel();

    assert!(execute_runtime_action_for_label(&mut app, "Plugin Widget", &tx).await);

    assert!(app.info_sidebar.visible);
    let focus = app.info_sidebar.focused_widget.as_ref().unwrap();
    assert_eq!(focus.plugin_id, "plugin.palette");
    assert_eq!(focus.widget_id, "review.queue");
}

#[tokio::test]
async fn open_panel_runtime_action_can_execute_focused_widget_action_normal() {
    let mut app = test_app();
    app.info_sidebar.visible = false;
    let plugin_id = PluginId::new("plugin.palette");
    app.plugins.ui_widget_descriptors =
        vec![review_queue_widget(plugin_id.clone()).with_runtime_action("widget.toggle")];
    app.plugins.runtime_action_descriptors = vec![
        open_widget_action(plugin_id.clone(), "Run Plugin Widget", true),
        RuntimeActionDescriptor::new(
            plugin_id,
            "widget.toggle",
            "Toggle Info Sidebar",
            "Host action exposed through a focused widget",
            RuntimeActionKind::HostAction,
        )
        .with_payload(serde_json::json!({ "action": "toggle_info_sidebar" })),
    ];
    let tx = channel();

    assert!(execute_runtime_action_for_label(&mut app, "Run Plugin Widget", &tx).await);

    assert!(!app.info_sidebar.visible);
    let focus = app.info_sidebar.focused_widget.as_ref().unwrap();
    assert_eq!(focus.plugin_id, "plugin.palette");
    assert_eq!(focus.widget_id, "review.queue");
}

#[tokio::test]
async fn open_panel_runtime_action_focuses_info_sidebar_panel_normal() {
    let mut app = test_app();
    app.info_sidebar.visible = false;
    let plugin_id = PluginId::new("plugin.palette");
    app.plugins.ui_panel_descriptors = vec![review_panel(plugin_id.clone())];
    app.plugins.runtime_action_descriptors =
        vec![open_panel_action(plugin_id, "Plugin Panel", false)];
    let tx = channel();

    assert!(execute_runtime_action_for_label(&mut app, "Plugin Panel", &tx).await);

    assert!(app.info_sidebar.visible);
    let focus = app.info_sidebar.focused_panel.as_ref().unwrap();
    assert_eq!(focus.plugin_id, "plugin.palette");
    assert_eq!(focus.panel_id, "review.panel");
    assert!(app.info_sidebar.focused_widget.is_none());
}

#[tokio::test]
async fn open_panel_runtime_action_can_execute_focused_panel_action_normal() {
    let mut app = test_app();
    app.info_sidebar.visible = false;
    let plugin_id = PluginId::new("plugin.palette");
    app.plugins.ui_panel_descriptors =
        vec![review_panel(plugin_id.clone()).with_runtime_action("panel.toggle")];
    app.plugins.runtime_action_descriptors = vec![
        open_panel_action(plugin_id.clone(), "Run Plugin Panel", true),
        RuntimeActionDescriptor::new(
            plugin_id,
            "panel.toggle",
            "Toggle Info Sidebar",
            "Host action exposed through a focused panel",
            RuntimeActionKind::HostAction,
        )
        .with_payload(serde_json::json!({ "action": "toggle_info_sidebar" })),
    ];
    let tx = channel();

    assert!(execute_runtime_action_for_label(&mut app, "Run Plugin Panel", &tx).await);

    assert!(!app.info_sidebar.visible);
    let focus = app.info_sidebar.focused_panel.as_ref().unwrap();
    assert_eq!(focus.plugin_id, "plugin.palette");
    assert_eq!(focus.panel_id, "review.panel");
}

#[test]
fn plugin_smoke_runtime_action_defaults_to_descriptor_plugin_normal() {
    let action = RuntimeActionDescriptor::new(
        PluginId::new("plugin.palette"),
        "plugin.smoke",
        "Smoke Plugin",
        "Run process-bridge smoke checks",
        RuntimeActionKind::PluginSmoke,
    );

    let target = plugin_smoke_target(&action);

    assert_eq!(target, "plugin.palette");
}

#[test]
fn plugin_smoke_runtime_action_accepts_payload_target_normal() {
    let action = RuntimeActionDescriptor::new(
        PluginId::new("builtin.plugin-management"),
        "plugin.smoke",
        "Smoke Plugin",
        "Run process-bridge smoke checks",
        RuntimeActionKind::PluginSmoke,
    )
    .with_payload(serde_json::json!({ "plugin": "demo-tool" }));

    let target = plugin_smoke_target(&action);

    assert_eq!(target, "demo-tool");
}

fn review_queue_widget(plugin_id: PluginId) -> UiWidgetDescriptor {
    UiWidgetDescriptor::new(
        plugin_id,
        UiMutationScope::InfoSidebar,
        "review.queue",
        "Review Queue",
        UiWidgetKind::Action,
    )
}

fn review_panel(plugin_id: PluginId) -> UiPanelDescriptor {
    UiPanelDescriptor::new(
        plugin_id,
        UiMutationScope::InfoSidebar,
        "review.panel",
        "Review Panel",
    )
}

fn open_widget_action(
    plugin_id: PluginId,
    label: &'static str,
    execute_widget_action: bool,
) -> RuntimeActionDescriptor {
    RuntimeActionDescriptor::new(
        plugin_id,
        "widget.open",
        label,
        "Open and focus the widget",
        RuntimeActionKind::OpenPanel,
    )
    .with_payload(serde_json::json!({
        "panel": "info_sidebar",
        "widget_id": "review.queue",
        "execute_widget_action": execute_widget_action
    }))
}

fn open_panel_action(
    plugin_id: PluginId,
    label: &'static str,
    execute_panel_action: bool,
) -> RuntimeActionDescriptor {
    RuntimeActionDescriptor::new(
        plugin_id,
        "panel.open",
        label,
        "Open and focus the panel",
        RuntimeActionKind::OpenPanel,
    )
    .with_payload(serde_json::json!({
        "panel": "info_sidebar",
        "panel_id": "review.panel",
        "execute_panel_action": execute_panel_action
    }))
}
