use crate::{
    app::EngineState,
    runtime::{
        EngineEvent, EventSender, StreamEvent, StreamRequestOverrides, scoped_stream_sender,
    },
    stream,
    types::{MessagePart, Role},
};
use std::sync::Arc;

use jfc_provider::{ModelId, Provider, ProviderMessage};

pub fn spawn_stream_response_scoped(
    state: &mut EngineState,
    tx: &EventSender,
    provider: Arc<dyn Provider>,
    messages: Vec<ProviderMessage>,
    model: ModelId,
    interrupt: Arc<std::sync::atomic::AtomicBool>,
    cancel: tokio_util::sync::CancellationToken,
    previous_message_id: Option<String>,
    overrides: StreamRequestOverrides,
) {
    let previous_message_id = previous_message_id
        .or_else(|| crate::cache_lineage::previous_response_id_for(state, provider.name(), &model));
    let stream_id = state.begin_stream_scope();
    let tx_stream = scoped_stream_sender(tx.clone(), stream_id);
    let tx_guard = tx.clone();
    let inner = tokio::spawn(async move {
        stream::stream_response(
            provider,
            messages,
            model,
            tx_stream,
            interrupt,
            cancel,
            previous_message_id,
            overrides,
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
                .send(EngineEvent::ScopedStream {
                    stream_id,
                    event: StreamEvent::Error(msg),
                })
                .await;
        }
    });
}

pub fn restart_stream_in_place(
    state: &mut EngineState,
    tx: &EventSender,
    assistant_idx: usize,
    turn_started_at: Option<std::time::Instant>,
) {
    restart_stream_in_place_with_overrides(
        state,
        tx,
        assistant_idx,
        turn_started_at,
        StreamRequestOverrides::default(),
    );
}

pub fn restart_stream_in_place_with_overrides(
    state: &mut EngineState,
    tx: &EventSender,
    assistant_idx: usize,
    turn_started_at: Option<std::time::Instant>,
    mut overrides: StreamRequestOverrides,
) {
    // Validate the assistant slot first so an aborted restart doesn't
    // silently drain the background-reminder queue. Without this the
    // reminders would be lost — they'd be moved into a discarded
    // `overrides` rather than carried forward to the next attempt.
    match state.messages.get(assistant_idx) {
        Some(msg) if msg.role == Role::Assistant => {}
        _ => return,
    }
    // Caller may have already populated `overrides.background_reminders`
    // (e.g. a future caller-supplied override) — extend rather than
    // replace so caller-supplied entries survive.
    overrides
        .background_reminders
        .extend(state.take_background_reminders());
    if overrides.disallowed_tools.is_empty() {
        overrides.disallowed_tools = state.effective_disallowed_tools();
    }
    if overrides.allowed_tools.is_empty() {
        overrides.allowed_tools = state.allowed_tools.clone();
    }
    if overrides.custom_betas.is_empty() {
        overrides.custom_betas = state.custom_betas.clone();
    }
    overrides.fine_grained_tool_streaming |= state.fine_grained_tool_streaming;
    overrides.strict_tool_schemas |= state.strict_tool_schemas;
    if overrides.task_budget.is_none() {
        overrides.task_budget = state.cli_task_budget;
    }
    if overrides.max_thinking_tokens.is_none() {
        overrides.max_thinking_tokens = state.cli_max_thinking_tokens;
    }
    if overrides.thinking_display.is_none() {
        overrides.thinking_display = state.cli_thinking_display.clone();
    }
    if overrides.last_usage_input_tokens.is_none() {
        overrides.last_usage_input_tokens = Some(state.last_usage_input as u64);
    }
    if overrides.context_window_tokens.is_none() {
        overrides.context_window_tokens = Some(state.max_context_tokens as u64);
    }

    let msg = state
        .messages
        .get_mut(assistant_idx)
        .expect("validated above");
    // Preserve tool calls that already EXECUTED in the failed turn. Wiping
    // them (the old behavior) erased Edits/Writes/Bash that had really run
    // from both the transcript and the provider rebuild, so the retried
    // model re-issued the same calls — the "duplicate write" bug: the same
    // append landing twice, then a cleanup turn. Unexecuted (pending) tools
    // are dropped; the retry decides whether to issue them again.
    let executed_tools: Vec<MessagePart> = msg
        .parts
        .iter()
        .filter(|p| {
            matches!(
                p,
                MessagePart::Tool(tc) if matches!(
                    tc.status,
                    crate::types::ToolStatus::Completed | crate::types::ToolStatus::Failed
                )
            )
        })
        .cloned()
        .collect();
    let has_executed_tools = !executed_tools.is_empty();
    msg.parts = executed_tools;
    msg.parts.push(MessagePart::Text(String::new()));
    msg.model_name = None;
    msg.cost_tier = None;
    msg.elapsed = None;
    msg.usage = None;

    state.streaming_text = String::new();
    state.streaming_reasoning = String::new();
    state.streaming_response_bytes = 0;
    state.turn_output_tokens = 0;
    state.refusal_fallback_attempted = false;
    state.refusal_resend_count = 0;
    state.refusal_rewrite_retry_count = 0;
    state.refusal_rewrite_attempts.clear();
    state.streaming_thinking_tokens = 0;
    state.last_thinking_estimate = 0;
    state.streaming_assistant_idx = Some(assistant_idx);
    state.is_streaming = true;
    let now = std::time::Instant::now();
    state.streaming_started_at = Some(now);
    state.last_stream_event_at = Some(now);
    state.streaming_last_token_at = Some(now);
    // Fresh rate window for the new turn; seed a zero-token sample at t=0 so
    // the first real sample has a baseline to measure throughput against.
    state.token_rate_samples.clear();
    state
        .token_rate_samples
        .push_back((std::time::Duration::ZERO, 0));
    state.turn_started_at = turn_started_at.or(Some(now));
    state.thinking_started_at = None;
    state.thinking_ended_at = None;
    state.last_usage_output = 0;
    state.usage_apply_baseline = (0, 0, 0, 0);
    state.current_stream_request = None;
    state.stream_lifecycle = None;
    state.push_effect(crate::app::EngineEffect::ScrollToBottom);

    let provider = state.provider.clone();
    // When the failed turn already executed tools, the retried request must
    // include the partial assistant message: its tool_use/tool_result pairs
    // tell the model those calls ALREADY ran. Excluding it (old behavior)
    // made the retry re-issue the same Edits/Bash — observed as duplicate
    // appends to files across stream retries.
    let slice_end = if has_executed_tools {
        assistant_idx + 1
    } else {
        assistant_idx
    };
    let messages = stream::build_provider_messages(&state.messages[..slice_end]);
    let model = state.model.clone();
    let identity = crate::cache_lineage::request_cache_identity(state, provider.name(), &model);
    crate::cache_lineage::stamp_assistant(&mut state.messages, assistant_idx, &identity);
    let interrupt = state.interrupt_flag.clone();
    interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
    state.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = state.cancel_token.clone();
    spawn_stream_response_scoped(
        state, tx, provider, messages, model, interrupt, cancel, None, overrides,
    );
}
