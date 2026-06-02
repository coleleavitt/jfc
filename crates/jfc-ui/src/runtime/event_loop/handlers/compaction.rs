//! `CompactionEvent::*` handlers. Hoisted out of the giant `event_loop::run`
//! match so the lifecycle (Started → Progress → Done | Failed) reads as a
//! single coherent file instead of four scattered match arms.

use crate::app::App;
use crate::context::ToolContext;
use crate::runtime::{
    CompactionEvent, EventSender, drain_queued_prompts, maybe_continue_task_factory,
};
use crate::types::ChatMessage;
use crate::{stream, toast};

pub(super) fn handle_started(app: &mut App) {
    // The compacting_started_at guard is now set synchronously
    // at the decision site to prevent the agentic-loop race.
    // This event still fires for logging/observability but the
    // fields are already initialized — only set them if they
    // weren't (handles the edge case of manual /compact which
    // may not go through the AllToolsComplete path).
    tracing::debug!(target: "jfc::compact", "CompactionStarted event received — showing spinner");
    if app.compacting_started_at.is_none() {
        app.compacting_started_at = Some(std::time::Instant::now());
        app.compacting_output_chars = 0;
        app.compacting_attempt_baseline = 0;
        app.compacting_last_progress = 0;
    }
}

pub(super) fn handle_progress(app: &mut App, output_chars: u64) {
    // Live token feedback during compact streaming. Mirrors
    // v126's PB7 addResponseLength → spinner refresh
    // (cli.js:396989).
    //
    // `compact()` retries internally when post_tokens is
    // still over the Blocked threshold or the model returns
    // a truncated summary. Each retry streams a fresh
    // response from 0 chars, so the per-attempt counter
    // regresses. Detect that and bump a baseline so the
    // spinner shows a monotonically-increasing total — the
    // user sees the true work-done across attempts instead
    // of a flickering counter that jumps `↓3k → ↓92 → ↓1k`.
    if output_chars < app.compacting_last_progress {
        app.compacting_attempt_baseline += app.compacting_last_progress;
    }
    app.compacting_last_progress = output_chars;
    app.compacting_output_chars = app.compacting_attempt_baseline + output_chars;
}

