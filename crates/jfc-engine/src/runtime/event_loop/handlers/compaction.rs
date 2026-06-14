//! `CompactionEvent::*` handlers. Hoisted out of the giant `event_loop::run`
//! match so the lifecycle (Started → Progress → Done | Failed) reads as a
//! single coherent file instead of four scattered match arms.

use crate::app::EngineState;
use crate::context::ToolContext;
use crate::runtime::{
    CompactionEvent, EventSender, drain_queued_prompts, maybe_continue_task_factory,
};
use crate::types::ChatMessage;
use crate::{stream, toast};

pub fn handle_started(state: &mut EngineState) {
    // The compacting_started_at guard is now set synchronously
    // at the decision site to prevent the agentic-loop race.
    // This event still fires for logging/observability but the
    // fields are already initialized — only set them if they
    // weren't (handles the edge case of manual /compact which
    // may not go through the AllToolsComplete path).
    tracing::debug!(target: "jfc::compact", "CompactionStarted event received — showing spinner");
    if state.compacting_started_at.is_none() {
        state.compacting_started_at = Some(std::time::Instant::now());
        state.compacting_output_chars = 0;
        state.compacting_attempt_baseline = 0;
        state.compacting_last_progress = 0;
    }
}

pub fn handle_progress(state: &mut EngineState, output_chars: u64) {
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
    if output_chars < state.compacting_last_progress {
        state.compacting_attempt_baseline += state.compacting_last_progress;
    }
    state.compacting_last_progress = output_chars;
    state.compacting_output_chars = state.compacting_attempt_baseline + output_chars;
}

pub async fn handle_done(
    state: &mut EngineState,
    tx: &EventSender,
    messages: Vec<ChatMessage>,
    tool_ctx: ToolContext,
    pre_tokens: usize,
    post_tokens: usize,
) {
    // Reset post-compact read tracker so we can detect re-reads.
    state.post_compact_reads.clear();
    let saved = pre_tokens.saturating_sub(post_tokens);
    // Stash the savings so the next outbound request forwards it as the
    // context-hint (context-hint-2026-04-09). Drained after one send. Only
    // worth hinting when non-trivial; the body builder enforces the 20k floor.
    if saved > 0 {
        state.pending_context_hint_tokens_saved = Some(saved as u64);
    }
    tracing::info!(
        target: "jfc::compact",
        pre_tokens, post_tokens, saved,
        new_message_count = messages.len(),
        "applying compaction result to app state"
    );
    let was_streaming = state.is_streaming;
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
        state.messages = messages;
        // Migrate cleanup flags (rapid_refill_count,
        // last_compact_turn, etc.) from the compact
        // worker's local tool_ctx, but preserve the
        // calibrated `approx_tokens` already on state —
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
        let preserved = state.tool_ctx.approx_tokens;
        state.tool_ctx = tool_ctx;
        // Use the smaller of (preserved calibrated value)
        // and post_tokens — preserved is wire-truth from
        // before compact, post_tokens is a local
        // estimate. After compaction the real prompt is
        // ≤ pre-compact; clamping to min protects against
        // showing the user a count larger than reality.
        state.tool_ctx.approx_tokens = preserved.min(post_tokens);
        // Add a fixed overhead estimate for system prompt, tool defs,
        // memories, etc. that the local message estimate doesn't include.
        // Without this, the gauge shows "safe" immediately post-compact
        // while the next request actually sends system+messages which can
        // be 50-100k+ of overhead.
        let overhead = state.last_system_prompt_len.unwrap_or(30_000);
        state.tool_ctx.approx_tokens = state.tool_ctx.approx_tokens.saturating_add(overhead);
        // Arm the post-compaction gauge ceiling. Anthropic's prompt cache
        // still holds the pre-compaction prefix for ~5 min, so the very next
        // request reports a `cache_read_tokens` ≈ the OLD (large) prefix —
        // which would snap the gauge right back to its pre-compact size
        // (the "compacts at 750k but never resets" bug). Clamp the gauge to
        // this freshly-compacted estimate (with generous headroom so a
        // genuinely growing post-compact turn isn't pinned low) until a real
        // cache_write proves the new, smaller prefix has been re-cached.
        let ceiling = state
            .tool_ctx
            .approx_tokens
            .saturating_mul(2)
            .max(state.tool_ctx.approx_tokens.saturating_add(50_000));
        state.post_compact_token_ceiling = Some(ceiling);
        state.last_usage_input = 0;
        // Reset the per-turn baseline so the next
        // `StreamUsage` cumulative delta builds from 0,
        // not from pre-compact totals — without this,
        // `apply_cumulative` would treat the post-compact
        // input as a negative delta and stall.
        state.usage_apply_baseline = (0, 0, 0, 0);
        // Repin to the bottom of the freshly-compacted transcript. The whole
        // message vec was just replaced, so any prior `scroll_offset` indexes
        // into a buffer that no longer exists; leaving `follow_bottom` false
        // (the user had scrolled up before a post-response or manual /compact)
        // would strand them mid-buffer staring at stale rows. Claude/OpenClaude
        // likewise repin on this transition — compaction is a hard transcript
        // reset, not an ordinary append. The content-addressed render cache
        // self-invalidates by text hash, so no explicit clear is needed.
        state.push_effect(crate::app::EngineEffect::ScrollToBottom);
    }
    state.compacting_started_at = None;
    state.compacting_output_chars = 0;
    state.compacting_attempt_baseline = 0;
    state.compacting_last_progress = 0;
    state.compact_suppressed = false;
    // Surface the compaction outcome to the user via a toast
    // — they don't have to scroll to see the boundary marker.
    let saved_k = saved / 1000;
    toast::push_with_cap(
        &mut state.toasts,
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
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        && !state
            .interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst)
        && !state.cancel_token.is_cancelled()
        && stream::should_continue_loop(&state.messages)
    {
        // Same mixed-mode gate as in AllToolsComplete:
        // if the original Done event flagged
        // pause_turn, route the resumed turn through
        // the pause-turn-resume builder so no
        // synthetic-Continue filler gets injected.
        // Single-shot semantics: clear the flag here so
        // a subsequent non-pause turn doesn't inherit
        // the routing.
        if state.pending_pause_turn_resume {
            state.pending_pause_turn_resume = false;
            tracing::info!(
                target: "jfc::stream",
                "agentic loop resuming after CompactionDone — pause_turn mixed mode, routing through continue_after_pause_turn"
            );
            stream::continue_after_pause_turn(state, tx).await;
        } else {
            tracing::info!(
                target: "jfc::stream",
                "agentic loop resuming after CompactionDone — tool results pending"
            );
            stream::continue_agentic_loop(state, tx).await;
        }
    } else if !was_streaming
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
    {
        // Compaction landed at end of turn (no pending
        // tool results). Drain queued prompts so they
        // start now that the context is clean.
        state.turn_started_at = None;
        drain_queued_prompts(state, tx).await;
        maybe_continue_task_factory(state, tx).await;
    }
}

