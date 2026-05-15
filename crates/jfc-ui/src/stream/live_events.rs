use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::provider::{EventStream, StopReason, StreamEvent};
use crate::runtime::{AppEvent, StreamEvent as RuntimeStreamEvent};
use crate::types::{ToolCall, ToolInput, ToolKind, ToolOutput};

const STREAM_INTERRUPT_POLL: Duration = Duration::from_millis(50);

pub(super) async fn drain_stream_events(
    mut stream: EventStream,
    tx: &mpsc::Sender<AppEvent>,
    interrupt: Arc<std::sync::atomic::AtomicBool>,
    cancel: CancellationToken,
) -> Option<StopReason> {
    let mut stop_reason = StopReason::EndTurn;
    let mut tool_accum: HashMap<usize, (String, String, String)> = HashMap::new();

    loop {
        // Cooperative cancel: the user pressed ESC twice. The legacy atomic
        // flag covers older callers; the CancellationToken gives immediate
        // wakeups for the migrated stream/task paths.
        if interrupt.load(std::sync::atomic::Ordering::SeqCst) || cancel.is_cancelled() {
            tracing::info!(target: "jfc::stream", "stream interrupted by user (ESCx2)");
            let _ = tx
                .send(AppEvent::Stream(RuntimeStreamEvent::Error(
                    "Interrupted by user".to_owned(),
                )))
                .await;
            return None;
        }

        let event = tokio::select! {
            biased;
            // Race SSE reads against cancellation so a stalled provider
            // does not trap the user in "Interrupting..." until the next
            // interrupt poll.
            _ = cancel.cancelled() => {
                tracing::info!(target: "jfc::stream", "stream cancelled via token");
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::Error(
                        "Interrupted by user".to_owned(),
                    )))
                    .await;
                return None;
            }
            _ = tokio::time::sleep(STREAM_INTERRUPT_POLL) => continue,
            event = stream.next() => event,
        };

        let Some(event) = event else {
            break;
        };

        let event = match event {
            Ok(e) => e,
            Err(e) => {
                tracing::error!(target: "jfc::stream", error = %e, "stream event error");
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::Error(e.to_string())))
                    .await;
                return None;
            }
        };

        match event {
            StreamEvent::TextDelta { delta, .. } => {
                // Send delta directly. The AppEvent channel is bounded;
                // try_send drops if full, which already provides back-pressure.
                if tx
                    .try_send(AppEvent::Stream(RuntimeStreamEvent::Chunk {
                        text: Some(delta),
                        reasoning: None,
                    }))
                    .is_err()
                {
                    tracing::trace!(target: "jfc::stream", "StreamChunk dropped (buffer full)");
                }
            }
            StreamEvent::ThinkingDelta { delta, .. } => {
                if tx
                    .try_send(AppEvent::Stream(RuntimeStreamEvent::Chunk {
                        text: None,
                        reasoning: Some(delta),
                    }))
                    .is_err()
                {
                    tracing::trace!(target: "jfc::stream", "StreamChunk(thinking) dropped (buffer full)");
                }
            }
            StreamEvent::ToolDelta { index, delta } => {
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
            } => {
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
                let tool = match parse_outcome {
                    Ok(input_val) => match ToolInput::from_value(&tool_name, input_val) {
                        Ok(parsed) => ToolCall::new_pending(id, kind, parsed),
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
                    }
                };
                tool_accum.remove(&index);

                // Server-side tools are executed by Anthropic's infrastructure.
                // JFC should surface them as completed records, not dispatch them.
                let tool = if matches!(
                    tool.kind,
                    ToolKind::ServerWebSearch | ToolKind::ServerCodeExecution
                ) {
                    let mut t = tool;
                    let display_text = match t.kind {
                        ToolKind::ServerWebSearch => format!(
                            "🔍 Executed server-side by Anthropic ({})",
                            t.input.summary()
                        ),
                        ToolKind::ServerCodeExecution => format!(
                            "⚡ Executed server-side by Anthropic ({})",
                            t.input.summary()
                        ),
                        _ => unreachable!(),
                    };
                    t.output = ToolOutput::Text(display_text);
                    let _ = t.mark_running();
                    let _ = t.mark_completed();
                    tracing::info!(
                        target: "jfc::stream",
                        tool_kind = t.kind.label(),
                        tool_use_id = %tool_use_id,
                        "server-side tool marked completed (no local dispatch)"
                    );
                    t
                } else {
                    tool
                };
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::Tool(tool)))
                    .await;
            }
            StreamEvent::Done { stop_reason: r } => {
                // Never downgrade from ToolUse to EndTurn. Some providers
                // emit Done(ToolUse) and then a final Done(EndTurn).
                tracing::debug!(
                    target: "jfc::stream",
                    incoming = ?r, current = ?stop_reason,
                    "StreamEvent::Done"
                );
                if stop_reason != StopReason::ToolUse {
                    stop_reason = r;
                }
            }
            StreamEvent::ResponseMetadata { response_id: _ } => {
                // Response ids are intentionally ignored. JFC sends full
                // history each turn and does not use server-side chaining.
            }
            StreamEvent::TextDone { .. } | StreamEvent::ThinkingDone { .. } => {}
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
                let _ = tx
                    .send(AppEvent::Stream(RuntimeStreamEvent::Error(message)))
                    .await;
                return None;
            }
        }
    }

    Some(stop_reason)
}
