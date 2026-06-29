use super::*;

use std::sync::Arc;

use jfc_core::TaskLifecycle;
use jfc_engine::swarm::{BackendType, TeammateInfo};
use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
use ratatui::{Terminal, backend::TestBackend};

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

fn background_task(
    status: TaskLifecycle,
    task_id: &str,
    description: &str,
) -> crate::app::BackgroundTask {
    crate::app::BackgroundTask {
        task_id: task_id.into(),
        description: description.into(),
        status,
        started_at: std::time::Instant::now(),
        completed_at: status.is_terminal().then(std::time::Instant::now),
        summary: None,
        error: None,
        last_tool: None,
        last_tool_info: None,
        recent_activities: Vec::new(),
        messages: Vec::new(),
        chat_messages: Vec::new(),
        tool_use_count: 0,
        latest_input_tokens: 0,
        latest_cache_read_tokens: 0,
        latest_cache_write_tokens: 0,
        cumulative_output_tokens: 0,
        model_used: None,
        agent_messages: Vec::new(),
        max_input_tokens: None,
        budget_killed: false,
        parent_task_id: None,
        workflow_progress: None,
        last_activity_at: std::time::Instant::now(),
    }
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

#[test]
fn team_section_uses_background_lifecycle_over_abort_handle_regression() {
    let mut app = App::new(Arc::new(TestProvider), "test-model");
    app.engine.task_store = jfc_session::TaskStore::in_memory();
    app.engine.team_context.team_name = Some("alpha".to_owned());
    let agent_id = "alice@alpha";
    let task_id = jfc_engine::swarm::runner::teammate_task_id(agent_id);
    app.engine.background_tasks.insert(
        task_id.clone(),
        background_task(TaskLifecycle::Completed, &task_id, "alice completed work"),
    );

    let (abort_tx, _abort_rx) = tokio::sync::watch::channel(false);
    app.engine.team_context.teammates.insert(
        agent_id.to_owned(),
        TeammateInfo {
            name: "alice".to_owned(),
            agent_type: Some("explore".to_owned()),
            color: None,
            cwd: "/tmp".to_owned(),
            spawned_at: std::time::Instant::now(),
            backend: BackendType::InProcess,
            abort_tx: Some(abort_tx),
        },
    );

    let backend = TestBackend::new(90, 24);
    // SAFE-EXPECT: render test setup should fail loudly if ratatui backend construction fails.
    let mut term = Terminal::new(backend).expect("terminal");
    let draw_result = term.draw(|f| super::teammates_panel::teammates_panel(f, &mut app));
    // SAFE-EXPECT: render test should fail loudly if drawing the panel fails.
    draw_result.expect("draw");

    let text = buffer_text(&term);
    assert!(
        text.contains("alice completed work"),
        "task row missing:\n{text}"
    );
    assert!(
        text.contains("alice  completed"),
        "team row should use completed lifecycle:\n{text}"
    );
    assert!(
        !text.contains("alice  running"),
        "abort handle must not make a completed teammate look running:\n{text}"
    );
}
