use crate::app::EngineState;
use crate::types::ChatMessage;

mod account;
mod compartments;
mod detected_limit;
mod message_pressure;
mod provider_archive;
mod provider_history;
mod provider_payload;
mod request_pressure;
mod transcript_boundary;

pub use account::{
    CONTRIB_COMPARTMENTS, CONTRIB_CONVERSATION, CONTRIB_DOCS, CONTRIB_MEMORIES, CONTRIB_SYSTEM,
    CONTRIB_TOOL_CALLS, CONTRIB_TOOL_DEFS, build_context_account,
};
pub use compartments::{build_compartment_sequence, compartment_total_tokens};
pub(crate) use detected_limit::{
    DetectedContextLimit, load_session_detected_context_limit, parse_detected_context_limit,
    persist_session_detected_context_limit,
};
pub(crate) use message_pressure::{
    estimate_transcript_tokens, pending_turn_tokens, transcript_visible_chars,
};
pub(crate) use provider_archive::{
    ProviderHistoryArchiveHit, archive_provider_history_current_project,
    list_provider_history_archives, load_session_provider_history_archive_seen,
    persist_session_provider_history_archive_seen, provider_history_archive_recall_block,
    render_provider_history_archive_by_id, search_provider_history_archives,
    search_provider_history_archives_in,
};
pub(crate) use provider_history::{
    ProviderHistoryBudget, ProviderHistoryTransform, compact_provider_history,
    compact_provider_history_with_archive,
};
pub(crate) use provider_payload::{chars_to_tokens, provider_messages_tokens};
pub(crate) use request_pressure::RequestContextPressure;
pub use request_pressure::{ContextPressureNudge, ContextPressureNudgeKind};
pub(crate) use transcript_boundary::{TranscriptBoundaryBudget, materialize_transcript_boundary};

pub fn fallback_request_overhead_tokens() -> usize {
    let budget = jfc_core::context_budget::typical_initial_budget();
    budget
        .system_prompt_tokens
        .saturating_add(budget.tool_definition_tokens)
        .saturating_add(budget.memory_tokens)
        .saturating_add(budget.project_instructions_tokens) as usize
}

pub fn has_usage_backed_messages(messages: &[ChatMessage]) -> bool {
    messages.iter().rev().any(|msg| msg.usage.is_some())
}

pub fn request_overhead_tokens(state: &EngineState) -> usize {
    state
        .last_system_prompt_len
        .unwrap_or_else(fallback_request_overhead_tokens)
}

pub fn model_visible_tokens_for_request(
    state: &EngineState,
    pending_prompt_tokens: usize,
) -> usize {
    let baseline = state
        .tool_ctx
        .approx_tokens
        .saturating_add(pending_prompt_tokens);
    if has_usage_backed_messages(&state.messages) {
        baseline
    } else {
        baseline.saturating_add(request_overhead_tokens(state))
    }
}

pub fn model_visible_tokens_for_display(state: &EngineState) -> usize {
    model_visible_tokens_for_request(state, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::ModelUsage;
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use std::sync::Arc;

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

    fn state() -> EngineState {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state
    }

    #[test]
    fn display_tokens_add_request_overhead_for_no_usage_resume_robust() {
        let mut state = state();
        state.tool_ctx.approx_tokens = 120_000;
        state.last_system_prompt_len = Some(30_000);
        state
            .messages
            .push(ChatMessage::user("legacy resumed prompt".to_owned()));

        assert_eq!(model_visible_tokens_for_display(&state), 150_000);
    }

    #[test]
    fn request_tokens_add_pending_prompt_after_overhead_normal() {
        let mut state = state();
        state.tool_ctx.approx_tokens = 120_000;
        state.last_system_prompt_len = Some(30_000);

        assert_eq!(model_visible_tokens_for_request(&state, 5_000), 155_000);
    }

    #[test]
    fn usage_backed_tokens_do_not_add_request_overhead_normal() {
        let mut state = state();
        state.tool_ctx.approx_tokens = 120_000;
        state.last_system_prompt_len = Some(30_000);
        let mut assistant = ChatMessage::assistant("usage-backed".to_owned());
        assistant.usage = Some(ModelUsage {
            input_tokens: 120_000,
            output_tokens: 0,
            thinking_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            cost_usd: None,
        });
        state.messages.push(assistant);

        assert_eq!(model_visible_tokens_for_display(&state), 120_000);
    }
}
