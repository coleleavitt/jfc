use std::{collections::HashMap, sync::Arc, time::Duration};

use futures::StreamExt;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::runtime::{EngineEvent, StreamEvent as RuntimeStreamEvent};
use crate::types::{ToolCall, ToolInput, ToolKind, ToolOutput};
use jfc_provider::{EventStream, StopReason, StreamEvent};

const STREAM_INTERRUPT_POLL: Duration = Duration::from_millis(50);
const TERMINAL_DONE_GRACE: Duration = Duration::from_secs(2);
const STREAM_VISIBLE_BATCH_LATENCY: Duration = Duration::from_millis(8);
const STREAM_VISIBLE_BATCH_MAX_BYTES: usize = 512;
const STREAM_VISIBLE_BATCH_MAX_EVENTS: usize = 16;

pub enum DrainOutcome {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PendingVisibleKind {
    Text,
    Reasoning,
}

#[derive(Debug, Default)]
struct PendingVisibleChunk {
    kind: Option<PendingVisibleKind>,
    body: String,
    events: usize,
    first_seen: Option<tokio::time::Instant>,
}

impl PendingVisibleChunk {
    fn kind(&self) -> Option<PendingVisibleKind> {
        self.kind
    }

    fn push(&mut self, kind: PendingVisibleKind, delta: String, now: tokio::time::Instant) {
        debug_assert!(self.kind.is_none() || self.kind == Some(kind));
        if self.kind.is_none() {
            self.kind = Some(kind);
            self.first_seen = Some(now);
        }
        self.body.push_str(&delta);
        self.events = self.events.saturating_add(1);
    }

    fn deadline(&self) -> Option<tokio::time::Instant> {
        self.first_seen.map(|t| t + STREAM_VISIBLE_BATCH_LATENCY)
    }

    fn should_flush(&self) -> bool {
        self.body.len() >= STREAM_VISIBLE_BATCH_MAX_BYTES
            || self.events >= STREAM_VISIBLE_BATCH_MAX_EVENTS
    }

    async fn flush(&mut self, tx: &mpsc::Sender<EngineEvent>) {
        let Some(kind) = self.kind.take() else {
            return;
        };
        let body = std::mem::take(&mut self.body);
        self.events = 0;
        self.first_seen = None;
        if body.is_empty() {
            return;
        }
        let event = match kind {
            PendingVisibleKind::Text => EngineEvent::Stream(RuntimeStreamEvent::Chunk {
                text: Some(body),
                reasoning: None,
            }),
            PendingVisibleKind::Reasoning => EngineEvent::Stream(RuntimeStreamEvent::Chunk {
                text: None,
                reasoning: Some(body),
            }),
        };
        let _ = tx.send(event).await;
    }
}

pub async fn drain_stream_events(
    mut stream: EventStream,
    tx: &mpsc::Sender<EngineEvent>,
    interrupt: Arc<std::sync::atomic::AtomicBool>,
    cancel: CancellationToken,
) -> DrainOutcome {
    let _linkscope_drain = linkscope::phase("stream.drain");
    let _linkscope_drain_trace = linkscope::trace("stream.drain");
    let mut stop_reason = StopReason::EndTurn;
    let mut tool_accum: HashMap<usize, (String, String, String)> = HashMap::new();
    let mut terminal_done_deadline: Option<tokio::time::Instant> = None;
    let mut saw_terminal_done = false;
    let mut committed_output = false;
    let mut sent_first_visible_delta = false;
    let mut pending_visible = PendingVisibleChunk::default();
    // Resumable-stream snapshot: mint a resume entry for this turn and feed text
    // deltas into it so a dropped connection can replay the partial answer.
    let resume = crate::stream::resume::DrainResumeHandle::begin();

    loop {
        // Cooperative cancel: the user pressed ESC twice. The legacy atomic
        // flag covers older callers; the CancellationToken gives immediate
        // wakeups for the migrated stream/task paths.
        if interrupt.load(std::sync::atomic::Ordering::SeqCst) || cancel.is_cancelled() {
            pending_visible.flush(tx).await;
            let by_user = interrupt.load(std::sync::atomic::Ordering::SeqCst);
            tracing::info!(target: "jfc::stream", by_user, "stream cancelled");
            return DrainOutcome::Cancelled(cancel_reason(by_user));
        }

        let visible_flush_deadline = pending_visible.deadline();
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
                pending_visible.flush(tx).await;
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
                pending_visible.flush(tx).await;
                break;
            }
            _ = async move {
                if let Some(deadline) = visible_flush_deadline {
                    tokio::time::sleep_until(deadline).await;
                } else {
                    std::future::pending::<()>().await;
                }
            } => {
                pending_visible.flush(tx).await;
                continue;
            }
            _ = tokio::time::sleep(STREAM_INTERRUPT_POLL) => continue,
            event = stream.next() => event,
        };

