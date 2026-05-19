//! `StreamEvent::{Chunk, ToolInputDelta, RedactedThinking, ResponseId}`
//! handlers — the body of an active stream that produces visible
//! text/reasoning before the model emits a tool or finishes.

use crate::app::App;
use crate::types::*;

use super::super::guards::streaming_assistant_mut;

pub(crate) fn handle_chunk(app: &mut App, text: Option<String>, reasoning: Option<String>) {
    app.record_stream_activity();
    app.network_recovery_status = None;
    app.network_recovery_attempts = 0;
    // Reset the stall clock on every chunk so the spinner's
    // sub-status (`warming up` / `thinking` / `almost done`)
    // reflects time-since-last-byte, not time-since-stream-start.
    let now = std::time::Instant::now();
    app.streaming_last_token_at = Some(now);
    // Stamp for the right-edge token-rain animation. The
    // renderer reads this each frame and lights one cell
    // in the rain column with intensity proportional to
    // recency (full at 0ms, dark at 800ms+).
    app.last_token_arrival = Some(now);
    // v126 responseLengthRef: accumulate ALL content bytes for the
    // spinner's chars/4 token estimate.
    if let Some(ref t) = text {
        app.streaming_response_bytes += t.len();
        app.network_bytes_in = app.network_bytes_in.saturating_add(t.len() as u64);
    }
    if let Some(ref r) = reasoning {
        app.streaming_response_bytes += r.len();
        app.network_bytes_in = app.network_bytes_in.saturating_add(r.len() as u64);
    }
    if let Some(chunk) = text {
        // First text byte after a thinking phase ⇒ thinking
        // ended. Mirrors v126's HcH transition from
        // `streamMode = "thinking"` to `"responding"` —
        // cli.js:413612 captures the duration here so the
        // spinner can switch from `thinking…` to
        // `thought for Ns`. Only set on the first transition;
        // a turn that toggles back into thinking later (rare
        // — the API doesn't really do this) keeps the first
        // duration so the timer doesn't reset visibly.
        if app.thinking_started_at.is_some() && app.thinking_ended_at.is_none() {
            app.thinking_ended_at = Some(now);
        }
        // Idle prefetch: throttled to one batch per 500ms,
        // max 2 concurrent in-flight reads.
        let prefetch_elapsed = now.duration_since(app.last_prefetch_at);
        if prefetch_elapsed >= std::time::Duration::from_millis(500) {
            let prefetch_targets = crate::idle_prefetch::extract_candidates(&chunk);
            let mut fired = 0usize;
            for path in prefetch_targets.into_iter() {
                if fired >= 2 {
                    break;
                }
                let in_flight = app
                    .prefetch_in_flight
                    .load(std::sync::atomic::Ordering::Relaxed);
                if in_flight >= 2 {
                    break;
                }
                if crate::idle_prefetch::get(&path, None, None).is_some() {
                    continue;
                }
                app.prefetch_in_flight
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let counter = app.prefetch_in_flight.clone();
                tokio::spawn(async move {
                    if let Ok(body) = tokio::fs::read_to_string(&path).await {
                        crate::idle_prefetch::put(&path, None, None, body);
                    }
                    counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                });
                fired += 1;
            }
            if fired > 0 {
                app.last_prefetch_at = now;
            }
        }

        app.streaming_text.push_str(&chunk);
        if let Some(msg) = streaming_assistant_mut(app) {
            // Append to the *last* part if it's still a Text
            // segment; otherwise start a new Text part. The
            // earlier `.find(|p| matches!(p, Text(_)))`
            // pattern always merged into the first Text part,
            // which silently glued post-tool text segments
            // back into the pre-tool paragraph and dropped
            // the natural part-boundary between them. See
            // session ses_20260509_205615 msg 649: five
            // logical turns collapsed to a single Text part
            // with `:`-joined run-on prose.
            match msg.parts.last_mut() {
                Some(MessagePart::Text(t)) => t.push_str(&chunk),
                _ => msg.parts.push(MessagePart::Text(chunk)),
            }
        }
    }
    if let Some(chunk) = reasoning {
        // First reasoning byte ⇒ thinking started. Mirrors
        // v126's HcH content_block_start type=thinking
        // transition (cli.js:413610). Subsequent chunks just
        // extend the streaming buffer; the spinner reads
        // `thinking_started_at` to know we're in
        // thinking-mode.
        if app.thinking_started_at.is_none() {
            app.thinking_started_at = Some(now);
        }
        app.streaming_reasoning.push_str(&chunk);
        if let Some(msg) = streaming_assistant_mut(app) {
            // Same fix as the text path above: append to
            // the last part if it's still a Reasoning
            // segment, otherwise start a new one so a
            // post-tool/post-text reasoning block doesn't
            // get merged into an earlier thinking segment.
            match msg.parts.last_mut() {
                Some(MessagePart::Reasoning(t)) => t.push_str(&chunk),
                _ => msg.parts.push(MessagePart::Reasoning(chunk)),
            }
        }
    }
    // Follow content as it streams *only when the user is
    // already pinned to the bottom*. `app.follow_bottom` is
    // set true on submit and on any explicit scroll-to-bottom;
    // it goes false the moment the user scrolls up. Without
    // this gate, scrolling up to read prior context during a
    // long stream would yank you back to the bottom on every
    // chunk. v126 has the same "stick when at bottom" rule.
    if app.follow_bottom {
        app.scroll_to_bottom();
    }
}

pub(crate) fn handle_tool_input_delta(app: &mut App, byte_len: usize) {
    app.network_recovery_status = None;
    app.network_recovery_attempts = 0;
    // Tool input JSON streaming — accumulate bytes for the spinner's
    // token estimate and reset the stall timer. Matches v126's
    // accumulation of input_json_delta into responseLengthRef.
    // Also tick `last_stream_event_at` via `record_stream_activity`
    // so the watchdog doesn't false-trip during a long Task prompt
    // stream (the JSON for a 4-KB prompt arrives over many seconds
    // with no other StreamChunk events between).
    app.streaming_response_bytes += byte_len;
    app.network_bytes_in = app.network_bytes_in.saturating_add(byte_len as u64);
    app.streaming_last_token_at = Some(std::time::Instant::now());
    app.record_stream_activity();
}

pub(crate) fn handle_redacted_thinking(app: &mut App, data: String) {
    app.record_stream_activity();
    app.network_bytes_in = app.network_bytes_in.saturating_add(data.len() as u64);
    if let Some(msg) = streaming_assistant_mut(app) {
        msg.parts.push(MessagePart::RedactedThinking(data));
    }
}

pub(crate) fn handle_response_id(app: &mut App, id: String) {
    // Even a bare response-id frame is signal that the
    // server is still alive — bump the EKG counter a
    // small fixed amount so the heartbeat reflects
    // server keepalives even when the model is silent.
    app.network_bytes_in = app.network_bytes_in.saturating_add(id.len() as u64);
    app.last_response_id = Some(id);
}
