//! `StreamEvent::Usage { ... }` handler — token accounting.

use crate::app::EngineState;

use super::super::guards::streaming_assistant_mut;

/// Handle `StreamEvent::Usage { ... }`.
pub fn handle_stream_usage(
    state: &mut EngineState,
    input_tokens: u32,
    output_tokens: u32,
    thinking_tokens: Option<u32>,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
) {
    state.record_stream_activity();
    // Anthropic sends *cumulative* token counts in every
    // `message_delta` event (sse.rs:212-218 — see also
    // anthropic-messaging spec). Naively calling `add_delta`
    // on each event triple-counts: a 10-delta turn ending at
    // 2000 output tokens would push 1+5+10+25+...+2000 into
    // the per-model bucket, producing 5-15× inflated totals
    // (the user's "84,284 in" with `ctx 28k / 200k` is this
    // bug). Compute the genuine delta against the per-turn
    // baseline before adding.
    let partial_input_only =
        output_tokens == 0 && cache_read_tokens == 0 && cache_write_tokens == 0;
    if !partial_input_only {
        state.stream_lifecycle = None;
        state.streaming_last_token_at = Some(std::time::Instant::now());
    }
    // Floor the `responseLengthRef` accumulator up to the wire-truth output
    // count so the spinner's `bytes/4` token estimate is corrected upward and
    // then keeps growing by chars from there — cli.js's `i54` reducer
    // (`Math.max($, responseLengthBaseline + outputTokens*4)`). Doing the
    // correction *in the accumulator* (instead of `max(wire, bytes/4)` fresh
    // each render frame) is what stops the count from pinning flat to wire and
    // jumping ~50 every batched `message_delta`. Skip partial/metadata-only
    // events (output_tokens == 0): they carry no output count to floor against.
    if !partial_input_only {
        // A new sub-stream restarts the message's cumulative `output_tokens`
        // lower than the last event we saw → snapshot the accumulator as the
        // baseline so the floor continues from what prior sub-streams built.
        if output_tokens < state.last_usage_output {
            state.streaming_response_baseline = state.streaming_response_bytes;
        }
        // Self-heal a stale baseline left by a turn-boundary reset that zeroed
        // `streaming_response_bytes` without clearing the baseline.
        if state.streaming_response_baseline > state.streaming_response_bytes {
            state.streaming_response_baseline = 0;
        }
        let wire_floor = state.streaming_response_baseline + output_tokens as usize * 4;
        state.streaming_response_bytes = state.streaming_response_bytes.max(wire_floor);
        // True output tokens for accounting/persisted usage — accumulate the
        // real per-event delta. The visible status row reads
        // `streaming_response_bytes / 4` so it can advance smoothly between
        // provider usage batches. `output_tokens` is cumulative within a
        // sub-stream, so the delta vs the previous event is its growth; a
        // regression (a new sub-stream restarting lower) contributes its tokens
        // from zero. No chars/4 in this accounting field.
        let token_delta = if output_tokens >= state.last_usage_output {
            output_tokens - state.last_usage_output
        } else {
            output_tokens
        };
        state.turn_output_tokens = state.turn_output_tokens.saturating_add(token_delta as u64);
        if let Some(tokens) = thinking_tokens {
            state.streaming_thinking_tokens = tokens as u64;
        }
    }
    state.last_usage_input = input_tokens;
    state.last_usage_output = output_tokens;
    // v126's tokenCountWithEstimation uses input + cache_creation +
    // cache_read + output (all four count against the context window).
    // Previously this only summed input + output, under-reporting by
    // the cache contribution — which can be 50-80% of context on
    // prompt-cache-heavy sessions.
    let mut reported_total = input_tokens as usize
        + output_tokens as usize
        + cache_read_tokens as usize
        + cache_write_tokens as usize;

    // Post-compaction stale-cache guard. Anthropic's prompt cache survives a
    // compaction (~5-min TTL), so the first request(s) after a compact report
    // a `cache_read_tokens` reflecting the OLD, pre-compaction prefix — which
    // would re-inflate the gauge straight back to its pre-compact size even
    // though the conversation was just trimmed. While the ceiling is armed,
    // clamp the reported total to the freshly-compacted estimate. A genuine
    // `cache_write` (the model re-caching the new, smaller prefix) disarms the
    // guard, after which the real numbers flow through normally.
    if let Some(ceiling) = state.post_compact_token_ceiling {
        if cache_write_tokens > 0 {
            // The new (smaller) prefix is being cached — the next cache_read
            // will be accurate, so stop clamping.
            tracing::debug!(
                target: "jfc::compact",
                ceiling,
                cache_write_tokens,
                "post-compact cache rebuilt — disarming gauge ceiling"
            );
            state.post_compact_token_ceiling = None;
        } else if reported_total > ceiling {
            tracing::debug!(
                target: "jfc::compact",
                reported_total,
                ceiling,
                cache_read_tokens,
                "clamping gauge to post-compact ceiling (stale prompt cache)"
            );
            reported_total = ceiling;
        }
    }

    state.tool_ctx.approx_tokens = if partial_input_only {
        // ResponseMetadata can arrive before full Usage and carries only
        // input_tokens. Treat it as an early lower-bound so the context gauge
        // doesn't visibly drop from a calibrated cache-inclusive total to an
        // incomplete input-only value, then jump back on message_delta.
        state.tool_ctx.approx_tokens.max(reported_total)
    } else {
        reported_total
    };
    // Stamp the cumulative usage onto the streaming
    // assistant message. v126 attaches usage to each
    // assistant message (cli.js:416673) so on resume
    // `Wd(messages)` (cli.js:197282) can walk back to
    // recover the gauge total. We do the same: at
    // resume time the picker reads the last message's
    // `usage` rather than a default of 0.
    //
    // BUG FIX (2026-06-01): ResponseMetadata can arrive early with
    // partial_input_only={input_tokens, output_tokens:0}. The original
    // logic stamped this incomplete usage onto the message, and if the
    // stream failed before the final Usage event arrived, the message
    // was persisted with output_tokens:0 even though it contained actual
    // streamed content. Now we only update the message's usage if:
    // 1. It's a full Usage event (not partial_input_only), OR
    // 2. The message already has usage and this event is strictly better
    //
    // This prevents the partial early snapshot from permanently clobbering
    // the message's usage field on incomplete streams.
    if let Some(msg) = streaming_assistant_mut(state)
        && (!partial_input_only
            || msg
                .usage
                .as_ref()
                .is_some_and(|usage| usage.total_context_tokens() <= reported_total as u64))
    {
        msg.usage = Some(crate::types::ModelUsage {
            input_tokens: input_tokens as u64,
            output_tokens: output_tokens as u64,
            thinking_tokens: thinking_tokens.map(u64::from).unwrap_or_default(),
            cache_read_tokens: cache_read_tokens as u64,
            cache_write_tokens: cache_write_tokens as u64,
            cost_usd: None,
        });
    }
    let model_key = state.model.as_str().to_owned();
    let cum = (
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    );
    let usage_baseline = state.usage_apply_baseline;
    let usage = state.usage_by_model.entry(model_key).or_default();
    state.usage_apply_baseline = usage.apply_cumulative(cum, usage_baseline);
    if let Some(tokens) = thinking_tokens
        && !partial_input_only
    {
        usage.thinking_tokens = u64::from(tokens);
    }

    let expected_cache_drop = crate::cache_lineage::observe_cache_usage(
        state,
        input_tokens,
        cache_read_tokens,
        cache_write_tokens,
        partial_input_only,
    );

    // Cache diagnosis: detect significant cache invalidation.
    // When cache_read_tokens is zero but input_tokens is high,
    // something invalidated the prefix cache. Log so the tracing
    // output shows what happened (mirrors v144's
    // cache-diagnosis-2026-04-07 telemetry feature).
    if !expected_cache_drop && cache_read_tokens == 0 && input_tokens > 10_000 {
        tracing::info!(
            target: "jfc::cache_diagnosis",
            input_tokens,
            cache_read_tokens,
            cache_write_tokens,
            "prompt cache miss — entire prefix uncached this request \
             (likely cause: system prompt change, tool schema change, or model switch)"
        );
    } else if !expected_cache_drop && cache_write_tokens > 0 && cache_read_tokens == 0 {
        tracing::debug!(
            target: "jfc::cache_diagnosis",
            cache_write_tokens,
            "cache warming — writing new prefix to cache (first turn or invalidation)"
        );
    }
}

