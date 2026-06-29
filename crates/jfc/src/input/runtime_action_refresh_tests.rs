use std::sync::Arc;

use super::runtime_action_router::execute_runtime_action_for_label;
use crate::app::App;
use crate::runtime::EngineEvent;
use jfc_plugin_sdk::{PluginId, RuntimeActionDescriptor, RuntimeActionKind};
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
async fn refresh_metrics_runtime_action_refreshes_plugin_state_normal() {
    let mut app = test_app();
    let plugin_id = PluginId::new("plugin.palette");
    app.plugins.last_refresh_at = None;
    app.plugins.runtime_action_descriptors = vec![RuntimeActionDescriptor::new(
        plugin_id,
        "metrics.refresh",
        "Refresh Metrics",
        "Refresh plugin metric state",
        RuntimeActionKind::RefreshMetrics,
    )];
    let tx = channel();

    assert!(execute_runtime_action_for_label(&mut app, "Refresh Metrics", &tx).await);

    assert!(app.plugins.last_refresh_at.is_some());
}

#[tokio::test]
async fn refresh_prompt_context_runtime_action_refreshes_plugin_state_normal() {
    let mut app = test_app();
    let plugin_id = PluginId::new("plugin.palette");
    app.plugins.last_refresh_at = None;
    app.plugins.runtime_action_descriptors = vec![RuntimeActionDescriptor::new(
        plugin_id,
        "context.refresh",
        "Refresh Prompt Context",
        "Refresh prompt-context descriptors",
        RuntimeActionKind::RefreshPromptContext,
    )];
    let tx = channel();

    assert!(execute_runtime_action_for_label(&mut app, "Refresh Prompt Context", &tx).await);

    assert!(app.plugins.last_refresh_at.is_some());
}

#[tokio::test]
async fn plugin_diagnostics_runtime_action_refreshes_and_reports_summary_normal() {
    let mut app = test_app();
    let project = tempfile::TempDir::new().expect("temp project");
    app.engine.cwd = project.path().display().to_string();
    let plugin_id = PluginId::new("builtin.jfc-ux");
    app.plugins.last_refresh_at = None;
    app.plugins.runtime_action_descriptors = vec![RuntimeActionDescriptor::new(
        plugin_id,
        "command_palette.plugin_diagnostics",
        "Run Plugin Diagnostics",
        "Refresh plugin descriptors and run smoke checks",
        RuntimeActionKind::PluginDiagnostics,
    )];
    let tx = channel();

    assert!(execute_runtime_action_for_label(&mut app, "Run Plugin Diagnostics", &tx).await);

    assert!(app.plugins.last_refresh_at.is_some());
    assert!(app.engine.toasts.iter().any(|toast| {
        toast.text.contains("Plugin diagnostics")
            && toast.text.contains("descriptor issues")
            && toast.text.contains("smoke checks")
    }));
}
