use crate::{
    app::App,
    runtime::{AppEvent, EventSender, StreamEvent},
    stream,
    types::{MessagePart, Role},
};

pub(crate) fn restart_stream_in_place(
    app: &mut App,
    tx: &EventSender,
    assistant_idx: usize,
    turn_started_at: Option<std::time::Instant>,
) {
    let Some(msg) = app.messages.get_mut(assistant_idx) else {
        return;
    };
    if msg.role != Role::Assistant {
        return;
    }

    msg.parts = vec![MessagePart::Text(String::new())];
    msg.model_name = None;
    msg.cost_tier = None;
    msg.elapsed = None;
    msg.usage = None;

    app.streaming_text = String::new();
    app.streaming_reasoning = String::new();
    app.streaming_response_bytes = 0;
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.last_stream_event_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.turn_started_at = turn_started_at.or(Some(now));
    app.thinking_started_at = None;
    app.thinking_ended_at = None;
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);
    app.scroll_to_bottom();

    let provider = app.provider.clone();
    let messages = stream::build_provider_messages(&app.messages[..assistant_idx]);
    let model = app.model.clone();
    let tx_spawn = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    interrupt.store(false, std::sync::atomic::Ordering::SeqCst);
    app.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = app.cancel_token.clone();
    let tx_guard = tx.clone();
    tokio::spawn(async move {
        let result = tokio::spawn(async move {
            stream::stream_response(provider, messages, model, tx_spawn, interrupt, cancel).await;
        })
        .await;
        if let Err(join_err) = result {
            let msg = if join_err.is_panic() {
                format!("stream task panicked: {join_err}")
            } else {
                format!("stream task cancelled: {join_err}")
            };
            let _ = tx_guard.send(AppEvent::Stream(StreamEvent::Error(msg)));
        }
    });
}
