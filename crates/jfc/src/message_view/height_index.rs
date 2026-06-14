//! Persistent per-message height index for the virtualized transcript.
//!
//! Pre-virtualization, every frame rebuilt `RenderItem`s for the ENTIRE
//! transcript and re-hashed every message's full text just to compute
//! `total_lines` for scroll math — O(transcript) per frame, the dominant
//! main-thread cost on long sessions (observed: 94% CPU with a ~1100
//! message transcript).
//!
//! The index stores one entry per message: a cheap *fingerprint* (no full
//! text hash — parts count + total byte length + last-part byte length)
//! and the message's visual height at the indexed width. Heights come from
//! the SAME producer the painter uses (`build_message_items` +
//! `RenderItem::height`), preserving the one-producer-one-truth rule that
//! killed the old "fast-path predictor" drift bugs.
//!
//! Per frame:
//!   * revalidate fingerprints (cheap integer compares; only changed
//!     messages rebuild their items to re-measure height);
//!   * prefix-sum heights → `total_lines` + scroll math;
//!   * build `RenderItem`s ONLY for messages intersecting the visible
//!     window.
//!
//! Invalidations:
//!   * width change / theme-ish global change → `clear()` (the caller
//!     compares widths);
//!   * message content change → fingerprint mismatch on that entry;
//!   * expansion state (reasoning / tool groups) changes message layout
//!     without changing content — the fingerprint folds in the per-message
//!     expansion inputs so toggles invalidate exactly the toggled message;
//!   * the streaming message is ALWAYS re-measured (its text accrues
//!     between fingerprint checks within the same byte length only in
//!     pathological cases, but pacing also changes revealed lines, which
//!     is invisible to the fingerprint).

use jfc_core::{ChatMessage, MessagePart, Role};

use super::core::{RenderCtx, build_message_items};

/// Cheap per-message content fingerprint. Not a cryptographic identity —
/// just enough to detect the mutations the engine actually performs
/// (append a part, append text to the last part, flip a tool's status,
/// toggle expansion). Collisions require equal part counts, equal total
/// bytes AND equal last-part bytes with different content — not produced
/// by any append-only mutation path.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct MsgFingerprint {
    parts: usize,
    total_bytes: usize,
    last_part_bytes: usize,
    /// Folded layout inputs that change a message's height without
    /// changing its text: tool statuses/expansion, reasoning expansion,
    /// queued flag, elapsed footer.
    layout: u64,
}

fn part_bytes(p: &MessagePart) -> usize {
    // `approx_text_len` covers every variant (tool input summary + output
    // text, task description + summary, …) and is the same cheap metric
    // the token estimator uses — any content mutation that changes what's
    // on screen changes this count on append-only paths.
    p.approx_text_len().max(1)
}

/// Layout-affecting inputs that aren't text content. Folded with a simple
/// FNV-ish mix — we only need "changed → different", not distribution.
fn layout_word(ctx: &RenderCtx<'_>, idx: usize, msg: &ChatMessage) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut mix = |v: u64| {
        h ^= v;
        h = h.wrapping_mul(0x1000_0000_01b3);
    };
    mix(msg.queued as u64);
    mix(msg.elapsed.is_some() as u64);
    let reasoning_expanded = ctx
        .reasoning_expanded
        .get(&idx)
        .copied()
        .unwrap_or(ctx.active_reasoning_idx == Some(idx));
    mix(reasoning_expanded as u64);
    for part in &msg.parts {
        match part {
            MessagePart::Tool(tc) => {
                mix(tc.status as u64 + 1);
                mix(tc.display.is_expanded() as u64);
                mix(tc.elapsed_ms.is_some() as u64);
                // Group expansion is keyed "idx:first_tool_id" — fold the
                // membership test for this message's tools.
                let group_key = format!("{}:{}", idx, tc.id);
                mix(ctx.tool_group_expanded.contains(&group_key) as u64);
            }
            MessagePart::TaskStatus(ts) => {
                mix(ts.status as u64 + 1);
                mix(ts.elapsed_ms.unwrap_or(0) / 1000);
            }
            // Text/Reasoning/Advisor/RedactedThinking/CompactBoundary have
            // no layout state beyond their content — the byte counts in the
            // fingerprint already cover them.
            MessagePart::Text(_)
            | MessagePart::Reasoning(_)
            | MessagePart::Advisor(_)
            | MessagePart::RedactedThinking(_)
            | MessagePart::CompactBoundary { .. } => {}
        }
    }
    h
}