        let Some(event) = event else {
            if !saw_terminal_done {
                pending_visible.flush(tx).await;
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
            pending_visible.flush(tx).await;
            break;
        };

        let event = match event {
            Ok(e) => e,
            Err(e) => {
                pending_visible.flush(tx).await;
                tracing::error!(target: "jfc::stream", error = %e, "stream event error");
                return DrainOutcome::Error {
                    message: e.to_string(),
                    committed_output,
                };
            }
        };

        // Canonical frame-category telemetry: one provider-neutral classification
        // per frame, regardless of which backend produced it. `commits_output`
        // is the same predicate the per-arm `committed_output = true` writes
        // below encode; tracing it here makes the cross-provider frame taxonomy
        // observable in one place.
        tracing::trace!(
            target: "jfc::stream::frame",
            category = ?event.category(),
            commits_output = event.commits_output(),
            "stream frame"
        );
        linkscope::record_items("stream.events", 1);
        linkscope::detail_event_fields(
            "stream.event",
            [
                linkscope::TraceField::text("category", format!("{:?}", event.category())),
                linkscope::TraceField::count(
                    "commits_output",
                    if event.commits_output() { 1 } else { 0 },
                ),
            ],
        );

        match event {
            StreamEvent::TextDelta { delta, .. } => {
                committed_output = true;
                linkscope::record_items("stream.text_delta", 1);
                linkscope::record_bytes("stream.text_delta", usize_to_u64_saturating(delta.len()));
                resume.record(&delta);
                // MUST use blocking send for text — try_send drops data on
                // backpressure, causing permanent text loss in the assistant
                // message. Blocking send applies backpressure to the SSE
                // reader instead (slows it down until the event loop catches
                // up). TextDelta is the model's output — we cannot lose it.
                if pending_visible.kind() == Some(PendingVisibleKind::Reasoning) {
                    pending_visible.flush(tx).await;
                }
                if !sent_first_visible_delta {
                    let _ = tx
                        .send(EngineEvent::Stream(RuntimeStreamEvent::Chunk {
                            text: Some(delta),
                            reasoning: None,
                        }))
                        .await;
                    sent_first_visible_delta = true;
                } else {
                    pending_visible.push(
                        PendingVisibleKind::Text,
                        delta,
                        tokio::time::Instant::now(),
                    );
                    if pending_visible.should_flush() {
                        pending_visible.flush(tx).await;
                    }
                }
            }
            StreamEvent::ThinkingDelta {
                delta,
                estimated_tokens,
                ..
            } => {
                committed_output = true;
                linkscope::record_items("stream.thinking_delta", 1);
                linkscope::record_bytes(
                    "stream.thinking_delta",
                    usize_to_u64_saturating(delta.len()),
                );
                // Same rationale as TextDelta — thinking text is displayed
                // in the UI and losing chunks creates gaps in the reasoning
                // trace.
                if pending_visible.kind() == Some(PendingVisibleKind::Text) {
                    pending_visible.flush(tx).await;
                }
                if !sent_first_visible_delta {
                    let _ = tx
                        .send(EngineEvent::Stream(RuntimeStreamEvent::Chunk {
                            text: None,
                            reasoning: Some(delta),
                        }))
                        .await;
                    sent_first_visible_delta = true;
                } else {
                    pending_visible.push(
                        PendingVisibleKind::Reasoning,
                        delta,
                        tokio::time::Instant::now(),
                    );
                    if pending_visible.should_flush() {
                        pending_visible.flush(tx).await;
                    }
                }
                if let Some(tokens) = estimated_tokens {
                    let _ = tx
                        .send(EngineEvent::Stream(RuntimeStreamEvent::ThinkingTokens(
                            tokens,
                        )))
                        .await;
                }
            }
            StreamEvent::ThinkingTokens { delta, .. } => {
                let _ = tx
                    .send(EngineEvent::Stream(RuntimeStreamEvent::ThinkingTokens(
                        delta,
                    )))
                    .await;
            }
            StreamEvent::ToolDelta { index, delta } => {
                committed_output = true;
                linkscope::record_items("stream.tool_delta", 1);
                linkscope::record_bytes("stream.tool_delta", usize_to_u64_saturating(delta.len()));
                pending_visible.flush(tx).await;
                tool_accum.entry(index).or_default().2.push_str(&delta);
                // MUST use blocking send — `try_send` drops on backpressure,
                // and during a large tool-input / file write the
                // `input_json_delta` stream is the ONLY event flowing. Dropping
                // it (a) corrupts the spinner's byte estimate and (b) — far
                // worse — starves `last_stream_event_at`, which the engine's
                // stream watchdog (`check_stream_watchdog`) keys off. A dropped
                // delta means the watchdog sees no activity even though bytes
                // are pouring in, so it false-cancels an actively-streaming
                // response after the idle window ("writing a big file and it
                // just cancels"). Tool-input JSON is model output; like
                // TextDelta/ThinkingDelta it must not be lost. Blocking send
                // applies backpressure to the SSE reader instead of dropping.
                let _ = tx
                    .send(EngineEvent::Stream(RuntimeStreamEvent::ToolInputDelta {
                        index,
                        delta,
                    }))
                    .await;
            }
            StreamEvent::ToolDone {
                index,
                tool_name,
                tool_use_id,
                input_json,
                thought_signature,
            } => {
                committed_output = true;
                pending_visible.flush(tx).await;
                let assembled = if input_json.is_empty() {
                    tool_accum
                        .get(&index)
                        .map(|(_, _, buf)| buf.clone())
                        .unwrap_or_default()
                } else {
                    input_json
                };
                linkscope::record_items("stream.tool_done", 1);
                linkscope::record_bytes(
                    "stream.tool_done",
                    usize_to_u64_saturating(assembled.len()),
                );
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
                    Ok(input_val) => match ToolInput::from_value_coerced(&tool_name, input_val) {
                        (Ok(parsed), outcome) => {
                            // CC 2.1.170 `tool_input_coerced`: malformed args were
                            // repaired to the schema rather than hard-failing.
                            if let jfc_core::CoercionOutcome::Coerced { .. } = &outcome {
                                tracing::info!(
                                    target: "jfc::stream",
                                    tool_name = %tool_name,
                                    tool_use_id = %tool_use_id,
                                    outcome = outcome.label(),
                                    shape = %outcome.shape_class(),
                                    "tool_input_coerced: repaired malformed tool args to schema"
                                );
                            }
                            ToolCall::new_pending(id, kind, parsed)
                                .with_thought_signature(signature.clone())
                        }
                        (Err(err), _outcome) => {
                            tracing::warn!(
                                target: "jfc::stream",
                                tool_name = %tool_name,
                                tool_use_id = %tool_use_id,
                                input_len = assembled.len(),
                                error = %err,
                                "tool_done: input shape validation failed (uncoercible) - failing tool"
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
                    .send(EngineEvent::Stream(RuntimeStreamEvent::Tool(Box::new(
                        tool,
                    ))))
                    .await;
            }
            StreamEvent::ServerToolResult {
                tool_use_id,
                tool_kind,
                content,
            } => {
                committed_output = true;
                pending_visible.flush(tx).await;
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
                    .send(EngineEvent::Stream(RuntimeStreamEvent::ServerToolResult {
                        tool_use_id: crate::ids::ToolId::from(tool_use_id),
                        tool_kind,
                        content,
                    }))
                    .await;
            }
            StreamEvent::Done { stop_reason: r } => {
                linkscope::event("stream.done", format!("{r:?}"));
                pending_visible.flush(tx).await;
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
                if matches!(r, StopReason::PauseTurn)
                    || !matches!(stop_reason, StopReason::ToolUse | StopReason::PauseTurn)
                {
                    stop_reason = r;
                }
                terminal_done_deadline
                    .get_or_insert_with(|| tokio::time::Instant::now() + TERMINAL_DONE_GRACE);
            }
            StreamEvent::ResponseMetadata {
                response_id,
                input_tokens,
            } => {
                pending_visible.flush(tx).await;
                let _ = tx
                    .send(EngineEvent::Stream(RuntimeStreamEvent::ResponseId {
                        id: response_id,
                        input_tokens,
                    }))
                    .await;
                // Feed early input-token count so context estimates are
                // available even if the stream aborts before message_delta.
                if let Some(tokens) = input_tokens {
                    let _ = tx
                        .send(EngineEvent::Stream(RuntimeStreamEvent::Usage {
                            input_tokens: tokens as u32,
                            output_tokens: 0,
                            thinking_tokens: None,
                            cache_read_tokens: 0,
                            cache_write_tokens: 0,
                        }))
                        .await;
                }
            }
            StreamEvent::TextDone { .. } => {
                pending_visible.flush(tx).await;
            }
            StreamEvent::ThinkingDone { signature, .. } => {
                pending_visible.flush(tx).await;
                if let Some(signature) = signature {
                    let _ = tx
                        .send(EngineEvent::Stream(RuntimeStreamEvent::ThinkingSignature(
                            signature,
                        )))
                        .await;
                }
            }
            StreamEvent::RedactedThinkingDone { data, .. } => {
                pending_visible.flush(tx).await;
                let _ = tx
                    .send(EngineEvent::Stream(RuntimeStreamEvent::RedactedThinking(
                        data,
                    )))
                    .await;
            }
            StreamEvent::Usage {
                input_tokens,
                output_tokens,
                thinking_tokens,
                cache_read_tokens,
                cache_write_tokens,
            } => {
                pending_visible.flush(tx).await;
                linkscope::record_items("stream.usage", 1);
                tracing::info!(
                    target: "jfc::stream",
                    input_tokens, output_tokens,
                    thinking_tokens,
                    cache_read_tokens, cache_write_tokens,
                    "stream usage report"
                );
                let _ = tx
                    .send(EngineEvent::Stream(RuntimeStreamEvent::Usage {
                        input_tokens,
                        output_tokens,
                        thinking_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                    }))
                    .await;
            }
            StreamEvent::Keepalive => {
                // Wire-liveness only: forward a content-free Keepalive so the
                // engine's idle watchdog (`check_stream_watchdog`) resets its
                // clock. Crucial during long no-delta phases (extended thinking,
                // large tool-input generation) where Anthropic keeps the socket
                // warm with `ping` frames and nothing else — without this the
                // 90s watchdog would cancel a stream that is genuinely alive.
                // Does NOT set `committed_output` (no model output was produced)
                // and never blocks: dropping one keepalive under backpressure is
                // harmless because the next byte/keepalive will tick the clock.
                pending_visible.flush(tx).await;
                if tx
                    .try_send(EngineEvent::Stream(RuntimeStreamEvent::Keepalive))
                    .is_err()
                {
                    tracing::trace!(target: "jfc::stream", "Keepalive dropped (buffer full)");
                }
            }
            StreamEvent::Error { message } => {
                pending_visible.flush(tx).await;
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
                pending_visible.flush(tx).await;
                let _ = tx
                    .send(EngineEvent::Stream(RuntimeStreamEvent::FallbackTriggered {
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

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    fn event_stream(events: Vec<anyhow::Result<StreamEvent>>) -> EventStream {
        Box::pin(futures::stream::iter(events))
    }

    fn text_chunk(ev: EngineEvent) -> Option<String> {
        match ev {
            EngineEvent::Stream(RuntimeStreamEvent::Chunk {
                text: Some(text), ..
            }) => Some(text),
            _ => None,
        }
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
            Ok(EngineEvent::Stream(RuntimeStreamEvent::Chunk {
                text: Some(text), ..
            })) => assert_eq!(text, "partial"),
            Ok(_) => panic!("expected forwarded text chunk, got different EngineEvent"),
            Err(err) => panic!("expected forwarded text chunk, got receive error: {err}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial]
    async fn visible_text_keeps_first_delta_immediate_then_batches_regression() {
        let (tx, mut rx) = mpsc::channel(8);
        let outcome = drain_stream_events(
            event_stream(vec![
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: "a".to_owned(),
                }),
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: "b".to_owned(),
                }),
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: "c".to_owned(),
                }),
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: "d".to_owned(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                }),
            ]),
            &tx,
            Arc::new(AtomicBool::new(false)),
            CancellationToken::new(),
        )
        .await;

        assert!(matches!(outcome, DrainOutcome::Done(StopReason::EndTurn)));
        drop(tx);
        assert_eq!(
            text_chunk(rx.recv().await.expect("first visible delta")),
            Some("a".to_owned())
        );
        assert_eq!(
            text_chunk(rx.recv().await.expect("batched visible delta")),
            Some("bcd".to_owned())
        );
        let mut extra_text_chunks = Vec::new();
        while let Some(event) = rx.recv().await {
            if let Some(text) = text_chunk(event) {
                extra_text_chunks.push(text);
            }
        }
        assert!(
            extra_text_chunks.is_empty(),
            "no per-character text chunk tail"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial]
    async fn visible_text_batch_flushes_before_tool_delta_regression() {
        let (tx, mut rx) = mpsc::channel(8);
        let outcome = drain_stream_events(
            event_stream(vec![
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: "intro ".to_owned(),
                }),
                Ok(StreamEvent::TextDelta {
                    index: 0,
                    delta: "before tool".to_owned(),
                }),
                Ok(StreamEvent::ToolDelta {
                    index: 1,
                    delta: "{\"cmd\"".to_owned(),
                }),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                }),
            ]),
            &tx,
            Arc::new(AtomicBool::new(false)),
            CancellationToken::new(),
        )
        .await;

        drop(tx);
        let mut seen = vec![format!(
            "outcome:{}",
            matches!(outcome, DrainOutcome::Done(StopReason::EndTurn))
        )];
        while let Some(event) = rx.recv().await {
            match event {
                EngineEvent::Stream(RuntimeStreamEvent::Chunk {
                    text: Some(text), ..
                }) => seen.push(format!("text:{text}")),
                EngineEvent::Stream(RuntimeStreamEvent::ToolInputDelta { index, delta }) => {
                    seen.push(format!("tool:{index}:{delta}"));
                }
                EngineEvent::Stream(_) => seen.push("stream:other".to_owned()),
                _ => seen.push("other".to_owned()),
            }
        }
        assert_eq!(
            seen,
            vec![
                "outcome:true".to_owned(),
                "text:intro ".to_owned(),
                "text:before tool".to_owned(),
                "tool:1:{\"cmd\"".to_owned(),
            ],
            "pending visible text must flush before the tool delta"
        );
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

    // A provider Keepalive (SSE ping) must be forwarded as a runtime Keepalive
    // so the engine dispatcher can reset the idle watchdog. It must NOT be
    // treated as model output (no Chunk) and must not end the turn.
    #[tokio::test]
    async fn keepalive_forwards_runtime_liveness_event_normal() {
        let (tx, mut rx) = mpsc::channel(8);
        let outcome = drain_stream_events(
            event_stream(vec![
                Ok(StreamEvent::Keepalive),
                Ok(StreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                }),
            ]),
            &tx,
            Arc::new(AtomicBool::new(false)),
            CancellationToken::new(),
        )
        .await;
        assert!(matches!(outcome, DrainOutcome::Done(StopReason::EndTurn)));
        assert!(
            matches!(
                rx.try_recv(),
                Ok(EngineEvent::Stream(RuntimeStreamEvent::Keepalive))
            ),
            "expected a forwarded runtime Keepalive event"
        );
    }

    // Every ToolInputDelta must be forwarded even when the receiver lags behind
    // the producer. Before the fix these used `try_send` and were silently
    // dropped on a full channel — which both corrupted tool input and starved
    // the watchdog clock during big file writes. Blocking `send` means a slow
    // consumer applies backpressure rather than losing deltas: all N arrive.
    #[tokio::test]
    async fn tool_input_deltas_are_never_dropped_under_backpressure_regression() {
        // Channel smaller than the delta count: a lossy `try_send` path would
        // drop the overflow. Blocking send must deliver all of them.
        let (tx, mut rx) = mpsc::channel(4);
        const N: usize = 64;
        let mut events: Vec<anyhow::Result<StreamEvent>> = (0..N)
            .map(|i| {
                Ok(StreamEvent::ToolDelta {
                    index: 0,
                    delta: format!("chunk{i}"),
                })
            })
            .collect();
        events.push(Ok(StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        }));

        // Drain concurrently so the producer's blocking sends can make progress.
        let drain = tokio::spawn(async move {
            drain_stream_events(
                event_stream(events),
                &tx,
                Arc::new(AtomicBool::new(false)),
                CancellationToken::new(),
            )
            .await
        });

        let mut deltas = 0usize;
        while let Some(ev) = rx.recv().await {
            if let EngineEvent::Stream(RuntimeStreamEvent::ToolInputDelta { delta, .. }) = ev {
                assert_eq!(delta, format!("chunk{deltas}"), "in-order, no gaps");
                deltas += 1;
            }
        }
        let outcome = drain.await.expect("drain task");
        assert!(matches!(outcome, DrainOutcome::Done(StopReason::EndTurn)));
        assert_eq!(deltas, N, "all tool-input deltas delivered (none dropped)");
    }

    #[tokio::test]
    async fn thinking_token_deltas_are_never_dropped_under_backpressure_regression() {
        let (tx, mut rx) = mpsc::channel(4);
        const N: usize = 64;
        let mut events: Vec<anyhow::Result<StreamEvent>> = (0..N)
            .map(|_| Ok(StreamEvent::ThinkingTokens { index: 0, delta: 1 }))
            .collect();
        events.push(Ok(StreamEvent::Done {
            stop_reason: StopReason::EndTurn,
        }));

        let drain = tokio::spawn(async move {
            drain_stream_events(
                event_stream(events),
                &tx,
                Arc::new(AtomicBool::new(false)),
                CancellationToken::new(),
            )
            .await
        });

        let mut deltas = 0usize;
        while let Some(ev) = rx.recv().await {
            if let EngineEvent::Stream(RuntimeStreamEvent::ThinkingTokens(delta)) = ev {
                assert_eq!(delta, 1);
                deltas += 1;
            }
        }
        let outcome = drain.await.expect("drain task");
        assert!(matches!(outcome, DrainOutcome::Done(StopReason::EndTurn)));
        assert_eq!(deltas, N);
    }
}
