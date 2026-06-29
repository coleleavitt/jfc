use std::sync::Arc;

use crate::app::App;
use jfc_plugin_sdk::{PluginId, UiMutationScope, UiWidgetDescriptor, UiWidgetKind};
use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
use ratatui::{Terminal, backend::TestBackend, layout::Rect};

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

#[test]
fn task_panel_renders_task_scope_plugin_widgets_normal() {
    let mut app = test_app();
    app.plugins.ui_widget_descriptors = vec![
        UiWidgetDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::TaskPanel,
            "task.widget",
            "Task Widget",
            UiWidgetKind::Text,
        )
        .with_body("task body"),
    ];
    let backend = TestBackend::new(100, 30);
    let mut term = Terminal::new(backend).expect("terminal");

    term.draw(|f| super::task_panel::task_panel(f, &mut app))
        .expect("draw");
    let text = buffer_text(&term);

    assert!(text.contains("Plugin widgets"), "missing section:\n{text}");
    assert!(text.contains("Task Widget"), "missing widget:\n{text}");
    assert!(text.contains("task body"), "missing widget body:\n{text}");
}

#[test]
fn session_sidebar_renders_session_scope_plugin_widgets_normal() {
    let mut app = test_app();
    app.plugins.ui_widget_descriptors = vec![
        UiWidgetDescriptor::new(
            PluginId::new("demo"),
            UiMutationScope::SessionSidebar,
            "session.widget",
            "Session Widget",
            UiWidgetKind::Action,
        )
        .with_runtime_action("session.open"),
    ];
    let backend = TestBackend::new(80, 20);
    let mut term = Terminal::new(backend).expect("terminal");

    term.draw(|f| super::session_sidebar::sidebar(f, &mut app, Rect::new(0, 0, 80, 20)))
        .expect("draw");
    let text = buffer_text(&term);

    assert!(text.contains("Plugin widgets"), "missing section:\n{text}");
    assert!(text.contains("Session Widget"), "missing widget:\n{text}");
    assert!(
        text.contains("action session.open"),
        "missing action:\n{text}"
    );
}

fn test_app() -> App {
    let mut app = App::new(Arc::new(TestProvider), "test-model");
    app.engine.task_store = jfc_session::TaskStore::in_memory();
    app
}

fn buffer_text(term: &Terminal<TestBackend>) -> String {
    let buf = term.backend().buffer();
    let area = buf.area();
    let mut out = String::new();
    for y in 0..area.height {
        for x in 0..area.width {
            out.push_str(buf[(x, y)].symbol());
        }
        out.push('\n');
    }
    out
}
