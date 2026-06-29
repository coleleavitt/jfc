use super::App;

impl App {
    /// Recompute the `wants_animation_frame` flag based on current state.
    /// Called once per tick so the tick task can adjust its sleep interval.
    pub fn update_wants_animation_frame(&self) {
        use std::sync::atomic::Ordering;
        let any_alive_background = self
            .engine
            .background_tasks
            .values()
            .any(|bt| bt.status.is_alive());
        // The spinner row must *animate* exactly when it is *shown*, or its
        // glyph freezes between input events (the "dots only move when I move
        // the cursor" jank, most visible while a foreground bash tool runs).
        // These mirror `show_spinner` in `render::frame`: a turn is in flight
        // (covers the whole agentic loop, including tool waits), tools are
        // queued, or a compaction is running — none of which set
        // `is_streaming`, so without them the tick loop drops to the idle
        // cadence and stops redrawing mid-turn.
        let turn_active = self.engine.turn_started_at.is_some()
            || self.engine.compacting_started_at.is_some()
            || self.engine.pipeline_busy_for_submit()
            || self.engine.network_recovery_status.is_some()
            || self.engine.stream_lifecycle.is_some();
        let dominated = self.launched_at.elapsed() < std::time::Duration::from_millis(1500)
            || self.engine.is_streaming
            || turn_active
            || any_alive_background
            // Drag-edge autoscroll runs on the tick; keep ticks at the
            // animation cadence so the selection extends smoothly instead of
            // stepping at the 80ms idle rate.
            || self.drag_autoscroll.is_some()
            // Voice recording/processing drives the animated RMS cursor — keep
            // ticks at the animation cadence so the glyph + hue update smoothly.
            || self.voice_state != jfc_voice::VoiceState::Idle
            || self
                .engine
                .toasts
                .iter()
                .any(|t| {
                    matches!(t.kind, jfc_engine::toast::ToastKind::Error)
                        && !t.is_expired_at(std::time::Instant::now())
                });
        self.wants_animation_frame
            .store(dominated, Ordering::Relaxed);
    }

