use crate::{
    runtime::{EngineEvent, EventSender, StreamEvent, StreamRequestOverrides},
    stream,
    types::{ChatMessage, MessagePart, Role},
};
use jfc_core::QueuedPrompt;
use crate::app::EngineState;

pub(crate) async fn drain_queued_prompts(state: &mut EngineState, tx: &EventSender) {
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
    let mut first_non_meta_text: Option<String> = None;
    for queued in drained {
        let QueuedPrompt {
            text,
            is_meta,
            attachments,
            ..
        } = queued;
        let glyph = if is_meta { "⚙" } else { "⏳" };
        let placeholder = format!("{glyph} {text}");
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
            if first_non_meta_text.is_none() {
                first_non_meta_text = Some(text.clone());
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

    #[cfg(feature = "intent-gate")]
    if let Some(intent_text) = first_non_meta_text.as_deref() {
        let intent_for_inject = crate::intent::classify(intent_text).intent;
        if crate::intent::is_graph_intent(intent_for_inject) {
            let cwd = std::path::PathBuf::from(&state.cwd);
            let injected = crate::intent::auto_inject_graph_context(
                &mut state.messages,
                intent_for_inject,
                intent_text,
                &cwd,
            );
            if injected {
                tracing::info!(
                    target: "jfc::intent::auto_ctx",
                    intent = ?intent_for_inject,
                    "auto graph-context injected (batched queued-prompt drain)"
                );
            }
        }
    }
    #[cfg(not(feature = "intent-gate"))]
    let _ = first_non_meta_text;

    state.messages.push(ChatMessage::assistant(String::new()));
    state.streaming_text = String::new();
    state.streaming_reasoning = String::new();
    state.streaming_response_bytes = 0;
    state.turn_output_tokens = 0;
    state.refusal_fallback_attempted = false;
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
    state.interrupt_flag
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
    let tx_spawn = tx.clone();
    let interrupt = state.interrupt_flag.clone();
    state.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = state.cancel_token.clone();
    let tx_guard = tx.clone();
    // Refresh CLAUDE.md frontmatter disallowed tools before each turn.
    if let Ok(cwd_path) = std::env::current_dir() {
        let hierarchy = crate::context::ClaudeMdHierarchy::load(&cwd_path);
        state.claudemd_disallowed_tools = hierarchy.collect_disallowed_tools();
    }
    let overrides = StreamRequestOverrides {
        background_reminders: state.take_background_reminders(),
        disallowed_tools: state.effective_disallowed_tools(),
        allowed_tools: state.allowed_tools.clone(),
        custom_betas: state.custom_betas.clone(),
        fine_grained_tool_streaming: state.fine_grained_tool_streaming,
        strict_tool_schemas: state.strict_tool_schemas,
        task_budget: state.cli_task_budget,
        max_thinking_tokens: state.cli_max_thinking_tokens,
        thinking_display: state.cli_thinking_display.clone(),
        brief_mode: state.brief_mode,
        context_hint_tokens_saved: state.take_context_hint_tokens_saved(),
        ..Default::default()
    };
    // Park the *inner* task's abort handle on App so the watchdog can
    // forcefully abort the actual stream_response task (see
    // App::active_stream_handle). Aborting the outer supervisor would only
    // drop its JoinHandle to the inner task, detaching rather than cancelling
    // it.
    let inner = tokio::spawn(async move {
        stream::stream_response(
            provider, messages, model, tx_spawn, interrupt, cancel, None, overrides,
        )
        .await;
    });
    state.active_stream_handle = Some(inner.abort_handle());
    tokio::spawn(async move {
        if let Err(join_err) = inner.await {
            let msg = if join_err.is_panic() {
                format!("stream task panicked: {join_err}")
            } else {
                format!("stream task cancelled: {join_err}")
            };
            let _ = tx_guard
                .send(EngineEvent::Stream(StreamEvent::Error(msg)))
                .await;
        }
    });
}
