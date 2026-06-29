use futures::StreamExt;

use jfc_provider::{EventStream, StopReason, StreamEvent};

use super::{BlockState, SseEvent, finalize_open_blocks, log_parsed_event, translate};

pub fn into_event_stream(resp: reqwest::Response) -> EventStream {
    // Tracing parity with the OpenWebUI provider: dump raw SSE bytes at TRACE,
    // log every parsed event type at DEBUG, log finish_reason / errors at INFO.
    // Flip `RUST_LOG=jfc::provider::anthropic_sse=trace` to see raw chunks
    // when debugging upstream SSE weirdness.
    let body_started_at = std::time::Instant::now();
    let mut first_body_chunk_seen = false;
    let mut body_bytes_seen = 0usize;
    let mut body_chunks_seen = 0u64;
    let byte_stream = resp.bytes_stream().map(move |result| {
        match &result {
            Ok(chunk) => {
                body_chunks_seen += 1;
                body_bytes_seen += chunk.len();
                if !first_body_chunk_seen {
                    first_body_chunk_seen = true;
                    tracing::info!(
                        target: "jfc::provider::anthropic_sse",
                        latency_ms = body_started_at.elapsed().as_millis() as u64,
                        chunk_bytes = chunk.len(),
                        "first SSE body bytes received"
                    );
                }
                tracing::trace!(
                    target: "jfc::provider::anthropic_sse",
                    chunk_bytes = chunk.len(),
                    body_bytes_seen,
                    body_chunks_seen,
                    "sse raw body chunk"
                );
            }
            Err(e) => {
                tracing::warn!(
                    target: "jfc::provider::anthropic_sse",
                    error = %e,
                    body_bytes_seen,
                    body_chunks_seen,
                    first_body_chunk_seen,
                    "SSE body byte stream error"
                );
            }
        }
        result
    });
    let event_stream = jfc_anthropic_sdk::sse::byte_stream_events(byte_stream)
        .scan(
            (
                Vec::<Option<BlockState>>::new(),
                None::<StopReason>,
                std::time::Instant::now(),
                false,
                0usize,
                0u64,
            ),
            |state, result| {
                let (
                    blocks,
                    stop_reason,
                    stream_started_at,
                    first_payload_seen,
                    bytes_seen,
                    events_seen,
                ) = state;
                let out = match result {
                    Ok(ev) => {
                        *events_seen += 1;
                        *bytes_seen += ev.data.len();
                        if !*first_payload_seen && ev.event != "ping" && !ev.data.is_empty() {
                            *first_payload_seen = true;
                            tracing::info!(
                                target: "jfc::provider::anthropic_sse",
                                latency_ms = stream_started_at.elapsed().as_millis() as u64,
                                event = %ev.event,
                                bytes_seen = *bytes_seen,
                                events_seen = *events_seen,
                                "first SSE payload received"
                            );
                        }
                        tracing::trace!(
                            target: "jfc::provider::anthropic_sse",
                            event = %ev.event,
                            data = %&ev.data[..ev.data.len().min(400)],
                            "sse raw"
                        );
                        if ev.event == "ping" || ev.data.is_empty() {
                            // Surface keepalives as an explicit liveness event
                            // instead of dropping them. Anthropic emits `ping`
                            // frames (and empty SSE comment lines) to keep the
                            // socket warm during long thinking / tool-input
                            // phases that produce no semantic delta. Forwarding
                            // a content-free `Keepalive` lets the runtime reset
                            // its idle watchdog on raw-byte liveness — mirroring
                            // Claude Code's byte-watchdog which refreshes on
                            // every chunk pull — so a slow-but-alive stream is
                            // never mistaken for a dead one.
                            return futures::future::ready(Some(vec![Ok(StreamEvent::Keepalive)]));
                        }
                        if ev.data == "[DONE]" {
                            tracing::debug!(target: "jfc::provider::anthropic_sse", "sse [DONE]");
                            return futures::future::ready(Some(Vec::new()));
                        }
                        // `context_hint` is a special SSE event type (not a JSON
                        // `type` field) that Anthropic sends when the model is
                        // approaching its context limit. Mirrors v132 cli.js line
                        // 471490: treat it the same as a prompt_too_long rejection
                        // so the main loop fires auto-compaction.
                        if ev.event == "context_hint" || ev.data.contains("\"context_hint\"") {
                            tracing::info!(
                                target: "jfc::provider::anthropic_sse",
                                event = %ev.event,
                                data = %&ev.data[..ev.data.len().min(200)],
                                "context_hint received — signalling auto-compact"
                            );
                            return futures::future::ready(Some(vec![Ok(StreamEvent::Error {
                                message: format!(
                                    "auto-compact: context_hint from server ({})",
                                    &ev.data[..ev.data.len().min(120)]
                                ),
                            })]));
                        }
                        match serde_json::from_str::<SseEvent>(&ev.data) {
                            Ok(parsed) => {
                                log_parsed_event(&parsed);
                                translate(parsed, blocks, stop_reason)
                                    .map(|ev| vec![Ok(ev)])
                                    .unwrap_or_default()
                            }
                            Err(e) => {
                                tracing::warn!(
                                    target: "jfc::provider::anthropic_sse",
                                    error = %e,
                                    data = %&ev.data[..ev.data.len().min(200)],
                                    "sse parse error"
                                );
                                // Flush any open text/thinking block before the
                                // error so committed output isn't lost when a
                                // malformed event lands mid-stream.
                                let mut batch: Vec<anyhow::Result<StreamEvent>> =
                                    finalize_open_blocks(blocks, stop_reason)
                                        .into_iter()
                                        .map(Ok)
                                        .collect();
                                batch.push(Err(anyhow::anyhow!("SSE parse error: {e}")));
                                batch
                            }
                        }
                    }
                    Err(e) => {
                        let prefix = if *first_payload_seen {
                            "SSE stream parse error"
                        } else {
                            "SSE stream failed before first event"
                        };
                        let mut batch: Vec<anyhow::Result<StreamEvent>> =
                            finalize_open_blocks(blocks, stop_reason)
                                .into_iter()
                                .map(Ok)
                                .collect();
                        batch.push(Err(anyhow::anyhow!("{prefix}: {e}")));
                        batch
                    }
                };
                futures::future::ready(Some(out))
            },
        )
        .flat_map(futures::stream::iter);

    Box::pin(event_stream)
}