#[cfg(test)]
mod tests {
    use crate::app::EngineState;
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;
    use crate::types::ChatMessage;

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
    fn input_only_usage_keeps_pre_output_lifecycle_regression() {
        let mut state = test_app();
        state.stream_lifecycle = Some(crate::runtime::StreamLifecycleStatus::new(
            crate::runtime::StreamLifecyclePhase::StreamOpened,
            Some("waiting for first event".to_owned()),
        ));

        handle_stream_usage(&mut state, 42, 0, None, 0, 0);

        assert!(state.stream_lifecycle.is_some());
        assert_eq!(state.tool_ctx.approx_tokens, 42);
    }

    #[test]
    fn output_usage_clears_pre_output_lifecycle_normal() {
        let mut state = test_app();
        state.stream_lifecycle = Some(crate::runtime::StreamLifecycleStatus::new(
            crate::runtime::StreamLifecyclePhase::StreamOpened,
            Some("waiting for first event".to_owned()),
        ));
        state.streaming_last_token_at = None;

        handle_stream_usage(&mut state, 42, 3, None, 0, 0);

        assert!(state.stream_lifecycle.is_none());
        assert!(state.streaming_last_token_at.is_some());
    }

    #[test]
    fn post_compact_ceiling_clamps_stale_cache_read_normal() {
        // After compaction, a huge stale cache_read must NOT re-inflate the
        // gauge while the ceiling is armed.
        let mut state = test_app();
        state.post_compact_token_ceiling = Some(60_000);
        // Stale cache: cache_read reports the old 800k prefix, no cache_write.
        handle_stream_usage(&mut state, 2, 100, None, 800_000, 0);
        assert_eq!(
            state.tool_ctx.approx_tokens, 60_000,
            "stale cache_read should be clamped to the post-compact ceiling"
        );
        // Ceiling stays armed until a cache_write proves the new prefix cached.
        assert_eq!(state.post_compact_token_ceiling, Some(60_000));
    }

