use crate::{
    app::{App, QueuedPrompt},
    input,
    runtime::{AppEvent, EventSender, StreamEvent},
    stream,
    types::{ChatMessage, MessagePart, Role},
};

pub(crate) async fn drain_queued_prompts(app: &mut App, tx: &EventSender) {
    let drained: Vec<QueuedPrompt> = app.queued_prompts.drain(..).collect();
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
        } = queued;
        let glyph = if is_meta { "⚙" } else { "⏳" };
        let placeholder = format!("{glyph} {text}");
        for msg in app.messages.iter_mut() {
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
            input::run_slash_command(app, &text).await;
        } else {
            if first_non_meta_text.is_none() {
                first_non_meta_text = Some(text.clone());
            }
            non_meta_texts.push(text);
        }
    }

    if !app.queued_prompts.is_empty() {
        Box::pin(drain_queued_prompts(app, tx)).await;
    }

    if non_meta_texts.is_empty() {
        return;
    }

    let assistant_idx = app.messages.len();
    #[cfg(debug_assertions)]
    if let Err(error) = crate::types::validate_turn_invariants_inner(
        &app.messages,
        /* allow_streaming_tail = */ true,
    ) {
        tracing::warn!(
            target: "jfc::ui::queue::invariants",
            error = %error,
            assistant_idx,
            "drain_queued_prompts: turn-invariant violation before staging assistant slot"
        );
    }
    app.tool_ctx.total_user_turns += 1;

    #[cfg(feature = "intent-gate")]
    if let Some(intent_text) = first_non_meta_text.as_deref() {
        let intent_for_inject = crate::intent::classify(intent_text).intent;
        if crate::intent::is_graph_intent(intent_for_inject) {
            let cwd = std::path::PathBuf::from(&app.cwd);
            let injected = crate::intent::auto_inject_graph_context(
                &mut app.messages,
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

    app.messages.push(ChatMessage::assistant(String::new()));
    app.streaming_text = String::new();
    app.streaming_reasoning = String::new();
    app.streaming_response_bytes = 0;
    app.network_recovery_status = None;
    app.network_recovery_attempts = 0;
    app.streaming_assistant_idx = Some(assistant_idx);
    app.is_streaming = true;
    let now = std::time::Instant::now();
    app.streaming_started_at = Some(now);
    app.last_stream_event_at = Some(now);
    app.streaming_last_token_at = Some(now);
    app.turn_started_at = Some(now);
    app.last_usage_output = 0;
    app.usage_apply_baseline = (0, 0, 0, 0);
    app.scroll_to_bottom();

    let provider = app.provider.clone();
    let messages = stream::build_provider_messages(&app.messages[..assistant_idx]);
    let route_text = non_meta_texts.first().cloned().unwrap_or_default();
    let model = if let Some(ref router) = app.slate {
        router.route(&route_text, app.model.clone())
    } else {
        app.model.clone()
    };
    let tx_spawn = tx.clone();
    let interrupt = app.interrupt_flag.clone();
    app.cancel_token = tokio_util::sync::CancellationToken::new();
    let cancel = app.cancel_token.clone();
    let tx_guard = tx.clone();
    tokio::spawn(async move {
        let result = tokio::spawn(async move {
            stream::stream_response(provider, messages, model, tx_spawn, interrupt, cancel, None).await;
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
