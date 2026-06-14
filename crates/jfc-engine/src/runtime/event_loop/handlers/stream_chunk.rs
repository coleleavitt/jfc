//! `StreamEvent::{Chunk, ToolInputDelta, RedactedThinking, ResponseId}`
//! handlers — the body of an active stream that produces visible
//! text/reasoning before the model emits a tool or finishes.

use crate::app::EngineState;
use crate::types::*;

use super::super::guards::streaming_assistant_mut;

pub fn handle_chunk(state: &mut EngineState, text: Option<String>, reasoning: Option<String>) {
    state.record_stream_activity();
    state.network_recovery_status = None;
    state.network_recovery_attempts = 0;
    state.stream_lifecycle = None;
    // First-byte trace: log exactly once per turn, when the very first
    // text/reasoning delta lands. This is the "connection opened, model is
    // producing output" signal — the boundary the interrupt-on-submit and
    // superseded-cancel logic keys off. One line per turn (gated on
    // `streaming_response_bytes == 0`), so it's cheap even on long streams.
    if state.streaming_response_bytes == 0 {
        // Time-to-first-token: gap from this stream round's open to its first
        // content delta. Re-captured each round (this block only runs when
        // `streaming_response_bytes == 0`, i.e. on the round's first byte), so
        // the footer always reflects the current round and no stale value can
        // leak across turn-start paths. Falls back to `turn_started_at` if the
        // per-round stream baseline is missing (resends reuse the turn clock).
        if let Some(start) = state.streaming_started_at.or(state.turn_started_at) {
            state.ttft_ms =
                Some(std::time::Instant::now().duration_since(start).as_millis() as u64);
        }
        tracing::debug!(
            target: "jfc::stream::lifecycle",
            assistant_idx = ?state.streaming_assistant_idx,
            first_kind = if text.is_some() { "text" } else { "reasoning" },
            ttft_ms = ?state.ttft_ms,
            "first stream byte — connection producing output"
        );
    }
    // Reset the quiet clock on every chunk so the spinner's `quiet Ns`
    // chip (and the row-dim past 30s) reflects time-since-last-byte, not
    // time-since-stream-start.
    let now = std::time::Instant::now();
    state.streaming_last_token_at = Some(now);
    // v126 responseLengthRef: accumulate ALL content bytes for the
    // spinner's chars/4 token estimate.
    if let Some(ref t) = text {
        state.streaming_response_bytes += t.len();
    }
    if let Some(ref r) = reasoning {
        state.streaming_response_bytes += r.len();
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
        if state.thinking_started_at.is_some() && state.thinking_ended_at.is_none() {
            state.thinking_ended_at = Some(now);
        }
        // Idle prefetch: throttled to one batch per 500ms,
        // max 2 concurrent in-flight reads.
        let prefetch_elapsed = now.duration_since(state.last_prefetch_at);
        if prefetch_elapsed >= std::time::Duration::from_millis(500) {
            let prefetch_targets = crate::idle_prefetch::extract_candidates(&chunk);
            let mut fired = 0usize;
            for path in prefetch_targets.into_iter() {
                if fired >= 2 {
                    break;
                }
                let in_flight = state
                    .prefetch_in_flight
                    .load(std::sync::atomic::Ordering::Relaxed);
                if in_flight >= 2 {
                    break;
                }
                if crate::idle_prefetch::get(&path, None, None).is_some() {
                    continue;
                }
                state
                    .prefetch_in_flight
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let counter = state.prefetch_in_flight.clone();
                tokio::spawn(async move {
                    if let Ok(body) = tokio::fs::read_to_string(&path).await {
                        crate::idle_prefetch::put(&path, None, None, body);
                    }
                    counter.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
                });
                fired += 1;
            }
            if fired > 0 {
                state.last_prefetch_at = now;
            }
        }

        state.streaming_text.push_str(&chunk);

        // CC 2.1.167 MessageDisplay hook — fires on each text chunk when a
        // registered handler wants to intercept/rewrite displayed content.
        // Fast path: no-op when no MessageDisplay hooks are registered.
        // Fire-and-forget (async, non-blocking) — display hooks must not
        // stall the stream.
        crate::hooks::fire_async(
            crate::hooks::HookPoint::OnMessageDisplay,
            &crate::hooks::HookContext::for_session(
                state
                    .current_session_id
                    .as_ref()
                    .map(|s| s.as_str())
                    .unwrap_or("<no-session>"),
            )
            .with_extra("chunk_len", chunk.len().to_string()),
        );

        if let Some(msg) = streaming_assistant_mut(state) {
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
        if state.thinking_started_at.is_none() {
            state.thinking_started_at = Some(now);
        }
        state.streaming_reasoning.push_str(&chunk);
        if let Some(msg) = streaming_assistant_mut(state) {
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
    // The "stick when at bottom" follow policy (and its freeze-during-drag
    // exception) is view logic — the frontend applies it when draining this
    // effect (see `apply_engine_effects`).
    state.push_effect(crate::app::EngineEffect::TranscriptAppended);
}

pub fn handle_tool_input_delta(state: &mut EngineState, byte_len: usize) {
    state.network_recovery_status = None;
    state.network_recovery_attempts = 0;
    state.stream_lifecycle = None;
    // Tool input JSON streaming — accumulate bytes for the spinner's
    // token estimate and reset the stall timer. Matches v126's
    // accumulation of input_json_delta into responseLengthRef.
    // Also tick `last_stream_event_at` via `record_stream_activity`
    // so the watchdog doesn't false-trip during a long Task prompt
    // stream (the JSON for a 4-KB prompt arrives over many seconds
    // with no other StreamChunk events between).
    state.streaming_response_bytes += byte_len;
    state.streaming_last_token_at = Some(std::time::Instant::now());
    state.record_stream_activity();
}

pub fn handle_redacted_thinking(state: &mut EngineState, data: String) {
    state.record_stream_activity();
    state.stream_lifecycle = None;
    if let Some(msg) = streaming_assistant_mut(state) {
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
pub fn handle_thinking_tokens(state: &mut EngineState, tokens: u32) {
    state.record_stream_activity();
    state.stream_lifecycle = None;
    let now = std::time::Instant::now();
    // Reset the quiet clock — an estimate ping is wire activity, same as a
    // visible delta. Without this a long silent-thinking phase would wrongly
    // trip the `quiet Ns` chip + row-dim while the model is actively working.
    state.streaming_last_token_at = Some(now);
    // Mark the thinking phase live if no visible reasoning byte set it. Only
    // on the first estimate, and only before any text byte ended thinking, so
    // we never re-open a concluded thinking block.
    if state.thinking_started_at.is_none() && state.thinking_ended_at.is_none() {
        state.thinking_started_at = Some(now);
    }
    // `tokens` is the API's `estimated_tokens` — the *cumulative running total*
    // for the current thinking block (per cli.js: "running total … not the
    // authoritative billed output_tokens"). Accumulate only the delta against
    // the last total we saw, exactly as the output-token path does with
    // `last_usage_output`. Adding the cumulative total each event triple-counts.
    let delta = if tokens >= state.last_thinking_estimate {
        tokens - state.last_thinking_estimate
    } else {
        // A new thinking block restarted the total lower → its growth so far is
        // the full `tokens` (it began from 0).
        tokens
    };
    state.last_thinking_estimate = tokens;
    state.streaming_thinking_tokens = state.streaming_thinking_tokens.saturating_add(delta as u64);
}

pub fn handle_response_id(state: &mut EngineState, id: String) {
    state.record_stream_activity();
    state.stream_lifecycle = None;
    state.last_response_id = Some(id);
}

#[cfg(test)]
mod tests {
    use crate::app::EngineState;
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

    fn test_app() -> EngineState {
        EngineState::new(Arc::new(TestProvider), "test-model")
    }

    // Summarized/redacted thinking: estimates arrive with no visible reasoning
    // text, so this handler is the only thing that can light up the thinking
    // phase. It must both accumulate the count AND mark thinking started.
    // `estimated_tokens` is the CUMULATIVE running total for the thinking block,
    // so the counter must follow the latest total — not sum every event. A
    // monotonically-rising 40 → 90 → 130 means the block is at 130, not 260.
    #[test]
    fn thinking_tokens_track_cumulative_total_normal() {
        let mut state = test_app();
        assert!(state.thinking_started_at.is_none());

        handle_thinking_tokens(&mut state, 40);
        handle_thinking_tokens(&mut state, 90);
        handle_thinking_tokens(&mut state, 130);

        assert_eq!(
            state.streaming_thinking_tokens, 130,
            "must equal the latest cumulative total, not the sum of events"
        );
        assert!(
            state.thinking_started_at.is_some(),
            "first estimate must mark the thinking phase live"
        );
        assert!(state.thinking_ended_at.is_none());
    }

    // REGRESSION (thinking-token triple-count): adding the cumulative total on
    // each event inflated the count (100,250,400 → 750). The delta-accumulation
    // fix must report the true block total (400).
    #[test]
    fn thinking_tokens_do_not_triple_count_regression() {
        let mut state = test_app();
        for total in [100u32, 250, 400] {
            handle_thinking_tokens(&mut state, total);
        }
        assert_eq!(
            state.streaming_thinking_tokens, 400,
            "cumulative totals must not be summed (was 750 before the fix)"
        );
    }

    // A second thinking block restarts `estimated_tokens` lower; its tokens must
    // ADD to the first block's total (turn-cumulative across blocks).
    #[test]
    fn thinking_tokens_sum_across_blocks_robust() {
        let mut state = test_app();
        // Block 1 reaches 400.
        handle_thinking_tokens(&mut state, 150);
        handle_thinking_tokens(&mut state, 400);
        // Block 2 restarts low (50) then grows to 120.
        handle_thinking_tokens(&mut state, 50);
        handle_thinking_tokens(&mut state, 120);
        assert_eq!(
            state.streaming_thinking_tokens, 520,
            "block totals accumulate across the turn (400 + 120)"
        );
    }

    // Once a visible text byte concluded thinking (`thinking_ended_at` set), a
    // late stray estimate must NOT re-open the thinking phase.
    #[test]
    fn thinking_tokens_do_not_reopen_concluded_phase_robust() {
        let mut state = test_app();
        let t0 = std::time::Instant::now();
        state.thinking_started_at = Some(t0);
        state.thinking_ended_at = Some(t0);

        handle_thinking_tokens(&mut state, 10);

        // Counter still moves (first total 10 from a fresh baseline), but the
        // phase stays concluded.
        assert_eq!(state.streaming_thinking_tokens, 10);
        assert_eq!(state.thinking_started_at, Some(t0));
        assert_eq!(state.thinking_ended_at, Some(t0));
    }

    // Saturating add: a pathological huge delta can't overflow the counter.
    #[test]
    fn thinking_tokens_saturate_robust() {
        let mut state = test_app();
        state.streaming_thinking_tokens = u64::MAX - 1;
        // First event from a zero baseline → delta == tokens == 100.
        handle_thinking_tokens(&mut state, 100);
        assert_eq!(state.streaming_thinking_tokens, u64::MAX);
    }
}