    #[test]
    fn post_compact_ceiling_disarms_on_cache_write_normal() {
        let mut state = test_app();
        state.post_compact_token_ceiling = Some(60_000);
        // A cache_write means the new (small) prefix is being cached → trust wire.
        handle_stream_usage(&mut state, 2, 100, None, 5_000, 4_000);
        assert_eq!(state.post_compact_token_ceiling, None);
        // Now the real (small) total flows through unclamped.
        assert_eq!(state.tool_ctx.approx_tokens, 2 + 100 + 5_000 + 4_000);
    }

    #[test]
    fn no_ceiling_means_wire_total_unclamped_robust() {
        let mut state = test_app();
        assert!(state.post_compact_token_ceiling.is_none());
        handle_stream_usage(&mut state, 1_000, 500, None, 200_000, 0);
        assert_eq!(state.tool_ctx.approx_tokens, 1_000 + 500 + 200_000);
    }

    #[test]
    fn partial_input_only_usage_does_not_lower_visible_context_regression() {
        let mut state = test_app();
        state.tool_ctx.approx_tokens = 120_000;

        handle_stream_usage(&mut state, 40_000, 0, None, 0, 0);

        assert_eq!(state.tool_ctx.approx_tokens, 120_000);
        assert_eq!(state.last_usage_input, 40_000);
        assert_eq!(state.last_usage_output, 0);
    }

