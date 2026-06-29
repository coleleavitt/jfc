use crate::app::EngineState;
use crate::{
    runtime::{EngineEvent, EventSender, StreamRequestOverrides},
    stream,
    types::{ChatMessage, MessagePart, Role},
};
use jfc_core::{QueuedPrompt, queued_prompt_placeholder};

pub async fn drain_queued_prompts(state: &mut EngineState, tx: &EventSender) {
    if state.queued_prompts.is_empty() {
        return;
    }
    if state.queued_prompts.iter().any(|queued| !queued.is_meta)
        && crate::runtime::ops::refuse_budget_cap_if_reached(state)
    {
        return;
    }

    let drained: Vec<QueuedPrompt> = state.queued_prompts.drain_all();
    if drained.is_empty() {
        return;
    }

    let total = drained.len();
    let meta_count = drained.iter().filter(|queued| queued.is_meta).count();
    tracing::info!(
        target: "jfc::ui::queue",
        total,
        meta_count,
        non_meta_count = total - meta_count,
        "drain_queued_prompts: batched drain"
    );

    let mut non_meta_texts: Vec<String> = Vec::with_capacity(total - meta_count);
    for queued in drained {
        let QueuedPrompt {
            text,
            is_meta,
            mut attachments,
            ..
        } = queued;
        let placeholder = queued_prompt_placeholder(&text, is_meta);
        let mut promoted = false;
        for msg in state.messages.iter_mut() {
            if msg.role == Role::User {
                let mut replaced = false;
                for part in msg.parts.iter_mut() {
                    if let MessagePart::Text(part_text) = part
                        && *part_text == placeholder
                    {
                        *part_text = text.clone();
                        replaced = true;
                        break;
                    }
                }
                if replaced {
                    if msg.queued {
                        msg.queued = false;
                    }
                    if !attachments.is_empty() {
                        tracing::info!(
                            target: "jfc::ui::queue",
                            count = attachments.len(),
                            "drain_queued_prompts: attaching images to promoted message"
                        );
                        msg.attachments = std::mem::take(&mut attachments);
                    }
                    promoted = true;
                    break;
                }
            }
        }

        if is_meta {
            crate::runtime::send_critical(
                tx,
                EngineEvent::Control(crate::runtime::ControlEvent::RunCommand(text.clone())),
            );
        } else {
            if !promoted {
                let mut message = ChatMessage::user(text.clone());
                if !attachments.is_empty() {
                    message.attachments = attachments;
                }
                state.messages.push(message);
            }
            non_meta_texts.push(text);
        }
    }

    // Meta (slash) commands run above may themselves enqueue more prompts.
    // Only recurse to drain those when THIS batch produced no stream of its
    // own (i.e. every entry was meta). When we *do* have non-meta text to
    // stream, we stage exactly one combined stream below and intentionally
    // leave any newly-enqueued prompts in the queue — they drain after this
    // stream finishes (stream_done re-drains on EndTurn). Recursing here while
    // also staging our own stream below would start a *second* concurrent
    // stream into the same conversation buffer, clobbering
    // `streaming_assistant_idx` so the live stream's chunks can't attach.
    if non_meta_texts.is_empty() {
        if !state.queued_prompts.is_empty() {
            Box::pin(drain_queued_prompts(state, tx)).await;
        }
        return;
    }

    let assistant_idx = state.messages.len();

    #[cfg(debug_assertions)]
    if let Err(error) = crate::types::validate_turn_invariants_inner(
        &state.messages,
        /* allow_streaming_tail = */ true,
    ) {
        tracing::warn!(
            target: "jfc::ui::queue::invariants",
            error = %error,
            assistant_idx,
            "drain_queued_prompts: turn-invariant violation before staging assistant slot"
        );
    }
    state.tool_ctx.total_user_turns += 1;

    state.messages.push(ChatMessage::assistant(String::new()));
    state.streaming_text = String::new();
    state.streaming_reasoning = String::new();
    state.streaming_response_bytes = 0;
    state.streaming_response_baseline = 0;
    state.streaming_thinking_tokens = 0;
    state.token_rate_samples.clear();
    state.token_rate_sample_thinking = None;
    state.turn_output_tokens = 0;
    state.refusal_fallback_attempted = false;
    state.refusal_resend_count = 0;
    state.refusal_rewrite_retry_count = 0;
    state.refusal_rewrite_attempts.clear();
    state.network_recovery_status = None;
    state.network_recovery_attempts = 0;
    state.stream_lifecycle = None;
    state.streaming_assistant_idx = Some(assistant_idx);
    state.is_streaming = true;
    let now = std::time::Instant::now();
    state.streaming_started_at = Some(now);
    state.last_stream_event_at = Some(now);
    state.streaming_last_token_at = Some(now);
    state.turn_started_at = Some(now);
    state.turn_start_cost = crate::cost::total_cost(&state.usage_by_model);
    state.pending_classifications = 0;
    state.agentic_turn_count = 0;
    // Reset cancel token + interrupt flag for the drained turn. Same
    // rationale as handle_submit — a stale cancel from the previous
    // turn would otherwise fire immediately on this freshly-drained
    // submission and emit "Interrupted by user".
    state.cancel_token = tokio_util::sync::CancellationToken::new();
    state
        .interrupt_flag
        .store(false, std::sync::atomic::Ordering::SeqCst);
    state.last_usage_output = 0;
    state.usage_apply_baseline = (0, 0, 0, 0);
    state.push_effect(crate::app::EngineEffect::ScrollToBottom);

    let provider = state.provider.clone();
    let route_text = non_meta_texts.first().cloned().unwrap_or_default();
    let model = if let Some(ref router) = state.slate {
        router.route(&route_text, state.model.clone())
    } else {
        state.model.clone()
    };
    let context_drain = crate::context_reduction::drain_context_reduction_queue(state);
    let identity = crate::cache_lineage::request_cache_identity(state, provider.name(), &model);
    crate::context_reduction::mark_expected_cache_drop(state, identity.clone(), context_drain);
    crate::cache_lineage::stamp_assistant(&mut state.messages, assistant_idx, &identity);
    let messages = stream::build_provider_messages(&state.messages[..assistant_idx]);
    let interrupt = state.interrupt_flag.clone();
    state.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = state.cancel_token.clone();
    // Refresh CLAUDE.md frontmatter disallowed tools before each turn.
    if let Ok(cwd_path) = std::env::current_dir() {
        let hierarchy =
            crate::context::ClaudeMdHierarchy::load_with_extra_roots(&cwd_path, &state.extra_dirs);
        state.claudemd_disallowed_tools = hierarchy.collect_disallowed_tools();
    }
    let overrides = StreamRequestOverrides {
        session_id: state
            .current_session_id
            .as_ref()
            .map(|s| s.as_str().to_owned()),
        provider_history_archive_seen: state.provider_history_archive_seen(),
        background_reminders: state.take_background_reminders(),
        disallowed_tools: state.effective_disallowed_tools(),
        extra_dirs: state.extra_dirs.clone(),
        allowed_tools: state.allowed_tools.clone(),
        custom_betas: state.custom_betas.clone(),
        fine_grained_tool_streaming: state.fine_grained_tool_streaming,
        strict_tool_schemas: state.strict_tool_schemas,
        task_budget: state.cli_task_budget,
        max_thinking_tokens: state.cli_max_thinking_tokens,
        thinking_display: state.cli_thinking_display.clone(),
        brief_mode: state.brief_mode,
        interaction_mode: state.active_interaction_mode,
        context_hint_tokens_saved: state.take_context_hint_tokens_saved(),
        last_usage_input_tokens: Some(state.last_usage_input as u64),
        context_window_tokens: Some(state.max_context_tokens as u64),
        ..Default::default()
    };
    // Scope stream events so stale provider errors from superseded tasks cannot
    // append duplicate hard-error assistant turns.
    crate::runtime::spawn_stream_response_scoped(
        state, tx, provider, messages, model, interrupt, cancel, None, overrides,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_core::{QueuePriority, QueuedPrompt, queued_prompt_placeholder};
    use std::sync::Arc;

    struct NoopProvider;

    #[async_trait::async_trait]
    impl jfc_provider::Provider for NoopProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for NoopProvider {}

    fn state() -> EngineState {
        EngineState::new(Arc::new(NoopProvider), "test-model")
    }

    fn queued(text: &str) -> QueuedPrompt {
        QueuedPrompt {
            text: text.to_owned(),
            is_meta: false,
            priority: QueuePriority::Later,
            attachments: Vec::new(),
        }
    }

    #[tokio::test]
    async fn drain_appends_user_message_when_placeholder_was_lost_regression() {
        let mut state = state();
        state
            .queued_prompts
            .push(queued("finish the compacted turn"));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        drain_queued_prompts(&mut state, &tx).await;

        assert_eq!(state.messages.len(), 2);
        assert_eq!(state.messages[0].role, Role::User);
        assert!(matches!(
            &state.messages[0].parts[..],
            [MessagePart::Text(text)] if text == "finish the compacted turn"
        ));
        assert_eq!(state.messages[1].role, Role::Assistant);
        assert!(state.is_streaming);
        assert!(state.queued_prompts.is_empty());
    }

    #[tokio::test]
    async fn drain_budget_refusal_keeps_queue_and_placeholder_regression() {
        let mut state = state();
        state.max_budget_usd = Some(1.00);
        state.usage_by_model.insert(
            "claude-opus-4-7".into(),
            crate::types::ModelUsage {
                input_tokens: 1_000_000,
                output_tokens: 0,
                thinking_tokens: 0,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost_usd: None,
            },
        );
        let placeholder = queued_prompt_placeholder("do more", false);
        state
            .messages
            .push(ChatMessage::user_queued(placeholder.clone()));
        state.queued_prompts.push(queued("do more"));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        drain_queued_prompts(&mut state, &tx).await;

        assert_eq!(state.queued_prompts.len(), 1);
        assert_eq!(state.messages.len(), 1);
        assert!(state.messages[0].queued);
        assert!(matches!(
            &state.messages[0].parts[..],
            [MessagePart::Text(text)] if text == &placeholder
        ));
        assert!(!state.is_streaming);
    }
}
