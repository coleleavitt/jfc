use super::*;
use crate::{
    app::EngineState,
    context_accounting::{ContextPressureNudge, ContextPressureNudgeKind},
    types::{ChatMessage, MessagePart},
};
use jfc_context::{ContextDropRange, ContextDropReplayMode, QueuedContextDrop};
use jfc_provider::{
    EventStream, ModelInfo, Provider, ProviderContent, ProviderMessage, StreamOptions,
};
use std::sync::{Arc, Mutex};

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

struct CaptureProvider {
    tx: Mutex<Option<tokio::sync::oneshot::Sender<Vec<ProviderMessage>>>>,
}

impl CaptureProvider {
    fn new(tx: tokio::sync::oneshot::Sender<Vec<ProviderMessage>>) -> Self {
        Self {
            tx: Mutex::new(Some(tx)),
        }
    }
}

impl jfc_provider::seal::Sealed for CaptureProvider {}

#[async_trait::async_trait]
impl Provider for CaptureProvider {
    fn name(&self) -> &str {
        "capture"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    async fn stream(
        &self,
        messages: Vec<ProviderMessage>,
        _options: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        if let Some(tx) = self.tx.lock().expect("capture mutex").take() {
            let _ = tx.send(messages);
        }
        Ok(Box::pin(futures::stream::empty()))
    }
}

#[test]
fn drain_context_reduction_queue_applies_ready_ranges_normal() {
    let mut state = state_with_messages(8);
    state
        .context_reduction_queue
        .extend([QueuedContextDrop::new(
            ContextDropRange::new(2, 3).expect("valid range"),
            ContextDropReplayMode::Full,
        )
        .expect("valid drop")]);

    let drain = drain_context_reduction_queue(&mut state);

    assert_eq!(drain.applied, 2);
    assert_eq!(drain.deferred, 0);
    assert!(state.context_reduction_queue.is_empty());
    assert_eq!(message_text(&state.messages[1]), "[dropped §2§]");
    assert_eq!(message_text(&state.messages[2]), "[dropped §3§]");
}

#[test]
fn drain_context_reduction_queue_defers_protected_tail_then_applies_normal() {
    let mut state = state_with_messages(8);
    state
        .context_reduction_queue
        .extend([QueuedContextDrop::protected_tail_skip(
            ContextDropRange::new(6, 8).expect("valid range"),
            6,
        )
        .expect("valid protected skip")]);

    let first = drain_context_reduction_queue(&mut state);

    assert_eq!(first.applied, 0);
    assert_eq!(first.deferred, 1);
    assert_eq!(message_text(&state.messages[5]), "msg 6");
    assert_eq!(state.context_reduction_queue.drops().len(), 1);

    state
        .messages
        .extend((9..=14).map(|id| ChatMessage::user(format!("msg {id}"))));
    let second = drain_context_reduction_queue(&mut state);

    assert_eq!(second.applied, 3);
    assert_eq!(second.deferred, 0);
    assert!(state.context_reduction_queue.is_empty());
    assert_eq!(message_text(&state.messages[5]), "[dropped §6§]");
    assert_eq!(message_text(&state.messages[7]), "[dropped §8§]");
}

#[tokio::test]
async fn start_turn_drains_context_reduction_queue_before_request_normal() {
    let (capture_tx, capture_rx) = tokio::sync::oneshot::channel();
    let provider = Arc::new(CaptureProvider::new(capture_tx));
    let mut state = EngineState::new(provider, "test-model");
    state.messages = (1..=8)
        .map(|id| ChatMessage::user(format!("msg {id}")))
        .collect();
    state.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&state.messages);
    state
        .context_reduction_queue
        .extend([QueuedContextDrop::new(
            ContextDropRange::new(2, 2).expect("valid range"),
            ContextDropReplayMode::Full,
        )
        .expect("valid drop")]);
    let (tx, _rx) = tokio::sync::mpsc::channel(8);

    crate::runtime::ops::start_turn_from_transcript(&mut state, &tx, "next").await;

    let captured = tokio::time::timeout(std::time::Duration::from_secs(1), capture_rx)
        .await
        .expect("provider stream called")
        .expect("captured provider messages");
    assert!(state.context_reduction_queue.is_empty());
    assert_eq!(message_text(&state.messages[1]), "[dropped §2§]");
    assert!(
        provider_text(&captured).contains("[dropped §2§]"),
        "provider messages should contain drained marker"
    );
    assert!(state.prompt_cache_expected_drop.is_some());
}

#[test]
fn pressure_reduction_ignores_channel_one_normal() {
    let mut state = state_with_messages(10);

    let queued = queue_pressure_reduction(
        &mut state,
        pressure_nudge(ContextPressureNudgeKind::ChannelOne, 3),
    );

    assert_eq!(queued, None);
    assert!(state.context_reduction_queue.is_empty());
}

#[test]
fn pressure_reduction_queues_oldest_active_outside_tail_normal() {
    let mut state = state_with_messages(10);

    let queued = queue_pressure_reduction(
        &mut state,
        pressure_nudge(ContextPressureNudgeKind::ChannelTwo, 3),
    )
    .expect("channel-two pressure should queue drops");

    assert_eq!(queued.queued_tags, 3);
    assert_eq!(queued.queued_ranges, 1);
    assert_eq!(queued.estimated_reclaim_tokens, 3);
    let drops = state.context_reduction_queue.drops();
    assert_eq!(drops.len(), 1);
    assert_eq!(drops[0].range().start(), 1);
    assert_eq!(drops[0].range().end(), 3);
}

fn state_with_messages(count: u32) -> EngineState {
    let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
    state.messages = (1..=count)
        .map(|id| ChatMessage::user(format!("msg {id}")))
        .collect();
    state.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&state.messages);
    state
}

fn pressure_nudge(
    kind: ContextPressureNudgeKind,
    reclaim_floor_tokens: u64,
) -> ContextPressureNudge {
    ContextPressureNudge {
        kind,
        level: crate::compact::CompactLevel::Compact,
        raw_tokens: 187_000,
        effective_tokens: 280_500,
        window_tokens: 200_000,
        threshold_tokens: 178_808,
        reclaim_floor_tokens,
    }
}

fn message_text(message: &ChatMessage) -> String {
    message
        .parts
        .iter()
        .map(MessagePart::text_only)
        .collect::<String>()
}

fn provider_text(messages: &[ProviderMessage]) -> String {
    messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|content| match content {
            ProviderContent::Text(text) => Some(text.as_str()),
            ProviderContent::Thinking { .. }
            | ProviderContent::ToolResult { .. }
            | ProviderContent::ToolUse { .. }
            | ProviderContent::ServerToolUse { .. }
            | ProviderContent::ServerToolResult { .. }
            | ProviderContent::Attachment(_)
            | ProviderContent::RedactedThinking { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}
