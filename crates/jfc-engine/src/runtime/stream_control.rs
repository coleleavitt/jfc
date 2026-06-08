use crate::{
    app::EngineState,
    runtime::{EngineEvent, EventSender, StreamEvent, StreamRequestOverrides},
    stream,
    types::{MessagePart, Role},
};

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

    let msg = state
        .messages
        .get_mut(assistant_idx)
        .expect("validated above");
    msg.parts = vec![MessagePart::Text(String::new())];
    msg.model_name = None;
    msg.cost_tier = None;
    msg.elapsed = None;
    msg.usage = None;

    state.streaming_text = String::new();
    state.streaming_reasoning = String::new();
    state.streaming_response_bytes = 0;
    state.turn_output_tokens = 0;
    state.refusal_fallback_attempted = false;
    state.streaming_thinking_tokens = 0;
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
    let messages = stream::build_provider_messages(&state.messages[..assistant_idx]);
    let model = state.model.clone();
    let tx_spawn = tx.clone();
    let interrupt = state.interrupt_flag.clone();
    interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
    state.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = state.cancel_token.clone();
    let prev_msg_id = state.last_response_id.take();
    let tx_guard = tx.clone();
    // Track the *inner* task's abort handle so the watchdog can forcefully
    // abort the actual stream task if it gets stuck in a blocking syscall.
    // Aborting the outer supervisor would only drop its JoinHandle to the
    // inner task, detaching rather than cancelling it.
    let inner = tokio::spawn(async move {
        stream::stream_response(
            provider,
            messages,
            model,
            tx_spawn,
            interrupt,
            cancel,
            prev_msg_id,
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
                .send(EngineEvent::Stream(StreamEvent::Error(msg)))
                .await;
        }
    });
}