pub(super) async fn handle_done(
    app: &mut App,
    tx: &EventSender,
    messages: Vec<ChatMessage>,
    tool_ctx: ToolContext,
    pre_tokens: usize,
    post_tokens: usize,
) {
    // Reset post-compact read tracker so we can detect re-reads.
    app.post_compact_reads.clear();
    let saved = pre_tokens.saturating_sub(post_tokens);
    // Stash the savings so the next outbound request forwards it as the
    // context-hint (context-hint-2026-04-09). Drained after one send. Only
    // worth hinting when non-trivial; the body builder enforces the 20k floor.
    if saved > 0 {
        app.pending_context_hint_tokens_saved = Some(saved as u64);
    }
    tracing::info!(
        target: "jfc::compact",
        pre_tokens, post_tokens, saved,
        new_message_count = messages.len(),
        "applying compaction result to app state"
    );
    let was_streaming = app.is_streaming;
    if was_streaming {
        // Defensive: should be unreachable with the synchronous
        // compacting_started_at guard, but if a stream somehow
        // started during compaction, don't clobber live state.
        tracing::error!(
            target: "jfc::compact",
            "CompactionDone arrived while streaming — \
             discarding compaction result to avoid data corruption"
        );
    } else {
        app.messages = messages;
        // Migrate cleanup flags (rapid_refill_count,
        // last_compact_turn, etc.) from the compact
        // worker's local tool_ctx, but preserve the
        // calibrated `approx_tokens` already on app —
        // either the wire-reported value from the most
        // recent `StreamUsage` or the resume-time anchor
        // from `recompute_token_estimate`. Overwriting
        // with the post-compaction chars-based estimate
        // (`post_tokens`) created a down-then-up flicker:
        // gauge would drop to the local estimate (e.g.
        // 60k) and then the next stream's first
        // `StreamUsage` would snap it back to the
        // wire-truth (e.g. 500k, dominated by cache_read
        // of the still-cached system prompt + tool defs).
        // Recompute from messages so the visible value
        // reflects what's actually about to be sent on
        // the next turn — both compaction and the
        // pre-submit gate now use the same source.
        let preserved = app.tool_ctx.approx_tokens;
        app.tool_ctx = tool_ctx;
        // Use the smaller of (preserved calibrated value)
        // and post_tokens — preserved is wire-truth from
        // before compact, post_tokens is a local
        // estimate. After compaction the real prompt is
        // ≤ pre-compact; clamping to min protects against
        // showing the user a count larger than reality.
        app.tool_ctx.approx_tokens = preserved.min(post_tokens);
        // Add a fixed overhead estimate for system prompt, tool defs,
        // memories, etc. that the local message estimate doesn't include.
        // Without this, the gauge shows "safe" immediately post-compact
        // while the next request actually sends system+messages which can
        // be 50-100k+ of overhead.
        let overhead = app.last_system_prompt_len.unwrap_or(30_000);
        app.tool_ctx.approx_tokens = app.tool_ctx.approx_tokens.saturating_add(overhead);
        app.last_usage_input = 0;
        // Reset the per-turn baseline so the next
        // `StreamUsage` cumulative delta builds from 0,
        // not from pre-compact totals — without this,
        // `apply_cumulative` would treat the post-compact
        // input as a negative delta and stall.
        app.usage_apply_baseline = (0, 0, 0, 0);
        // Repin to the bottom of the freshly-compacted transcript. The whole
        // message vec was just replaced, so any prior `scroll_offset` indexes
        // into a buffer that no longer exists; leaving `follow_bottom` false
        // (the user had scrolled up before a post-response or manual /compact)
        // would strand them mid-buffer staring at stale rows. Claude/OpenClaude
        // likewise repin on this transition — compaction is a hard transcript
        // reset, not an ordinary append. The content-addressed render cache
        // self-invalidates by text hash, so no explicit clear is needed.
        app.scroll_to_bottom();
    }
    app.compacting_started_at = None;
    app.compacting_output_chars = 0;
    app.compacting_attempt_baseline = 0;
    app.compacting_last_progress = 0;
    app.compact_suppressed = false;
    // Surface the compaction outcome to the user via a toast
    // — they don't have to scroll to see the boundary marker.
    let saved_k = saved / 1000;
    toast::push_with_cap(
        &mut app.toasts,
        toast::Toast::new(
            toast::ToastKind::Success,
            format!("Compacted — saved ~{saved_k}k tokens"),
        ),
    );
    // Resume any deferred agentic continuation. When
    // compaction was triggered from `AllToolsComplete`,
    // that handler's continuation check skipped because
    // `compacting_started_at.is_some()`. Without this
    // resume the user's tool result never feeds back into
    // the model — the turn silently dies right after the
    // "Compacted" toast and queued prompts back up while
    // the spinner hangs. Mirror AllToolsComplete's gate:
    // continue only if the transcript ends on
    // tool_results (should_continue_loop=true) and
    // there's no other reason to pause.
    if !was_streaming
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty()
        && !app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst)
        && !app.cancel_token.is_cancelled()
        && stream::should_continue_loop(&app.messages)
    {
        // Same mixed-mode gate as in AllToolsComplete:
        // if the original Done event flagged
        // pause_turn, route the resumed turn through
        // the pause-turn-resume builder so no
        // synthetic-Continue filler gets injected.
        // Single-shot semantics: clear the flag here so
        // a subsequent non-pause turn doesn't inherit
        // the routing.
        if app.pending_pause_turn_resume {
            app.pending_pause_turn_resume = false;
            tracing::info!(
                target: "jfc::stream",
                "agentic loop resuming after CompactionDone — pause_turn mixed mode, routing through continue_after_pause_turn"
            );
            stream::continue_after_pause_turn(app, tx).await;
        } else {
            tracing::info!(
                target: "jfc::stream",
                "agentic loop resuming after CompactionDone — tool results pending"
            );
            stream::continue_agentic_loop(app, tx).await;
        }
    } else if !was_streaming
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty()
    {
        // Compaction landed at end of turn (no pending
        // tool results). Drain queued prompts so they
        // start now that the context is clean.
        app.turn_started_at = None;
        drain_queued_prompts(app, tx).await;
        maybe_continue_task_factory(app, tx).await;
    }
}

