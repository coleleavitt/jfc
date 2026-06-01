//! `StreamEvent::Usage { ... }` handler — token accounting.

use crate::app::App;

use super::super::guards::streaming_assistant_mut;

/// Handle `StreamEvent::Usage { input_tokens, output_tokens, cache_read_tokens, cache_write_tokens }`.
pub(crate) fn handle_stream_usage(
    app: &mut App,
    input_tokens: u32,
    output_tokens: u32,
    cache_read_tokens: u32,
    cache_write_tokens: u32,
) {
    app.record_stream_activity();
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
    app.last_usage_input = input_tokens;
    app.last_usage_output = output_tokens;
    // v126's tokenCountWithEstimation uses input + cache_creation +
    // cache_read + output (all four count against the context window).
    // Previously this only summed input + output, under-reporting by
    // the cache contribution — which can be 50-80% of context on
    // prompt-cache-heavy sessions.
    let reported_total = input_tokens as usize
        + output_tokens as usize
        + cache_read_tokens as usize
        + cache_write_tokens as usize;
    app.tool_ctx.approx_tokens = if partial_input_only {
        // ResponseMetadata can arrive before full Usage and carries only
        // input_tokens. Treat it as an early lower-bound so the context gauge
        // doesn't visibly drop from a calibrated cache-inclusive total to an
        // incomplete input-only value, then jump back on message_delta.
        app.tool_ctx.approx_tokens.max(reported_total)
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
    if let Some(msg) = streaming_assistant_mut(app)
        && (!partial_input_only
            || msg
                .usage
                .as_ref()
                .is_some_and(|usage| usage.total_context_tokens() <= reported_total as u64))
    {
        msg.usage = Some(crate::types::ModelUsage {
            input_tokens: input_tokens as u64,
            output_tokens: output_tokens as u64,
            cache_read_tokens: cache_read_tokens as u64,
            cache_write_tokens: cache_write_tokens as u64,
            cost_usd: None,
        });
    }
    let model_key = app.model.as_str().to_owned();
    let cum = (
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
    );
    app.usage_apply_baseline = app
        .usage_by_model
        .entry(model_key)
        .or_default()
        .apply_cumulative(cum, app.usage_apply_baseline);

    // Cache diagnosis: detect significant cache invalidation.
    // When cache_read_tokens is zero but input_tokens is high,
    // something invalidated the prefix cache. Log so the tracing
    // output shows what happened (mirrors v144's
    // cache-diagnosis-2026-04-07 telemetry feature).
    if cache_read_tokens == 0 && input_tokens > 10_000 {
        tracing::info!(
            target: "jfc::cache_diagnosis",
            input_tokens,
            cache_read_tokens,
            cache_write_tokens,
            "prompt cache miss — entire prefix uncached this request \
             (likely cause: system prompt change, tool schema change, or model switch)"
        );
    } else if cache_write_tokens > 0 && cache_read_tokens == 0 {
        tracing::debug!(
            target: "jfc::cache_diagnosis",
            cache_write_tokens,
            "cache warming — writing new prefix to cache (first turn or invalidation)"
        );
    }
}

#[cfg(test)]
mod tests {
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

    fn test_app() -> App {
        App::new(Arc::new(TestProvider), "test-model")
    }

    #[test]
    fn partial_input_only_usage_does_not_lower_visible_context_regression() {
        let mut app = test_app();
        app.tool_ctx.approx_tokens = 120_000;

        handle_stream_usage(&mut app, 40_000, 0, 0, 0);

        assert_eq!(app.tool_ctx.approx_tokens, 120_000);
        assert_eq!(app.last_usage_input, 40_000);
        assert_eq!(app.last_usage_output, 0);
    }

    #[test]
    fn full_usage_replaces_partial_visible_context_normal() {
        let mut app = test_app();
        app.tool_ctx.approx_tokens = 120_000;
        handle_stream_usage(&mut app, 40_000, 0, 0, 0);

        handle_stream_usage(&mut app, 40_000, 2_000, 75_000, 5_000);

        assert_eq!(app.tool_ctx.approx_tokens, 122_000);
        assert_eq!(app.last_usage_input, 40_000);
        assert_eq!(app.last_usage_output, 2_000);
    }

    #[test]
    fn partial_input_only_usage_does_not_stamp_message_on_first_arrival_bugfix() {
        // BUG FIX (2026-06-01): ResponseMetadata arriving before chunks
        // should NOT stamp the message with output_tokens:0. If the stream
        // fails before the final Usage event, the message would persist
        // with output_tokens:0 even though it contains actual content.
        let mut app = test_app();
        app.messages.push(ChatMessage::assistant(String::new()));
        app.streaming_assistant_idx = Some(0);

        // Simulate ResponseMetadata arriving with input_tokens only
        handle_stream_usage(&mut app, 40_000, 0, 0, 0);

        // The message should NOT have usage stamped yet (prevents the bug)
        assert!(
            app.messages[0].usage.is_none(),
            "partial_input_only on first arrival must NOT stamp message"
        );

        // Later, final Usage event arrives with full data
        handle_stream_usage(&mut app, 40_000, 2_000, 75_000, 5_000);

        // Now the full usage is stamped
        let usage = app.messages[0].usage.as_ref().expect("usage");
        assert_eq!(usage.output_tokens, 2_000);
        assert_eq!(usage.total_context_tokens(), 122_000);
    }

    #[test]
    fn partial_input_only_usage_does_not_clobber_richer_message_usage_regression() {
        let mut app = test_app();
        app.messages.push(ChatMessage::assistant(String::new()));
        app.streaming_assistant_idx = Some(0);
        handle_stream_usage(&mut app, 40_000, 2_000, 75_000, 5_000);

        handle_stream_usage(&mut app, 41_000, 0, 0, 0);

        let usage = app.messages[0].usage.as_ref().expect("usage");
        assert_eq!(usage.total_context_tokens(), 122_000);
    }
}