pub async fn handle_failed(
    state: &mut EngineState,
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
        state.tool_ctx.approx_tokens = real_count;
    }
    state.compacting_started_at = None;
    state.compacting_output_chars = 0;
    state.compacting_attempt_baseline = 0;
    state.compacting_last_progress = 0;
    // Permanent failures (provider unsupported, exhausted retries,
    // breaker tripped) latch suppression so we stop spamming
    // compact attempts on every AllToolsComplete; the user clears
    // it explicitly with /compact. Transient failures (e.g.
    // TooFewGroups) self-resolve as the conversation grows, so
    // suppressing them would silently disable auto-compact for
    // the rest of the session.
    if !transient {
        state.compact_suppressed = true;
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
    toast::push_with_cap(&mut state.toasts, toast::Toast::new(toast_kind, toast_msg));

    // Re-check agentic loop continuation after failed compaction —
    // without this, tool results that triggered the compaction attempt
    // sit unreplied-to and the loop stalls permanently.
    if state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        && !state
            .interrupt_flag
            .load(std::sync::atomic::Ordering::SeqCst)
        && !state.cancel_token.is_cancelled()
        && stream::should_continue_loop(&state.messages)
    {
        tracing::info!(
            target: "jfc::compact",
            "agentic loop resuming after CompactionFailed — tool results pending"
        );
        stream::continue_agentic_loop(state, tx).await;
    }
}

// Glue: dispatch a `CompactionEvent` to the matching handler.
pub async fn handle_compaction_event(
    state: &mut EngineState,
    tx: &EventSender,
    ev: CompactionEvent,
) {
    match ev {
        CompactionEvent::Started => handle_started(state),
        CompactionEvent::Progress { output_chars } => handle_progress(state, output_chars),
        CompactionEvent::Done {
            messages,
            tool_ctx,
            pre_tokens,
            post_tokens,
        } => handle_done(state, tx, messages, tool_ctx, pre_tokens, post_tokens).await,
        CompactionEvent::Failed {
            reason,
            calibrated_tokens,
            transient,
        } => handle_failed(state, tx, reason, calibrated_tokens, transient).await,
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::types::Role;

    #[test]
    fn compact_boundary_is_user_role_invariant() {
        let b = ChatMessage::compact_boundary("s", 1);
        assert_eq!(b.role, Role::User);
        assert!(b.is_compact_boundary());
    }
}