pub(super) async fn handle_failed(
    app: &mut App,
    tx: &EventSender,
    reason: String,
    calibrated_tokens: Option<usize>,
    transient: bool,
) {
    tracing::warn!(
        target: "jfc::compact",
        %reason,
        ?calibrated_tokens,
        transient,
        "compaction failed — surfacing toast to user"
    );
    if let Some(real_count) = calibrated_tokens {
        app.tool_ctx.approx_tokens = real_count;
    }
    app.compacting_started_at = None;
    app.compacting_output_chars = 0;
    app.compacting_attempt_baseline = 0;
    app.compacting_last_progress = 0;
    // Permanent failures (provider unsupported, exhausted retries,
    // breaker tripped) latch suppression so we stop spamming
    // compact attempts on every AllToolsComplete; the user clears
    // it explicitly with /compact. Transient failures (e.g.
    // TooFewGroups) self-resolve as the conversation grows, so
    // suppressing them would silently disable auto-compact for
    // the rest of the session.
    if !transient {
        app.compact_suppressed = true;
        crate::notifications::notify_compact_failed(&reason);
    }
    let toast_kind = if transient {
        toast::ToastKind::Info
    } else {
        toast::ToastKind::Error
    };
    let toast_msg = if transient {
        reason.clone()
    } else {
        format!("Compaction failed: {reason}")
    };
    toast::push_with_cap(&mut app.toasts, toast::Toast::new(toast_kind, toast_msg));

    // Re-check agentic loop continuation after failed compaction —
    // without this, tool results that triggered the compaction attempt
    // sit unreplied-to and the loop stalls permanently.
    if app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty()
        && !app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst)
        && !app.cancel_token.is_cancelled()
        && stream::should_continue_loop(&app.messages)
    {
        tracing::info!(
            target: "jfc::compact",
            "agentic loop resuming after CompactionFailed — tool results pending"
        );
        stream::continue_agentic_loop(app, tx).await;
    }
}

// Glue: dispatch a `CompactionEvent` to the matching handler.
pub(crate) async fn handle_compaction_event(app: &mut App, tx: &EventSender, ev: CompactionEvent) {
    match ev {
        CompactionEvent::Started => handle_started(app),
        CompactionEvent::Progress { output_chars } => handle_progress(app, output_chars),
        CompactionEvent::Done {
            messages,
            tool_ctx,
            pre_tokens,
            post_tokens,
        } => handle_done(app, tx, messages, tool_ctx, pre_tokens, post_tokens).await,
        CompactionEvent::Failed {
            reason,
            calibrated_tokens,
            transient,
        } => handle_failed(app, tx, reason, calibrated_tokens, transient).await,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;
    use crate::context::ToolContext;
    use crate::types::Role;

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
            #[allow(dead_code)] _messages: Vec<ProviderMessage>,
            #[allow(dead_code)] _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl jfc_provider::seal::Sealed for TestProvider {}

    fn test_app() -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = jfc_session::TaskStore::in_memory();
        app
    }

    // Regression (fix #4): after compaction replaces the whole message vec,
    // any prior `scroll_offset` indexes a buffer that no longer exists. The
    // user who had scrolled up (follow_bottom=false) before a /compact must be
    // repinned to the bottom, not stranded mid-buffer on stale rows.
    #[tokio::test]
    async fn compaction_done_repins_scroll_to_bottom_robust() {
        let mut app = test_app();
        // Simulate a user who scrolled up before compaction.
        app.total_lines = 500;
        app.viewport_height = 20;
        app.scroll_offset = 5;
        app.follow_bottom = false;
        app.is_streaming = false;

        let compacted = vec![
            ChatMessage::compact_boundary("summary of the session so far", 120_000),
            ChatMessage::assistant("resumed reply".into()),
        ];
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_done(
            &mut app,
            &tx,
            compacted,
            ToolContext::new(),
            120_000,
            20_000,
        )
        .await;

        assert!(
            app.follow_bottom,
            "compaction is a hard transcript reset — follow_bottom must re-arm"
        );
        // scroll_to_bottom() pins to max_scroll() = total_lines - viewport_height
        // (500 - 20 = 480), clearing the stale offset of 5.
        assert_eq!(
            app.scroll_offset, 480,
            "scroll_offset must be repinned to the bottom of the new transcript"
        );
    }

    // Defensive: if a stream is somehow live when CompactionDone arrives, the
    // result is discarded — and we must NOT touch scroll state in that path.
    #[tokio::test]
    async fn compaction_done_while_streaming_does_not_repin_edge() {
        let mut app = test_app();
        app.is_streaming = true;
        app.follow_bottom = false;
        app.scroll_offset = 5;
        let before = app.messages.len();
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_done(
            &mut app,
            &tx,
            vec![ChatMessage::assistant("ignored".into())],
            ToolContext::new(),
            120_000,
            20_000,
        )
        .await;

        assert_eq!(app.messages.len(), before, "result must be discarded");
        assert!(!app.follow_bottom, "discard path must not repin");
        assert_eq!(app.scroll_offset, 5);
    }

    #[test]
    fn compact_boundary_is_user_role_invariant() {
        let b = ChatMessage::compact_boundary("s", 1);
        assert_eq!(b.role, Role::User);
        assert!(b.is_compact_boundary());
    }
}