pub fn fingerprint(ctx: &RenderCtx<'_>, idx: usize, msg: &ChatMessage) -> MsgFingerprint {
    MsgFingerprint {
        parts: msg.parts.len(),
        total_bytes: msg.parts.iter().map(part_bytes).sum(),
        last_part_bytes: msg.parts.last().map(part_bytes).unwrap_or(0),
        layout: layout_word(ctx, idx, msg),
    }
}

#[derive(Clone, Copy, Debug)]
struct Entry {
    fp: MsgFingerprint,
    /// Visual rows this message contributes (including its trailing Blank
    /// separator). 0 for skipped messages (reminder-only user turns, the
    /// still-empty in-flight assistant message).
    height: usize,
    /// The `prev_role` value AFTER this message was measured — threading
    /// state for the same-speaker label suppression. Re-measurement of
    /// message N+1 needs the value from message N.
    prev_role_after: Option<Role>,
    /// Measured while this message was the live in-flight stream target.
    /// Such heights may reflect pacer-truncated text (revealed lines <
    /// full text) and the streaming-specific markdown fast path, while
    /// the fingerprint (byte counts) can already equal the final state —
    /// so the entry must NEVER be reused once streaming moves on.
    measured_streaming: bool,
}

/// Persistent height index. Owned by `App`; survives across frames.
#[derive(Default)]
pub struct HeightIndex {
    entries: Vec<Entry>,
    width: usize,
    /// Prefix sums: `prefix[i]` = total rows of messages `0..i`.
    /// `prefix[len]` = grand total. Rebuilt whenever any entry changes.
    prefix: Vec<usize>,
}

