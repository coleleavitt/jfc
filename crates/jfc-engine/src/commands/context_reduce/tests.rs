use super::*;
use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
use std::sync::Arc;

struct TestProvider;

impl jfc_provider::seal::Sealed for TestProvider {}

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

#[tokio::test]
async fn ctx_reduce_command_queues_requested_transcript_messages_normal() {
    let mut state = state_with_messages(12);

    cmd_ctx_reduce(&mut state, &["/ctx-reduce", "2-3"], "/ctx-reduce 2-3", None).await;

    assert_eq!(message_text(&state.messages[1]), "msg 2");
    assert_eq!(message_text(&state.messages[2]), "msg 3");
    assert_eq!(state.context_reduction_queue.drops().len(), 1);
    assert_eq!(state.context_reduction_queue.drops()[0].range().start(), 2);
    assert_eq!(state.context_reduction_queue.drops()[0].range().end(), 3);
    assert_eq!(
        message_text(state.messages.last().expect("assistant reply")),
        "ctx_reduce queued 2 drops across 1 range."
    );
}

#[tokio::test]
async fn ctx_reduce_command_defers_recent_tail_robust() {
    let mut state = state_with_messages(12);

    cmd_ctx_reduce(
        &mut state,
        &["/ctx-reduce", "4-12"],
        "/ctx-reduce 4-12",
        None,
    )
    .await;

    assert_eq!(message_text(&state.messages[3]), "msg 4");
    assert_eq!(message_text(&state.messages[5]), "msg 6");
    assert_eq!(message_text(&state.messages[6]), "msg 7");
    assert_eq!(state.context_reduction_queue.drops().len(), 2);
    assert_eq!(state.context_reduction_queue.drops()[0].range().start(), 4);
    assert_eq!(state.context_reduction_queue.drops()[0].range().end(), 6);
    assert_eq!(state.context_reduction_queue.drops()[1].range().start(), 7);
    assert_eq!(state.context_reduction_queue.drops()[1].range().end(), 12);
    let reply = message_text(state.messages.last().expect("assistant reply"));
    assert!(reply.contains("ctx_reduce queued 3 drops across 1 range."));
    assert!(reply.contains("Protected tail deferred 6 drops across 1 range: §7§-§12§."));
}

#[tokio::test]
async fn ctx_reduce_command_rejects_compact_boundary_malformed() {
    let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
    state
        .messages
        .push(ChatMessage::compact_boundary("summary", 123));

    cmd_ctx_reduce(&mut state, &["/ctx-reduce", "1"], "/ctx-reduce 1", None).await;

    assert!(
        message_text(state.messages.last().expect("assistant reply"))
            .contains("context tag is already compacted")
    );
}

fn state_with_messages(count: u32) -> EngineState {
    let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
    state.messages = (1..=count)
        .map(|id| ChatMessage::user(format!("msg {id}")))
        .collect();
    state.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&state.messages);
    state
}

fn message_text(message: &ChatMessage) -> String {
    message
        .parts
        .iter()
        .map(MessagePart::text_only)
        .collect::<String>()
}
