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
    // superseded-cancel logic keys off.
    let first_content_delta = state.streaming_text.is_empty()
        && state.streaming_reasoning.is_empty()
        && state.streaming_response_bytes == 0;
    if first_content_delta {
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
    // v126/177 responseLengthRef: text bytes grow directly, while thinking
    // progress is driven by `thinking_delta.estimated_tokens` / signature
    // estimates in handle_thinking_tokens. Raw reasoning bytes are preserved in
    // the transcript below but do not inflate the shared response-length meter.
    if let Some(ref t) = text {
        state.streaming_response_bytes += t.len();
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

        let session_id_for_chunk = state
            .current_session_id
            .as_ref()
            .map(|s| s.as_str())
            .unwrap_or("<no-session>");

        // CC 2.1.167 MessageDisplay hook — fires on each text chunk when a
        // registered handler wants to intercept/rewrite displayed content.
        // This is a hot path, and `fire_async` is synchronous despite its
        // name, so skip context allocation entirely when no hook can observe
        // the event.
        if crate::hooks::has_hooks(crate::hooks::HookPoint::OnMessageDisplay) {
            crate::hooks::fire_async(
                crate::hooks::HookPoint::OnMessageDisplay,
                &crate::hooks::HookContext::for_session(session_id_for_chunk)
                    .with_extra("chunk_len", chunk.len().to_string()),
            );
        }

        // OnModelResponse hook — fires on every text delta so external
        // scripts can observe the raw stream. High-frequency site: only
        // costs anything when Shell handlers are registered for this point.
        if crate::hooks::has_hooks(crate::hooks::HookPoint::OnModelResponse) {
            crate::hooks::fire_async(
                crate::hooks::HookPoint::OnModelResponse,
                &crate::hooks::HookContext::for_session(session_id_for_chunk)
                    .with_extra("chunk", chunk.clone())
                    .with_extra("chunk_len", chunk.len().to_string())
                    .with_extra("is_final", "false"),
            );
        }

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
    state.streaming_last_token_at = Some(std::time::Instant::now());
    if let Some(msg) = streaming_assistant_mut(state) {
        msg.parts.push(MessagePart::RedactedThinking(data));
    }
}

pub fn handle_thinking_tokens(state: &mut EngineState, token_delta: u32) {
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
    state.streaming_thinking_tokens = state
        .streaming_thinking_tokens
        .saturating_add(token_delta as u64);
}

pub fn handle_thinking_signature(state: &mut EngineState, signature: String) {
    if signature.is_empty() {
        return;
    }
    if let Some(msg) = streaming_assistant_mut(state) {
        if matches!(msg.parts.last(), Some(MessagePart::ReasoningSignature(_))) {
            msg.parts.pop();
        }
        msg.parts.push(MessagePart::ReasoningSignature(signature));
    }
}

pub fn handle_response_id(state: &mut EngineState, id: String) {
    state.record_stream_activity();
    crate::cache_lineage::record_response_id(state, id);
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

    #[test]
    fn response_id_keeps_pre_output_lifecycle_regression() {
        let mut state = test_app();
        state.stream_lifecycle = Some(crate::runtime::StreamLifecycleStatus::new(
            crate::runtime::StreamLifecyclePhase::StreamOpened,
            Some("waiting for first event".to_owned()),
        ));

        handle_response_id(&mut state, "msg_123".to_owned());

        assert!(state.stream_lifecycle.is_some());
        assert_eq!(state.last_response_id.as_deref(), Some("msg_123"));
    }

    #[test]
    fn thinking_tokens_accumulate_delta_frames_normal() {
        let mut state = test_app();
        assert!(state.thinking_started_at.is_none());

        handle_thinking_tokens(&mut state, 40);
        handle_thinking_tokens(&mut state, 50);
        handle_thinking_tokens(&mut state, 40);

        assert_eq!(
            state.streaming_thinking_tokens, 130,
            "raw estimated_tokens frames are deltas in Claude Code 2.1.177"
        );
        assert!(
            state.thinking_started_at.is_some(),
            "first estimate must mark the thinking phase live"
        );
        assert!(state.thinking_ended_at.is_none());
    }

    #[test]
    fn repeated_equal_thinking_token_deltas_keep_advancing_regression() {
        let mut state = test_app();
        for delta in [50u32, 50, 50] {
            handle_thinking_tokens(&mut state, delta);
        }
        assert_eq!(
            state.streaming_thinking_tokens, 150,
            "equal 50-token delta frames must not be mistaken for a stuck cumulative total"
        );
    }

    #[test]
    fn thinking_tokens_sum_across_blocks_robust() {
        let mut state = test_app();
        handle_thinking_tokens(&mut state, 150);
        handle_thinking_tokens(&mut state, 50);
        handle_thinking_tokens(&mut state, 120);
        assert_eq!(
            state.streaming_thinking_tokens, 320,
            "delta frames accumulate across the turn"
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

    #[test]
    fn reasoning_text_does_not_advance_response_length_regression() {
        let mut state = test_app();

        handle_chunk(&mut state, None, Some("thinking text".to_owned()));

        assert_eq!(
            state.streaming_response_bytes, 0,
            "reasoning text should not be counted as visible output"
        );
        assert_eq!(state.streaming_reasoning, "thinking text");
    }

    #[test]
    fn redacted_thinking_resets_quiet_clock_regression() {
        let mut state = test_app();
        state.streaming_last_token_at = None;

        handle_redacted_thinking(&mut state, "opaque".to_owned());

        assert!(state.streaming_last_token_at.is_some());
    }

    #[test]
    fn thinking_tokens_do_not_count_as_output_regression() {
        let mut state = test_app();

        handle_thinking_tokens(&mut state, 12);

        assert_eq!(state.streaming_thinking_tokens, 12);
        assert_eq!(
            state.streaming_response_bytes, 0,
            "thinking tokens are shown in the thinking chip, not as output tokens"
        );
    }
}