impl HeightIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
        self.prefix.clear();
        self.width = 0;
    }

    /// Revalidate the index against the current transcript, re-measuring
    /// only changed/new messages. Returns the total visual rows.
    pub fn sync(&mut self, ctx: &RenderCtx<'_>, inner_w: usize) -> usize {
        if self.width != inner_w {
            self.clear();
            self.width = inner_w;
        }
        self.entries.truncate(ctx.messages.len());

        let mut prev_role: Option<Role> = None;
        let mut dirty = self.prefix.len() != ctx.messages.len() + 1;
        // When a re-measured message ends with a different `prev_role_after`
        // than before, the NEXT message's same-speaker label suppression may
        // flip — force it to re-measure too so heights never drift from what
        // the painter will draw.
        let mut force_next = false;
        for (idx, msg) in ctx.messages.iter().enumerate() {
            let fp = fingerprint(ctx, idx, msg);
            let is_streaming = ctx.streaming_idx == Some(idx) && ctx.is_streaming;
            let reusable = !is_streaming
                && !force_next
                && self
                    .entries
                    .get(idx)
                    .is_some_and(|e| e.fp == fp && !e.measured_streaming);
            if reusable {
                prev_role = self.entries[idx].prev_role_after;
                continue;
            }
            // Re-measure: build this message's items at the paint width.
            let mut role_state = prev_role;
            let mut items = Vec::new();
            build_message_items(ctx, idx, msg, &mut role_state, inner_w, &mut items);
            let height: usize = items.iter().map(|i| i.height(inner_w)).sum();
            let entry = Entry {
                fp,
                height,
                prev_role_after: role_state,
                measured_streaming: is_streaming,
            };
            force_next = self
                .entries
                .get(idx)
                .is_some_and(|old| old.prev_role_after != role_state);
            if idx < self.entries.len() {
                self.entries[idx] = entry;
            } else {
                self.entries.push(entry);
            }
            prev_role = role_state;
            dirty = true;
        }

        if dirty {
            self.prefix.clear();
            self.prefix.reserve(self.entries.len() + 1);
            self.prefix.push(0);
            let mut acc = 0usize;
            for e in &self.entries {
                acc += e.height;
                self.prefix.push(acc);
            }
        }
        self.total()
    }

    pub fn total(&self) -> usize {
        self.prefix.last().copied().unwrap_or(0)
    }

    /// Rows of messages `0..idx`.
    pub fn rows_before(&self, idx: usize) -> usize {
        self.prefix
            .get(idx)
            .copied()
            .unwrap_or_else(|| self.total())
    }

    /// The half-open message range `[first, last)` intersecting the visual
    /// row window `[top, top + height)`, via binary search on the prefix
    /// sums. Also returns the row offset of `first`'s top edge.
    pub fn window(&self, top: usize, height: usize) -> (usize, usize, usize) {
        let n = self.entries.len();
        if n == 0 || height == 0 {
            return (0, 0, 0);
        }
        let bottom = top.saturating_add(height);
        // first = greatest i with prefix[i] <= top
        let first = match self.prefix.binary_search(&top) {
            Ok(i) => i.min(n.saturating_sub(1)),
            Err(i) => i.saturating_sub(1).min(n.saturating_sub(1)),
        };
        // Skip zero-height messages that sit exactly at `top` — walk first
        // back while the previous prefix equals the same row (they emit no
        // rows, including order doesn't change output but keeps prev_role
        // anchoring simple).
        let mut last = first;
        while last < n && self.prefix[last] < bottom {
            last += 1;
        }
        (first, last, self.prefix[first])
    }

    /// `prev_role` threading value to seed a windowed build starting at
    /// message `first` (the value after message `first - 1`).
    pub fn prev_role_before(&self, first: usize) -> Option<Role> {
        if first == 0 {
            return None;
        }
        self.entries.get(first - 1).and_then(|e| e.prev_role_after)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use jfc_core::ChatMessage;

    fn ctx_for(app: &App) -> RenderCtx<'_> {
        RenderCtx::from_app(app)
    }

    fn test_app(messages: Vec<ChatMessage>) -> App {
        use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
        use std::sync::Arc;
        struct Stub;
        #[async_trait::async_trait]
        impl Provider for Stub {
            fn name(&self) -> &str {
                "test"
            }
            fn available_models(&self) -> Vec<ModelInfo> {
                Vec::new()
            }
            async fn stream(
                &self,
                _: Vec<ProviderMessage>,
                _: &StreamOptions,
            ) -> anyhow::Result<EventStream> {
                Ok(Box::pin(futures::stream::empty()))
            }
        }
        impl jfc_provider::seal::Sealed for Stub {}
        let mut app = App::new(Arc::new(Stub), "test-model");
        app.engine.messages = messages;
        app
    }

    fn msgs(n: usize) -> Vec<ChatMessage> {
        (0..n)
            .flat_map(|i| {
                [
                    ChatMessage::user(format!("question {i}")),
                    ChatMessage::assistant(format!("answer {i}\nwith a second line")),
                ]
            })
            .collect()
    }

    #[test]
    fn total_matches_full_walk_normal() {
        let app = test_app(msgs(20));
        let ctx = ctx_for(&app);
        let mut index = HeightIndex::new();
        let total = index.sync(&ctx, 80);
        let full: usize = super::super::core::build_render_items_ctx(&ctx, 80)
            .iter()
            .map(|i| i.height(80))
            .sum();
        assert_eq!(total, full, "index total must equal the full-walk total");
    }

    #[test]
    fn resync_is_stable_normal() {
        let app = test_app(msgs(10));
        let ctx = ctx_for(&app);
        let mut index = HeightIndex::new();
        let t1 = index.sync(&ctx, 80);
        let t2 = index.sync(&ctx, 80);
        assert_eq!(t1, t2);
    }

    #[test]
    fn width_change_invalidates_robust() {
        // A long line wraps differently at different widths — totals differ.
        let mut m = msgs(1);
        m[1] = ChatMessage::assistant("word ".repeat(60));
        let app = test_app(m);
        let ctx = ctx_for(&app);
        let mut index = HeightIndex::new();
        let wide = index.sync(&ctx, 200);
        let narrow = index.sync(&ctx, 40);
        assert!(narrow > wide, "narrow {narrow} must exceed wide {wide}");
        // And the narrow total matches a fresh full walk at the same width.
        let full: usize = super::super::core::build_render_items_ctx(&ctx, 40)
            .iter()
            .map(|i| i.height(40))
            .sum();
        assert_eq!(narrow, full);
    }

    #[test]
    fn content_edit_invalidates_only_that_entry_normal() {
        let mut app = test_app(msgs(10));
        let mut index = HeightIndex::new();
        {
            let ctx = ctx_for(&app);
            index.sync(&ctx, 80);
        }
        // Append text to message 5 — its fingerprint changes.
        if let Some(jfc_core::MessagePart::Text(t)) = app.engine.messages[5].parts.last_mut() {
            t.push_str("\nmore\nlines\nhere");
        }
        let ctx = ctx_for(&app);
        let total = index.sync(&ctx, 80);
        let full: usize = super::super::core::build_render_items_ctx(&ctx, 80)
            .iter()
            .map(|i| i.height(80))
            .sum();
        assert_eq!(total, full, "edited transcript total must match full walk");
    }

    #[test]
    fn window_covers_visible_rows_normal() {
        let app = test_app(msgs(30));
        let ctx = ctx_for(&app);
        let mut index = HeightIndex::new();
        let total = index.sync(&ctx, 80);
        assert!(total > 20);
        // A window in the middle.
        let (first, last, first_top) = index.window(total / 2, 10);
        assert!(first < last && last <= app.engine.messages.len());
        assert!(first_top <= total / 2);
        // Window rows fully cover [top, top+10).
        assert!(index.rows_before(last) >= total / 2 + 10 || last == app.engine.messages.len());
    }

    #[test]
    fn windowed_total_equals_sum_of_entries_robust() {
        let app = test_app(msgs(15));
        let ctx = ctx_for(&app);
        let mut index = HeightIndex::new();
        let total = index.sync(&ctx, 80);
        // Entire range as one window.
        let (first, last, top) = index.window(0, total);
        assert_eq!((first, top), (0, 0));
        assert_eq!(last, app.engine.messages.len());
    }

    #[test]
    fn empty_transcript_robust() {
        let app = test_app(Vec::new());
        let ctx = ctx_for(&app);
        let mut index = HeightIndex::new();
        assert_eq!(index.sync(&ctx, 80), 0);
        assert_eq!(index.window(0, 24), (0, 0, 0));
    }

    #[test]
    fn streaming_entry_remeasured_after_stream_ends_robust() {
        let mut app = test_app(msgs(3));
        let last = app.engine.messages.len() - 1;
        app.engine.streaming_assistant_idx = Some(last);
        app.engine.is_streaming = true;
        let mut index = HeightIndex::new();
        {
            let ctx = ctx_for(&app);
            index.sync(&ctx, 80);
        }
        // Stream ends with identical bytes — the streaming-measured entry
        // must not be trusted (the streaming markdown fast path differs).
        app.engine.is_streaming = false;
        app.engine.streaming_assistant_idx = None;
        let ctx = ctx_for(&app);
        let total = index.sync(&ctx, 80);
        let full: usize = super::super::core::build_render_items_ctx(&ctx, 80)
            .iter()
            .map(|i| i.height(80))
            .sum();
        assert_eq!(total, full);
    }

    /// THE virtualization invariant: for every scroll offset, painting the
    /// windowed items with the window-relative scroll produces a buffer
    /// identical to painting ALL items with the absolute scroll.
    #[test]
    fn windowed_render_equals_full_render_robust() {
        use super::super::core::{
            MessageView, PrebuiltItems, build_render_items_ctx, build_render_items_window,
        };
        use ratatui::buffer::Buffer;
        use ratatui::layout::Rect;
        use ratatui::widgets::Widget;

        let app = test_app(msgs(12));
        let area = Rect::new(0, 0, 60, 12);
        let inner_w = area.width as usize;
        let viewport = area.height as usize;

        let ctx = ctx_for(&app);
        let mut index = HeightIndex::new();
        let total = index.sync(&ctx, inner_w);
        assert!(total > viewport, "fixture must overflow the viewport");

        for scroll in [0usize, 3, total / 2, total - viewport] {
            // Full path: all items, absolute scroll.
            let full_items = build_render_items_ctx(&ctx, inner_w);
            let mut full_buf = Buffer::empty(area);
            MessageView {
                app: &app,
                prebuilt: Some(PrebuiltItems {
                    items: full_items,
                    total_h: total,
                    scroll,
                }),
            }
            .render(area, &mut full_buf);

            // Windowed path: window items, window-relative scroll.
            let (first, last, top) = index.window(scroll, viewport);
            let prev_role = index.prev_role_before(first);
            let win_items = build_render_items_window(&ctx, inner_w, first, last, prev_role);
            let mut win_buf = Buffer::empty(area);
            MessageView {
                app: &app,
                prebuilt: Some(PrebuiltItems {
                    items: win_items,
                    total_h: total,
                    scroll: scroll - top,
                }),
            }
            .render(area, &mut win_buf);

            assert_eq!(
                full_buf, win_buf,
                "windowed paint must match full paint at scroll={scroll}"
            );
        }
    }
}
