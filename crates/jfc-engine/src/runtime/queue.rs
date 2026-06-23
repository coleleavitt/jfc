use crate::app::EngineState;
use crate::{
    runtime::{EngineEvent, EventSender, StreamRequestOverrides},
    stream,
    types::{ChatMessage, MessagePart, Role},
};
use jfc_core::{QueuedPrompt, queued_prompt_placeholder};

pub async fn drain_queued_prompts(state: &mut EngineState, tx: &EventSender) {
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
            attachments,
            ..
        } = queued;
        let placeholder = queued_prompt_placeholder(&text, is_meta);
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
                        msg.attachments = attachments;
                    }
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
    if crate::runtime::ops::refuse_budget_cap_if_reached(state) {
        return;
    }

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
    let messages = stream::build_provider_messages(&state.messages[..assistant_idx]);
    let route_text = non_meta_texts.first().cloned().unwrap_or_default();
    let model = if let Some(ref router) = state.slate {
        router.route(&route_text, state.model.clone())
    } else {
        state.model.clone()
    };
    let identity = crate::cache_lineage::request_cache_identity(state, provider.name(), &model);
    crate::cache_lineage::stamp_assistant(&mut state.messages, assistant_idx, &identity);
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
