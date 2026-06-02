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
    app.stream_lifecycle = None;
    // First-byte trace: log exactly once per turn, when the very first
    // text/reasoning delta lands. This is the "connection opened, model is
    // producing output" signal — the boundary the interrupt-on-submit and
    // superseded-cancel logic keys off. One line per turn (gated on
    // `streaming_response_bytes == 0`), so it's cheap even on long streams.
    if app.streaming_response_bytes == 0 {
        tracing::debug!(
            target: "jfc::stream::lifecycle",
            assistant_idx = ?app.streaming_assistant_idx,
            first_kind = if text.is_some() { "text" } else { "reasoning" },
            "first stream byte — connection producing output"
        );
    }
    // Reset the quiet clock on every chunk so the spinner's `quiet Ns`
    // chip (and the row-dim past 30s) reflects time-since-last-byte, not
    // time-since-stream-start.
    let now = std::time::Instant::now();
    app.streaming_last_token_at = Some(now);
    // v126 responseLengthRef: accumulate ALL content bytes for the
    // spinner's chars/4 token estimate.
    if let Some(ref t) = text {
        app.streaming_response_bytes += t.len();
    }
    if let Some(ref r) = reasoning {
        app.streaming_response_bytes += r.len();
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
    // …but freeze the viewport while the user is mid-drag selecting text.
    // The selection is anchored to absolute screen cells; autoscrolling
    // would slide the transcript out from under the highlight and copy the
    // wrong content on release.
    let selecting = app.text_selection.is_some_and(|s| s.dragged);
    if app.follow_bottom && !selecting {
        app.scroll_to_bottom();
    }
}

pub(crate) fn handle_tool_input_delta(app: &mut App, byte_len: usize) {
    app.network_recovery_status = None;
    app.network_recovery_attempts = 0;
    app.stream_lifecycle = None;
    // Tool input JSON streaming — accumulate bytes for the spinner's
    // token estimate and reset the stall timer. Matches v126's
    // accumulation of input_json_delta into responseLengthRef.
    // Also tick `last_stream_event_at` via `record_stream_activity`
    // so the watchdog doesn't false-trip during a long Task prompt
    // stream (the JSON for a 4-KB prompt arrives over many seconds
    // with no other StreamChunk events between).
    app.streaming_response_bytes += byte_len;
    app.streaming_last_token_at = Some(std::time::Instant::now());
    app.record_stream_activity();
}

pub(crate) fn handle_redacted_thinking(app: &mut App, data: String) {
    app.record_stream_activity();
    app.stream_lifecycle = None;
    if let Some(msg) = streaming_assistant_mut(app) {
        msg.parts.push(MessagePart::RedactedThinking(data));
    }
}

/// Accumulate a server-authoritative thinking-token estimate
/// (`thinking_delta.estimated_tokens`). During *summarized* or *redacted*
/// thinking the API streams these estimates without any visible reasoning
/// text, so `handle_chunk` never fires and the spinner would otherwise show
/// no thinking activity at all. Marking `thinking_started_at` here lets the
/// `thinking …` verb surface during that phase, and the running total feeds
/// the `⟳ N thinking` chip. Mirrors cli.js's `thinkingTokenEstimate +=`
/// accumulation (cli.beautified.js:574722).
pub(crate) fn handle_thinking_tokens(app: &mut App, tokens: u32) {
    app.record_stream_activity();
    app.stream_lifecycle = None;
    let now = std::time::Instant::now();
    // Reset the quiet clock — an estimate ping is wire activity, same as a
    // visible delta. Without this a long silent-thinking phase would wrongly
    // trip the `quiet Ns` chip + row-dim while the model is actively working.
    app.streaming_last_token_at = Some(now);
    // Mark the thinking phase live if no visible reasoning byte set it. Only
    // on the first estimate, and only before any text byte ended thinking, so
    // we never re-open a concluded thinking block.
    if app.thinking_started_at.is_none() && app.thinking_ended_at.is_none() {
        app.thinking_started_at = Some(now);
    }
    app.streaming_thinking_tokens = app.streaming_thinking_tokens.saturating_add(tokens as u64);
}

pub(crate) fn handle_response_id(app: &mut App, id: String) {
    app.record_stream_activity();
    app.stream_lifecycle = None;
    app.last_response_id = Some(id);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;

    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }

        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }

        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }

    impl jfc_provider::seal::Sealed for TestProvider {}

    fn test_app() -> App {
        App::new(Arc::new(TestProvider), "test-model")
    }

    // Summarized/redacted thinking: estimates arrive with no visible reasoning
    // text, so this handler is the only thing that can light up the thinking
    // phase. It must both accumulate the count AND mark thinking started.
    #[test]
    fn thinking_tokens_accumulate_and_mark_phase_normal() {
        let mut app = test_app();
        assert!(app.thinking_started_at.is_none());

        handle_thinking_tokens(&mut app, 40);
        handle_thinking_tokens(&mut app, 35);

        assert_eq!(app.streaming_thinking_tokens, 75);
        assert!(
            app.thinking_started_at.is_some(),
            "first estimate must mark the thinking phase live"
        );
        assert!(app.thinking_ended_at.is_none());
    }

    // Once a visible text byte concluded thinking (`thinking_ended_at` set), a
    // late stray estimate must NOT re-open the thinking phase.
    #[test]
    fn thinking_tokens_do_not_reopen_concluded_phase_robust() {
        let mut app = test_app();
        let t0 = std::time::Instant::now();
        app.thinking_started_at = Some(t0);
        app.thinking_ended_at = Some(t0);

        handle_thinking_tokens(&mut app, 10);

        // Counter still moves, but the phase stays concluded.
        assert_eq!(app.streaming_thinking_tokens, 10);
        assert_eq!(app.thinking_started_at, Some(t0));
        assert_eq!(app.thinking_ended_at, Some(t0));
    }

    // Saturating add: pathological huge estimates can't overflow the counter.
    #[test]
    fn thinking_tokens_saturate_robust() {
        let mut app = test_app();
        app.streaming_thinking_tokens = u64::MAX - 1;
        handle_thinking_tokens(&mut app, 100);
        assert_eq!(app.streaming_thinking_tokens, u64::MAX);
    }
}