    #[test]
    fn full_usage_replaces_partial_visible_context_normal() {
        let mut state = test_app();
        state.tool_ctx.approx_tokens = 120_000;
        handle_stream_usage(&mut state, 40_000, 0, None, 0, 0);

        handle_stream_usage(&mut state, 40_000, 2_000, None, 75_000, 5_000);

        assert_eq!(state.tool_ctx.approx_tokens, 122_000);
        assert_eq!(state.last_usage_input, 40_000);
        assert_eq!(state.last_usage_output, 2_000);
    }

    #[test]
    fn partial_input_only_usage_does_not_stamp_message_on_first_arrival_bugfix() {
        // BUG FIX (2026-06-01): ResponseMetadata arriving before chunks
        // should NOT stamp the message with output_tokens:0. If the stream
        // fails before the final Usage event, the message would persist
        // with output_tokens:0 even though it contains actual content.
        let mut state = test_app();
        state.messages.push(ChatMessage::assistant(String::new()));
        state.streaming_assistant_idx = Some(0);

        // Simulate ResponseMetadata arriving with input_tokens only
        handle_stream_usage(&mut state, 40_000, 0, None, 0, 0);

        // The message should NOT have usage stamped yet (prevents the bug)
        assert!(
            state.messages[0].usage.is_none(),
            "partial_input_only on first arrival must NOT stamp message"
        );

        // Later, final Usage event arrives with full data
        handle_stream_usage(&mut state, 40_000, 2_000, None, 75_000, 5_000);

        // Now the full usage is stamped
        let usage = state.messages[0].usage.as_ref().expect("usage");
        assert_eq!(usage.output_tokens, 2_000);
        assert_eq!(usage.total_context_tokens(), 122_000);
    }

    #[test]
    fn turn_output_tokens_tracks_true_wire_across_substreams_normal() {
        let mut state = test_app();
        // Input-only metadata arrives first — must not move the output count.
        handle_stream_usage(&mut state, 12, 0, None, 0, 0);
        assert_eq!(state.turn_output_tokens, 0);
        // Sub-stream 1: cumulative output grows; we accumulate the real delta.
        handle_stream_usage(&mut state, 12, 100, None, 0, 0);
        assert_eq!(state.turn_output_tokens, 100);
        handle_stream_usage(&mut state, 12, 250, None, 0, 0);
        assert_eq!(state.turn_output_tokens, 250);
        // Sub-stream 2 restarts output_tokens lower (a regression). Its tokens
        // are counted from zero so the turn total stays true and monotonic.
        handle_stream_usage(&mut state, 12, 30, None, 0, 0);
        assert_eq!(state.turn_output_tokens, 280);
        handle_stream_usage(&mut state, 12, 80, None, 0, 0);
        assert_eq!(state.turn_output_tokens, 330);
    }

    #[test]
    fn partial_input_only_usage_does_not_clobber_richer_message_usage_regression() {
        let mut state = test_app();
        state.messages.push(ChatMessage::assistant(String::new()));
        state.streaming_assistant_idx = Some(0);
        handle_stream_usage(&mut state, 40_000, 2_000, None, 75_000, 5_000);

        handle_stream_usage(&mut state, 41_000, 0, None, 0, 0);

        let usage = state.messages[0].usage.as_ref().expect("usage");
        assert_eq!(usage.total_context_tokens(), 122_000);
    }

    // --- responseLengthRef accumulator floor (spinner token count) ---

    #[test]
    fn usage_floors_response_accumulator_to_wire_normal() {
        // The displayed count is `streaming_response_bytes / 4`. A batched
        // `message_delta` reporting 200 output tokens must floor the
        // accumulator up to 200*4 = 800 bytes so the count reads 200 — even
        // if only a few chars have streamed so far.
        let mut state = test_app();
        state.streaming_response_bytes = 40; // 10 tokens of chars so far
        handle_stream_usage(&mut state, 1_000, 200, None, 0, 0);
        assert_eq!(state.streaming_response_bytes, 800);
        assert_eq!(state.streaming_response_bytes / 4, 200);
    }

