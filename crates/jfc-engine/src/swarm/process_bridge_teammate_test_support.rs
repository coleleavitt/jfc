use std::sync::Arc;

use async_trait::async_trait;
use jfc_core::TaskInput;
use jfc_provider::{
    CompletionResponse, EventStream, ModelInfo, Provider, ProviderMessage as PMsg,
    StreamConvention, StreamOptions as SOpts,
};
use tokio::sync::mpsc;

use super::runner::TeammateEvent;

pub(super) struct NamedProvider {
    pub(super) name: &'static str,
}

impl NamedProvider {
    pub(super) fn arc(name: &'static str) -> Arc<dyn Provider> {
        Arc::new(Self { name }) as Arc<dyn Provider>
    }
}

#[async_trait]
impl Provider for NamedProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::AnthropicNative
    }

    async fn stream(&self, _m: Vec<PMsg>, _o: &SOpts) -> anyhow::Result<EventStream> {
        anyhow::bail!("unused")
    }

    async fn complete(&self, _m: Vec<PMsg>, _o: &SOpts) -> anyhow::Result<CompletionResponse> {
        anyhow::bail!("unused")
    }
}

impl jfc_provider::seal::Sealed for NamedProvider {}

pub(super) async fn collect_events_until_terminal(
    teammate_rx: &mut mpsc::UnboundedReceiver<TeammateEvent>,
) -> Vec<TeammateEvent> {
    let mut events = Vec::new();
    for _ in 0..8 {
        let event = timeout_teammate(teammate_rx).await;
        let done = matches!(
            event,
            TeammateEvent::Completed { .. }
                | TeammateEvent::Cancelled { .. }
                | TeammateEvent::Failed { .. }
        );
        events.push(event);
        if done {
            break;
        }
    }
    assert!(
        events.last().is_some_and(|event| matches!(
            event,
            TeammateEvent::Completed { .. }
                | TeammateEvent::Cancelled { .. }
                | TeammateEvent::Failed { .. }
        )),
        "teammate stream did not reach a terminal event: {events:?}"
    );
    events
}

pub(super) fn teammate_task_input() -> TaskInput {
    TaskInput {
        description: "spawn teammate".to_owned(),
        prompt: "inspect".to_owned(),
        subagent_type: None,
        category: None,
        run_in_background: false,
        model: None,
        launcher: None,
        effort: None,
        name: Some("reviewer".to_owned()),
        team_name: Some("alpha".to_owned()),
        mode: None,
        isolation: None,
        parent_task_id: None,
        schema: None,
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        cwd: None,
    }
}

async fn timeout_teammate(
    teammate_rx: &mut mpsc::UnboundedReceiver<TeammateEvent>,
) -> TeammateEvent {
    tokio::time::timeout(std::time::Duration::from_secs(2), teammate_rx.recv())
        .await
        .expect("teammate event")
        .expect("teammate channel")
}