    /// Recompute `tool_ctx.approx_tokens` and the live-usage cache fields
    /// (`last_usage_input` / `last_usage_output`) from the current
    /// `messages`. Call after a session resume so the Context gauge and
    /// the pre-submit compact gate reflect the loaded conversation —
    /// without this, both read 0 until the next stream's `StreamUsage`
    /// event lands, and the pre-submit compact silently mis-estimates a
    /// huge resumed history as "fits".
    ///
    /// Strategy mirrors v126 `Wd(messages)` (cli.js:197282-197294): walk
    /// the messages backwards looking for the most recent assistant
    /// message with `usage` attached. If found, that's the authoritative
    /// resume baseline (matches what the wire reported). If not (e.g. a
    /// pre-usage-tracking session file), fall back to
    /// `compact::estimate_tokens` over message content — same heuristic
    /// the live token counter uses.
    pub fn recompute_token_estimate(&mut self) {
        let old_estimate = self.engine.tool_ctx.approx_tokens;
        // v126's `tokenCountWithEstimation` (tokens.ts:226-261): find the last
        // assistant message with API usage, use that as the authoritative base,
        // then rough-estimate any messages added AFTER it (user prompts, tool
        // results). This prevents the gap between API calls where the gauge
        // reads 0 or stale for newly-added messages.
        let last_usage_idx = self
            .engine
            .messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, m)| m.usage.as_ref().map(|u| (i, u.clone())));
        if let Some((idx, u)) = last_usage_idx {
            self.engine.last_usage_input = u.input_tokens as u32;
            self.engine.last_usage_output = u.output_tokens as u32;
            let base = u.total_context_tokens() as usize;
            // Estimate tokens for messages added after the usage-bearing
            // message — but exclude queued placeholders, since they
            // aren't actually in the prompt the model sees (see
            // `build_provider_messages`).
            let tail: Vec<jfc_core::ChatMessage> = self.engine.messages[idx + 1..]
                .iter()
                .filter(|m| !m.queued)
                .cloned()
                .collect();
            let tail_estimate = jfc_engine::compact::estimate_tokens(&tail);
            self.engine.tool_ctx.approx_tokens = base + tail_estimate;
        } else {
            self.engine.last_usage_input = 0;
            self.engine.last_usage_output = 0;
            // Same queued filter as above. Without this, queueing a
            // long prompt during a streaming turn would visibly bump
            // the context gauge even though that text isn't part of
            // the current prompt.
            let unqueued: Vec<jfc_core::ChatMessage> = self
                .engine
                .messages
                .iter()
                .filter(|m| !m.queued)
                .cloned()
                .collect();
            self.engine.tool_ctx.approx_tokens = jfc_engine::compact::estimate_tokens(&unqueued);
        }
        tracing::debug!(
            target: "jfc::app",
            old_estimate,
            new_estimate = self.engine.tool_ctx.approx_tokens,
            "recompute_token_estimate"
        );
    }

    // (clear_selection_on_scroll is gone: selections are stored in
    // scroll-invariant content-line coordinates now, so scrolling no longer
    // invalidates them — the renderer just paints the visible slice.)

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset))]
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.max_scroll();
        self.follow_bottom = true;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset))]
    pub fn scroll_to_top(&mut self) {
        self.scroll_offset = 0;
        self.follow_bottom = false;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset, lines))]
    pub fn scroll_up(&mut self, lines: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.follow_bottom = false;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset, lines))]
    pub fn scroll_down(&mut self, lines: usize) {
        let max = self.max_scroll();
        self.scroll_offset = (self.scroll_offset + lines).min(max);
        if self.scroll_offset >= max {
            self.follow_bottom = true;
        }
    }

    #[tracing::instrument(target = "jfc::app", skip(self))]
    pub fn scroll_page_up(&mut self) {
        let half = self.half_page();
        self.scroll_up(half);
    }

    #[tracing::instrument(target = "jfc::app", skip(self))]
    pub fn scroll_page_down(&mut self) {
        let half = self.half_page();
        self.scroll_down(half);
    }
    pub fn is_at_bottom(&self) -> bool {
        self.scroll_offset >= self.max_scroll()
    }

    /// Apply one tick of drag-edge autoscroll. While a drag-selection's cursor
    /// is pinned past the top/bottom edge of the transcript (`drag_autoscroll`
    /// holds the rows-past-edge overrun: negative = above the top edge → scroll
    /// up, positive = below the bottom edge → scroll down), scroll in that
    /// direction and pull the selection head to the freshly revealed edge row —
    /// so a drag can select content beyond the visible viewport (the
    /// never-resolved "copy is limited to the visible area" report).
    ///
    /// Returns `true` when it scrolled (the caller should redraw). This is the
    /// single source of truth for the gesture: `handle_tick` calls it each tick
    /// and the tests drive it directly, so the test can't drift from the real
    /// handler.
    ///
    /// The step grows with how far past the edge the cursor is (natural
    /// acceleration), capped so a big overrun can't teleport. Auto-following
    /// the bottom is NOT re-engaged by a downward copy-drag: `scroll_down` sets
    /// `follow_bottom` when it reaches the max, but a drag is a copy gesture,
    /// not a "pin to live output" request, so we restore the prior
    /// `follow_bottom` afterward.
    pub fn apply_drag_autoscroll_tick(&mut self) -> bool {
        let Some(overrun) = self.drag_autoscroll else {
            return false;
        };
        let step = (overrun.unsigned_abs() as usize).clamp(1, 6);
        let follow_before = self.follow_bottom;
        if overrun < 0 {
            self.scroll_up(step);
        } else {
            self.scroll_down(step);
            // A copy-drag must not silently re-arm auto-follow when it reaches
            // the bottom — the user is selecting text, not pinning to live
            // output. Keep whatever follow state they had before the drag.
            self.follow_bottom = follow_before;
        }
        // Re-anchor the selection head to the current edge content-line so the
        // highlight follows the revealed rows.
        if let (Some(area), Some(sel)) =
            (*self.messages_rect.borrow(), self.text_selection.as_mut())
        {
            let top = area.y;
            let bottom = area.y + area.height; // exclusive
            let edge_row = if overrun < 0 { top } else { bottom - 1 };
            let content_line = self.scroll_offset + edge_row.saturating_sub(top) as usize;
            sel.head = (sel.head.0, content_line);
        }
        true
    }

    fn max_scroll(&self) -> usize {
        self.total_lines.saturating_sub(self.viewport_height.max(1))
    }

    /// Switch sessions: engine-side reset plus the view-side resets that go
    /// with it (task panel selection, task drill-down state, token gauge).
    pub fn switch_session(&mut self, id: Option<jfc_engine::ids::SessionId>) {
        self.engine.switch_session(id);
        self.task_panel.reset_selection();
        self.task_panel.reset_drilldown();
        self.recompute_token_estimate();
    }

    fn half_page(&self) -> usize {
        (self.viewport_height / 2).max(1)
    }
}
