use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::runtime::{AppEvent, StreamEvent as RuntimeStreamEvent};
use crate::types::{ToolCall, ToolInput, ToolKind, ToolOutput};
use jfc_provider::{EventStream, StopReason, StreamEvent};

const STREAM_INTERRUPT_POLL: Duration = Duration::from_millis(50);
const TERMINAL_DONE_GRACE: Duration = Duration::from_secs(2);

pub(super) enum DrainOutcome {
    Done(StopReason),
    Cancelled(String),
    Error {
        message: String,
        committed_output: bool,
    },
}

/// Build the user-facing error text for a cancelled stream. The user-abort
/// path sets the interrupt flag; the watchdog only cancels the token. Without
/// this split a watchdog timeout shows up as "Interrupted by user", making a
/// hard-idle stream look like a phantom keypress.
fn cancel_reason(by_user: bool) -> String {
    if by_user {
        "Interrupted by user".to_owned()
    } else {
        "Stream timed out — the model stopped sending data and the watchdog \
         cancelled it. Press Ctrl+R to retry."
            .to_owned()
    }
}

pub(super) async fn drain_stream_events(
    mut stream: EventStream,
    tx: &mpsc::Sender<AppEvent>,
    interrupt: Arc<std::sync::atomic::AtomicBool>,
    cancel: CancellationToken,
) -> DrainOutcome {
    let mut stop_reason = StopReason::EndTurn;
    let mut tool_accum: HashMap<usize, (String, String, String)> = HashMap::new();
    let mut terminal_done_deadline: Option<tokio::time::Instant> = None;
    let mut saw_terminal_done = false;
    let mut committed_output = false;

    loop {
        // Cooperative cancel: the user pressed ESC twice. The legacy atomic
        // flag covers older callers; the CancellationToken gives immediate
        // wakeups for the migrated stream/task paths.
        if interrupt.load(std::sync::atomic::Ordering::SeqCst) || cancel.is_cancelled() {
            let by_user = interrupt.load(std::sync::atomic::Ordering::SeqCst);
            tracing::info!(target: "jfc::stream", by_user, "stream cancelled");
            return DrainOutcome::Cancelled(cancel_reason(by_user));
        }

        let event = tokio::select! {
            biased;
            // Race SSE reads against cancellation so a stalled provider
            // does not trap the user in "Interrupting..." until the next
            // interrupt poll.
            _ = cancel.cancelled() => {
                // Distinguish a real user abort (ESC×2 / interrupt-on-submit
                // set the interrupt flag) from a watchdog timeout, which only
                // cancels the token. Mislabeling watchdog kills as user
                // interrupts made hard-idle streams look like random ESCs.
                let by_user = interrupt.load(std::sync::atomic::Ordering::SeqCst);
                tracing::info!(target: "jfc::stream", by_user, "stream cancelled via token");
                return DrainOutcome::Cancelled(cancel_reason(by_user));
            }
            _ = async {
                if let Some(deadline) = terminal_done_deadline {
                    tokio::time::sleep_until(deadline).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                tracing::warn!(
                    target: "jfc::stream",
                    ?stop_reason,
                    grace_ms = TERMINAL_DONE_GRACE.as_millis() as u64,
                    "stream terminal Done grace elapsed before EOF; finalizing turn"
                );
                break;
            }
            _ = tokio::time::sleep(STREAM_INTERRUPT_POLL) => continue,
            event = stream.next() => event,
        };

        let Some(event) = event else {
            if !saw_terminal_done {
                tracing::error!(
                    target: "jfc::stream",
                    committed_output,
                    "provider stream ended before StreamEvent::Done"
                );
                return DrainOutcome::Error {
                    message: "Provider stream ended before `message_stop`; the response may be incomplete. Press Ctrl+R to retry.".to_owned(),
                    committed_output,
                };
            }
            break;
        };

        let event = match event {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(target: "jfc::stream", error = %e, "stream event error");
                return DrainOutcome::Error {
                    message: e.to_string(),
                    committed_output,
                };
            }
        };

        match event {
            StreamEvent::TextDelta { delta, .. } => {
                committed_output = true;
                // MUST use blocking send for text — try_send drops data on
                // backpressure, causing permanent text loss in the assistant
                // message. Blocking send applies backpressure to the SSE
                // reader instead (slows it down until the event loop catches
                // up). TextDelta is the model's output — we cannot lose it.
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::Chunk {
                        text: Some(delta),
                        reasoning: None,
                    }))
                    .await;
            }
            StreamEvent::ThinkingDelta {
                delta,
                estimated_tokens,
                ..
            } => {
                committed_output = true;
                // Same rationale as TextDelta — thinking text is displayed
                // in the UI and losing chunks creates gaps in the reasoning
                // trace.
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::Chunk {
                        text: None,
                        reasoning: Some(delta),
                    }))
                    .await;
                // Emit server-authoritative thinking token estimate if available.
                // Matches cli.js pattern of separate "thinking_tokens" system events.
                if let Some(tokens) = estimated_tokens
                    && tx
                        .try_send(AppEvent::Stream(RuntimeStreamEvent::ThinkingTokens(tokens)))
                        .is_err()
                {
                    tracing::trace!(target: "jfc::stream", "ThinkingTokens dropped (buffer full)");
                }
            }
            StreamEvent::ToolDelta { index, delta } => {
                committed_output = true;
                let byte_len = delta.len();
                tool_accum.entry(index).or_default().2.push_str(&delta);
                // Keep spinner byte estimate and stall timer live while
                // providers stream input_json_delta fragments.
                if tx
                    .try_send(AppEvent::Stream(RuntimeStreamEvent::ToolInputDelta(
                        byte_len,
                    )))
                    .is_err()
                {
                    tracing::trace!(target: "jfc::stream", "ToolInputDelta dropped (buffer full)");
                }
            }
            StreamEvent::ToolDone {
                index,
                tool_name,
                tool_use_id,
                input_json,
                thought_signature,
            } => {
                committed_output = true;
                let assembled = if input_json.is_empty() {
                    tool_accum
                        .get(&index)
                        .map(|(_, _, buf)| buf.clone())
                        .unwrap_or_default()
                } else {
                    input_json
                };
                tracing::debug!(
                    target: "jfc::stream",
                    index,
                    tool_name = %tool_name,
                    tool_use_id = %tool_use_id,
                    input_len = assembled.len(),
                    "tool_done"
                );

                let parse_outcome: Result<serde_json::Value, _> = if assembled.trim().is_empty() {
                    Ok(serde_json::Value::Object(serde_json::Map::new()))
                } else {
                    serde_json::from_str(&assembled)
                };
                let kind = ToolKind::from_name(&tool_name);
                let make_stub = || ToolInput::Generic {
                    summary: if assembled.is_empty() {
                        format!("(empty input for {tool_name})")
                    } else {
                        assembled.clone()
                    },
                };
                let id = crate::ids::ToolId::from(tool_use_id.clone());
                let signature = thought_signature.clone();
                let tool = match parse_outcome {
                    Ok(input_val) => match ToolInput::from_value(&tool_name, input_val) {
                        Ok(parsed) => ToolCall::new_pending(id, kind, parsed)
                            .with_thought_signature(signature.clone()),
                        Err(err) => {
                            tracing::warn!(
                                target: "jfc::stream",
                                tool_name = %tool_name,
                                tool_use_id = %tool_use_id,
                                input_len = assembled.len(),
                                error = %err,
                                "tool_done: input shape validation failed - failing tool"
                            );
                            let msg = format!(
                                "{err}\n\n\
                                 The tool input was valid JSON but didn't match the \
                                 tool's required schema. Retry with the correct fields."
                            );
                            ToolCall::new_failed(id, kind, make_stub(), ToolOutput::Text(msg))
                                .with_thought_signature(signature.clone())
                        }
                    },
                    Err(err) => {
                        tracing::warn!(
                            target: "jfc::stream",
                            tool_name = %tool_name,
                            tool_use_id = %tool_use_id,
                            input_len = assembled.len(),
                            error = %err,
                            "tool_done: input JSON parse failed - failing tool"
                        );
                        let msg = format!(
                            "Tool input was not valid JSON ({} bytes received): {}\n\n\
                             The provider stream finished before sending a complete \
                             `input` object. Retry the tool call with a properly-formed \
                             JSON input.",
                            assembled.len(),
                            err,
                        );
                        ToolCall::new_failed(id, kind, make_stub(), ToolOutput::Text(msg))
                            .with_thought_signature(signature.clone())
                    }
                };
                tool_accum.remove(&index);

                // Server-side tools are executed by Anthropic's infrastructure.
                // JFC should surface them as records with the result attached
                // when it arrives (via StreamEvent::ServerToolResult below),
                // not dispatch them locally. The output stays `Empty` here so
                // the matching ServerToolResult event can fill it in with the
                // real content — fabricating a "🔍 Executed server-side"
                // placeholder used to make the resend path lossy because
                // `tool_result_content` then turned the placeholder into a
                // synthetic user `tool_result` block that broke Anthropic's
                // server-side sampling loop resumption. See cli.js v142:7057.
                let tool = if matches!(
                    tool.kind,
                    ToolKind::ServerWebSearch
                        | ToolKind::ServerCodeExecution
                        | ToolKind::ServerAdvisor
                ) {
                    let mut t = tool;
                    // Leave output `Empty` — populated by the matching
                    // StreamEvent::ServerToolResult event below.
                    t.output = ToolOutput::Empty;
                    let _ = t.mark_running();
                    // NOTE: do NOT mark_completed yet. The matching
                    // ServerToolResult event will flip status to Completed
                    // when the result arrives. If the stream ends with a
                    // PauseTurn before the result block, the tool stays
                    // Running and `pause_turn` resume sees the original
                    // server_tool_use block on the wire — exactly the cue
                    // Anthropic uses to resume the loop (cli.js v142:622686).
                    tracing::info!(
                        target: "jfc::stream",
                        tool_kind = t.kind.label(),
                        tool_use_id = %tool_use_id,
                        "server-side tool registered (awaiting result block)"
                    );
                    if matches!(t.kind, ToolKind::ServerAdvisor) {
                        tracing::info!(
                            target: "jfc::advisor",
                            tool_use_id = %tool_use_id,
                            "tengu_advisor_tool_call"
                        );
                    }
                    t
                } else {
                    tool
                };
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::Tool(Box::new(tool))))
                    .await;
            }
            StreamEvent::ServerToolResult {
                tool_use_id,
                tool_kind,
                content,
            } => {
                committed_output = true;
                // Anthropic emitted the paired result for a previously-
                // dispatched server_tool_use block. Forward to the
                // event_loop, which finds the matching ToolCall on the
                // streaming assistant message and replaces its output
                // with ToolOutput::ServerToolResult so the result
                // round-trips byte-faithfully on the next resend.
                tracing::info!(
                    target: "jfc::stream",
                    tool_use_id = %tool_use_id,
                    wire_type = tool_kind.wire_type(),
                    "stream server_tool_result received"
                );
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::ServerToolResult {
                        tool_use_id: crate::ids::ToolId::from(tool_use_id),
                        tool_kind,
                        content,
                    }))
                    .await;
            }
            StreamEvent::Done { stop_reason: r } => {
                // Never downgrade from ToolUse or PauseTurn to EndTurn.
                // Some providers emit Done(ToolUse) followed by a final
                // Done(EndTurn); a server-side-tool resume signals
                // Done(PauseTurn) and must not be overwritten by a later
                // EndTurn from a synthetic stream close. Both states are
                // "loop must continue" — surface them faithfully so the
                // event_loop dispatches the right branch.
                tracing::debug!(
                    target: "jfc::stream",
                    incoming = ?r, current = ?stop_reason,
                    "StreamEvent::Done"
                );
                saw_terminal_done = true;
                if !matches!(stop_reason, StopReason::ToolUse | StopReason::PauseTurn) {
                    stop_reason = r;
                }
                terminal_done_deadline
                    .get_or_insert_with(|| tokio::time::Instant::now() + TERMINAL_DONE_GRACE);
            }
            StreamEvent::ResponseMetadata {
                response_id,
                input_tokens,
            } => {
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::ResponseId(
                        response_id,
                    )))
                    .await;
                // Feed early input-token count so context estimates are
                // available even if the stream aborts before message_delta.
                if let Some(tokens) = input_tokens {
                    let _ = tx
                        .send(AppEvent::Stream(RuntimeStreamEvent::Usage {
                            input_tokens: tokens as u32,
                            output_tokens: 0,
                            cache_read_tokens: 0,
                            cache_write_tokens: 0,
                        }))
                        .await;
                }
            }
            StreamEvent::TextDone { .. } | StreamEvent::ThinkingDone { .. } => {}
            StreamEvent::RedactedThinkingDone { data, .. } => {
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::RedactedThinking(data)))
                    .await;
            }
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => {
                tracing::info!(
                    target: "jfc::stream",
                    input_tokens, output_tokens,
                    cache_read_tokens, cache_write_tokens,
                    "stream usage report"
                );
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::Usage {
                        input_tokens,
                        output_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                    }))
                    .await;
            }
            StreamEvent::Error { message } => {
                tracing::error!(target: "jfc::stream", %message, "stream error event");
                return DrainOutcome::Error {
                    message,
                    committed_output,
                };
            }
            StreamEvent::FallbackTriggered(info) => {
                tracing::info!(
                    target: "jfc::stream",
                    original = %info.original_model,
                    fallback = %info.fallback_model,
                    reason = %info.reason,
                    "model fallback triggered"
                );
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::FallbackTriggered {
                        original_model: info.original_model.to_string(),
                        fallback_model: info.fallback_model.to_string(),
                        reason: info.reason,
                    }))
                    .await;
            }
        }
    }

    DrainOutcome::Done(stop_reason)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn event_stream(events: Vec<anyhow::Result<StreamEvent>>) -> EventStream {
        Box::pin(futures::stream::iter(events))
    }

    #[tokio::test]
    async fn eof_without_done_is_error() {
        let (tx, mut rx) = mpsc::channel(8);
        let outcome = drain_stream_events(
            event_stream(vec![Ok(StreamEvent::TextDelta {
                index: 0,
                delta: "partial".to_owned(),
            })]),
            &tx,
            Arc::new(AtomicBool::new(false)),
            CancellationToken::new(),
        )
        .await;

        match outcome {
            DrainOutcome::Error {
                message,
                committed_output,
            } => {
                assert!(committed_output);
                assert!(message.contains("message_stop"), "{message}");
            }
            _ => panic!("expected incomplete EOF to be an error"),
        }

        match rx.try_recv() {
            Ok(AppEvent::Stream(RuntimeStreamEvent::Chunk {
                text: Some(text), ..
            })) => assert_eq!(text, "partial"),
            Ok(_) => panic!("expected forwarded text chunk, got different AppEvent"),
            Err(err) => panic!("expected forwarded text chunk, got receive error: {err}"),
        }
    }

    #[tokio::test]
    async fn eof_after_done_is_clean() {
        let (tx, _rx) = mpsc::channel(8);
        let outcome = drain_stream_events(
            event_stream(vec![Ok(StreamEvent::Done {
                stop_reason: StopReason::EndTurn,
            })]),
            &tx,
            Arc::new(AtomicBool::new(false)),
            CancellationToken::new(),
        )
        .await;

        match outcome {
            DrainOutcome::Done(reason) => assert_eq!(reason, StopReason::EndTurn),
            _ => panic!("expected Done after terminal stream event"),
        }
    }
}