    #[test]
    fn char_growth_continues_above_wire_floor_normal() {
        // After a wire floor to 200 tokens (800 bytes), streamed chars keep
        // adding *on top* — the count advances smoothly, it does not pin flat
        // to the next wire delta. This is the anti-"jumps by 50" guarantee.
        let mut state = test_app();
        handle_stream_usage(&mut state, 1_000, 200, None, 0, 0); // floor → 800
        state.streaming_response_bytes += 120; // 30 tokens of chars arrive → 920
        assert_eq!(state.streaming_response_bytes / 4, 230);
        // Next batched delta (210 tokens) is *below* the char-grown
        // accumulator, so it must NOT pull the count back down to 210.
        handle_stream_usage(&mut state, 1_000, 210, None, 0, 0);
        assert_eq!(
            state.streaming_response_bytes, 920,
            "wire must not lower a char-led count"
        );
        assert_eq!(state.streaming_response_bytes / 4, 230);
    }

    #[test]
    fn baseline_carries_across_substreams_robust() {
        // Sub-stream 1 reaches 200 output tokens (800 bytes). Sub-stream 2
        // restarts `output_tokens` at a lower cumulative; the baseline must
        // snapshot the accumulator so sub-stream 2's floor continues from it
        // rather than collapsing back to its own small count.
        let mut state = test_app();
        handle_stream_usage(&mut state, 1_000, 200, None, 0, 0); // → 800
        // New sub-stream: output restarts at 50 (< previous 200).
        handle_stream_usage(&mut state, 1_000, 50, None, 0, 0);
        assert_eq!(
            state.streaming_response_baseline, 800,
            "baseline snapshots prior accumulator"
        );
        // 50 more tokens this sub-stream → floor = 800 + 50*4 = 1000.
        assert_eq!(state.streaming_response_bytes, 1_000);
        assert_eq!(state.streaming_response_bytes / 4, 250);
    }

    #[test]
    fn final_thinking_usage_replaces_live_estimate_normal() {
        let mut state = test_app();
        state.streaming_thinking_tokens = 75_000;

        handle_stream_usage(&mut state, 1_000, 900, Some(63_250), 0, 0);

        assert_eq!(state.streaming_thinking_tokens, 63_250);
        let usage = state
            .usage_by_model
            .get("test-model")
            .expect("model usage should be recorded");
        assert_eq!(usage.thinking_tokens, 63_250);
        assert_eq!(usage.total_context_tokens(), 1_900);
    }

    #[test]
    fn stale_baseline_self_heals_after_turn_reset_robust() {
        // A turn boundary zeros `streaming_response_bytes` but (defensively)
        // may leave a stale baseline. The next usage event must clamp the
        // baseline back to 0 rather than over-flooring the fresh turn.
        let mut state = test_app();
        state.streaming_response_baseline = 5_000; // stale from a prior turn
        state.streaming_response_bytes = 0; // fresh turn
        handle_stream_usage(&mut state, 1_000, 30, None, 0, 0);
        assert_eq!(
            state.streaming_response_baseline, 0,
            "stale baseline self-heals"
        );
        assert_eq!(state.streaming_response_bytes, 120); // 30*4, not 5000+120
    }

    #[test]
    fn partial_usage_leaves_accumulator_untouched_robust() {
        // An input-only metadata event (output_tokens == 0) carries no output
        // count, so it must not touch the accumulator or baseline.
        let mut state = test_app();
        state.streaming_response_bytes = 640; // 160 tokens of content
        handle_stream_usage(&mut state, 50_000, 0, None, 0, 0);
        assert_eq!(state.streaming_response_bytes, 640);
        assert_eq!(state.streaming_response_baseline, 0);
    }
}
