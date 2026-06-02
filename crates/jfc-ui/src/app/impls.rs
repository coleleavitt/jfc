use std::time::Instant;

use crate::types::{MessagePart, Role, ToolCall, ToolKind};
use jfc_provider::ModelInfo;

use super::state::{
    DEFERRED_TOOL_USES_CAP, DeferredToolUse, TOOL_USE_SUMMARIES_CAP, ToolUseSummary,
};
use super::{App, PermissionDecision, STREAM_WATCHDOG_TIMEOUT_SECS};

impl App {
    /// Recompute the `wants_animation_frame` flag based on current state.
    /// Called once per tick so the tick task can adjust its sleep interval.
    pub fn update_wants_animation_frame(&self) {
        use std::sync::atomic::Ordering;
        let any_alive_background = self
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
        let turn_active = self.turn_started_at.is_some()
            || self.compacting_started_at.is_some()
            || !self.pending_tool_calls.is_empty()
            || self.network_recovery_status.is_some();
        let dominated = self.launched_at.elapsed() < std::time::Duration::from_millis(1500)
            || self.is_streaming
            || turn_active
            || any_alive_background
            || self.scroll_velocity.abs() > 0.5
            || self
                .toasts
                .iter()
                .any(|t| !t.is_expired_at(std::time::Instant::now()));
        self.wants_animation_frame
            .store(dominated, Ordering::Relaxed);
    }

    pub fn record_stream_activity(&mut self) {
        self.last_stream_event_at = Some(Instant::now());
    }

    pub fn pipeline_busy_for_submit(&self) -> bool {
        self.compacting_started_at.is_some()
            || self.pending_approval.is_some()
            || !self.approval_queue.is_empty()
            || !self.pending_tool_calls.is_empty()
            || self.pending_classifications > 0
            || self.in_flight_eager_dispatches > 0
            || self.in_flight_tool_batches > 0
            || !self.in_progress_tool_use_ids.is_empty()
    }

    pub fn has_interruptible_work(&self) -> bool {
        self.is_streaming
            || self
                .active_stream_handle
                .as_ref()
                .is_some_and(|handle| !handle.is_finished())
            || self.turn_started_at.is_some()
            || self.pipeline_busy_for_submit()
            || self.goal_evaluator_in_flight
            || self
                .background_tasks
                .values()
                .any(|bt| bt.status.is_alive())
    }

    pub fn record_deferred_tool_use(
        &mut self,
        id: String,
        name: String,
        input_preview: String,
        reason: String,
    ) {
        if let Some(existing) = self
            .deferred_tool_uses
            .iter_mut()
            .find(|deferred| deferred.id == id)
        {
            existing.name = name;
            existing.input_preview = input_preview;
            existing.reason = reason;
            existing.queued_at = Instant::now();
            return;
        }
        if self.deferred_tool_uses.len() >= DEFERRED_TOOL_USES_CAP {
            self.deferred_tool_uses.pop_front();
        }
        self.deferred_tool_uses.push_back(DeferredToolUse {
            id,
            name,
            input_preview,
            reason,
            queued_at: Instant::now(),
        });
    }

    pub fn clear_deferred_tool_use(&mut self, id: &str) {
        self.deferred_tool_uses.retain(|deferred| deferred.id != id);
    }

    pub fn set_in_progress_tool_use_ids(&mut self, action: &str, ids: &[String]) {
        match action {
            "set" => {
                self.in_progress_tool_use_ids.clear();
                self.in_progress_tool_use_ids.extend(ids.iter().cloned());
                for id in ids {
                    self.clear_deferred_tool_use(id);
                }
            }
            "add" => {
                for id in ids {
                    self.in_progress_tool_use_ids.insert(id.clone());
                    self.clear_deferred_tool_use(id);
                }
            }
            "remove" => {
                for id in ids {
                    self.in_progress_tool_use_ids.remove(id);
                    self.clear_deferred_tool_use(id);
                }
            }
            other => {
                tracing::warn!(
                    target: "jfc::tool_state",
                    action = other,
                    ids = ?ids,
                    "unknown set_in_progress_tool_use_ids action"
                );
            }
        }
    }

    pub fn record_tool_use_summary(
        &mut self,
        summary: String,
        preceding_tool_use_ids: Vec<String>,
    ) {
        if summary.trim().is_empty() || preceding_tool_use_ids.is_empty() {
            return;
        }
        if self.tool_use_summaries.len() >= TOOL_USE_SUMMARIES_CAP {
            self.tool_use_summaries.pop_front();
        }
        self.tool_use_summaries.push_back(ToolUseSummary {
            summary,
            preceding_tool_use_ids,
            created_at: Instant::now(),
        });
    }

    pub fn check_stream_watchdog(&mut self) {
        if !self.is_streaming {
            return;
        }
        let Some(timeout_secs) = stream_watchdog_timeout_secs() else {
            return;
        };
        let timed_out = self
            .last_stream_event_at
            .map(|t| t.elapsed().as_secs() >= timeout_secs)
            .unwrap_or(false);
        if timed_out {
            let streaming_assistant_idx = self.streaming_assistant_idx;
            tracing::warn!(
                target: "jfc::app",
                elapsed_secs = self.last_stream_event_at.map(|t| t.elapsed().as_secs()).unwrap_or(0),
                "stream watchdog: cancelling hard-idle stream"
            );
            // Cancel the stream task so it actually stops sending events.
            // Without this the stream task continues running in the
            // background, can still modify messages, and can dispatch
            // tools into a stale context — the "half-dead state" bug.
            self.cancel_token.cancel();
            // Belt-and-suspenders: forcefully abort the spawned driver
            // task too. The cooperative cancel above only stops the
            // task if it polls `cancel_token`. A task wedged inside a
            // blocking syscall (sync DNS lookup, sync audit-log write)
            // never reaches a `.cancelled()` check, so the next user
            // submission would race a second concurrent stream task
            // writing the same conversation buffer — interleaved
            // assistant prose. `JoinHandle::abort` schedules a forced
            // unwind at the next await point.
            if let Some(handle) = self.active_stream_handle.take() {
                handle.abort();
            }
            // CRITICAL: replace the token after cancelling so the NEXT
            // user submission gets a fresh, uncancelled token. Without
            // this, every subsequent stream would immediately see
            // `is_cancelled() == true` and emit "Interrupted by user"
            // — that was the spurious-interrupt bug. The previous user
            // submission's cancel flowed forward forever because the
            // token is a single shared instance, not per-turn.
            self.cancel_token = tokio_util::sync::CancellationToken::new();
            self.is_streaming = false;
            self.streaming_started_at = None;
            self.last_stream_event_at = None;
            self.streaming_last_token_at = None;
            self.token_rate_samples.clear();
            self.thinking_started_at = None;
            self.thinking_ended_at = None;
            self.streaming_text.clear();
            self.streaming_reasoning.clear();
            self.streaming_response_bytes = 0;
            self.streaming_assistant_idx = None;
            self.current_stream_request = None;
            self.turn_started_at = None;
            // Clear any pending tool calls that accumulated during the
            // dead stream — they're stale and would dispatch into wrong
            // context if processed later.
            self.pending_tool_calls.clear();
            self.pre_dispatched_tool_ids.clear();
            self.deferred_tool_uses.clear();
            self.in_progress_tool_use_ids.clear();
            self.in_flight_eager_dispatches = 0;
            self.in_flight_tool_batches = 0;
            if let Some(idx) = streaming_assistant_idx
                && idx < self.messages.len()
            {
                let msg = &self.messages[idx];
                let empty_stream_placeholder = msg.role == Role::Assistant
                    && msg.parts.iter().all(
                        |part| matches!(part, MessagePart::Text(text) if text.trim().is_empty()),
                    );
                if empty_stream_placeholder {
                    self.messages.remove(idx);
                }
            }
        }
    }

    /// Resolve the git repository root by walking up from `cwd`.
    /// Caches the result in `self.git_root`. Call `invalidate_git_root()`
    /// on Resize to force re-resolution.
    pub fn resolve_git_root(&mut self) {
        if self.git_root.is_some() {
            return;
        }
        let mut dir = std::env::current_dir().ok();
        while let Some(d) = dir {
            if d.join(".git").exists() {
                self.git_root = Some(Some(d));
                return;
            }
            dir = d.parent().map(|p| p.to_path_buf());
        }
        self.git_root = Some(None);
    }

    /// Invalidate the cached git root so it will be re-resolved on next access.
    #[allow(dead_code)]
    pub fn invalidate_git_root(&mut self) {
        self.git_root = None;
    }

    /// Switch to a different session id and reset all per-session state
    /// (tasks, completion-fade timers, task panel selection). Mirrors v126's
    /// new-session reset: each session has its own task bucket so tasks
    /// don't bleed across `/clear` or `/continue`.
    ///
    /// Pass `None` to mint a fresh session id; pass `Some(id)` to adopt an
    /// existing one (the session-load path through the sidebar / `/continue`).
    pub fn switch_session(&mut self, id: Option<crate::ids::SessionId>) {
        let old_id = self.current_session_id.clone();
        let new_id = id.unwrap_or_else(jfc_session::generate_session_id);
        tracing::info!(
            target: "jfc::app",
            old_session_id = ?old_id,
            new_session_id = %new_id,
            "switch_session"
        );
        self.current_session_id = Some(new_id.clone());
        self.task_store = jfc_session::TaskStore::open(new_id.as_str());
        self.task_completion_times.clear();
        self.task_activities.clear();
        self.task_panel_selected = 0;
        self.task_panel_state = ratatui::widgets::TableState::default().with_selected(Some(0));
        self.task_panel_detail = false;
        self.viewing_task_id = None;
        self.viewing_task_expanded.clear();
        self.deferred_tool_uses.clear();
        self.in_progress_tool_use_ids.clear();
        self.tool_use_summaries.clear();
        self.compact_suppressed = false;
        self.recompute_token_estimate();
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
        let old_estimate = self.tool_ctx.approx_tokens;
        // v126's `tokenCountWithEstimation` (tokens.ts:226-261): find the last
        // assistant message with API usage, use that as the authoritative base,
        // then rough-estimate any messages added AFTER it (user prompts, tool
        // results). This prevents the gap between API calls where the gauge
        // reads 0 or stale for newly-added messages.
        let last_usage_idx = self
            .messages
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, m)| m.usage.as_ref().map(|u| (i, u.clone())));
        if let Some((idx, u)) = last_usage_idx {
            self.last_usage_input = u.input_tokens as u32;
            self.last_usage_output = u.output_tokens as u32;
            let base = u.total_context_tokens() as usize;
            // Estimate tokens for messages added after the usage-bearing
            // message — but exclude queued placeholders, since they
            // aren't actually in the prompt the model sees (see
            // `build_provider_messages`).
            let tail: Vec<crate::types::ChatMessage> = self.messages[idx + 1..]
                .iter()
                .filter(|m| !m.queued)
                .cloned()
                .collect();
            let tail_estimate = crate::compact::estimate_tokens(&tail);
            self.tool_ctx.approx_tokens = base + tail_estimate;
        } else {
            self.last_usage_input = 0;
            self.last_usage_output = 0;
            // Same queued filter as above. Without this, queueing a
            // long prompt during a streaming turn would visibly bump
            // the context gauge even though that text isn't part of
            // the current prompt.
            let unqueued: Vec<crate::types::ChatMessage> = self
                .messages
                .iter()
                .filter(|m| !m.queued)
                .cloned()
                .collect();
            self.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&unqueued);
        }
        tracing::debug!(
            target: "jfc::app",
            old_estimate,
            new_estimate = self.tool_ctx.approx_tokens,
            "recompute_token_estimate"
        );
    }

    /// Drop any persisted post-copy text-selection highlight. The selection is
    /// anchored to absolute screen cells, so once the transcript scrolls those
    /// cells map to different content — invalidate it the instant that happens.
    fn clear_selection_on_scroll(&mut self) {
        if self.text_selection.is_some() {
            self.text_selection = None;
        }
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset))]
    pub fn scroll_to_bottom(&mut self) {
        self.clear_selection_on_scroll();
        self.scroll_offset = self.max_scroll();
        self.follow_bottom = true;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset))]
    pub fn scroll_to_top(&mut self) {
        self.clear_selection_on_scroll();
        self.scroll_offset = 0;
        self.follow_bottom = false;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset, lines))]
    pub fn scroll_up(&mut self, lines: usize) {
        self.clear_selection_on_scroll();
        self.scroll_offset = self.scroll_offset.saturating_sub(lines);
        self.follow_bottom = false;
    }

    #[tracing::instrument(target = "jfc::app", skip(self), fields(scroll_offset = self.scroll_offset, lines))]
    pub fn scroll_down(&mut self, lines: usize) {
        self.clear_selection_on_scroll();
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

    #[allow(dead_code)]
    pub fn is_at_bottom(&self) -> bool {
        self.scroll_offset >= self.max_scroll()
    }

    pub fn selected_model_info(&self) -> Option<ModelInfo> {
        let provider_name = self.provider.name();
        self.provider_models
            .get(provider_name)
            .and_then(|models| models.iter().find(|model| model.id == self.model).cloned())
            .or_else(|| {
                self.providers
                    .iter()
                    .find(|provider| provider.name() == provider_name)
                    .and_then(|provider| {
                        provider
                            .available_models()
                            .into_iter()
                            .find(|model| model.id == self.model)
                    })
            })
    }

    pub fn selected_context_window_tokens(&self) -> usize {
        let result = self
            .selected_model_info()
            .and_then(|model| model.context_window_tokens)
            .unwrap_or_else(|| {
                // Model info not yet loaded (async fetch_models hasn't completed).
                // Use model-name heuristic to avoid the gauge showing 100% for
                // large sessions on models with >200k windows (e.g. opus 4.6 = 1M).
                crate::providers::openwebui::infer_context_window_from_model_name(
                    self.model.as_str(),
                    None,
                )
            });
        tracing::trace!(
            target: "jfc::app",
            model = %self.model,
            result,
            "selected_context_window_tokens"
        );
        result
    }

    pub fn sync_selected_context_window(&mut self) {
        let old = self.max_context_tokens;
        self.max_context_tokens = self.selected_context_window_tokens();
        // When the model/provider changes, re-estimate token count. But if
        // we already have a usage-based estimate from a loaded session
        // (recompute_token_estimate found a message with `usage`), prefer
        // that over the rough heuristic — it's accurate to the token.
        // Without this guard, an async `ModelsLoaded` event firing after
        // session resume clobbers the 298k accurate value with a ~75k
        // chars/4 heuristic, making the gauge jump down to near-zero.
        let has_usage_based_estimate = self.messages.iter().rev().any(|m| m.usage.is_some());
        if !has_usage_based_estimate {
            // Exclude queued placeholders — same rationale as
            // `recompute_token_estimate`.
            let unqueued: Vec<crate::types::ChatMessage> = self
                .messages
                .iter()
                .filter(|m| !m.queued)
                .cloned()
                .collect();
            self.tool_ctx.approx_tokens = crate::compact::estimate_tokens(&unqueued);
        }
        tracing::info!(
            target: "jfc::app",
            old_max_context_tokens = old,
            new_max_context_tokens = self.max_context_tokens,
            approx_tokens = self.tool_ctx.approx_tokens,
            has_usage_based_estimate,
            model = %self.model,
            "sync_selected_context_window"
        );
    }

    fn max_scroll(&self) -> usize {
        self.total_lines.saturating_sub(self.viewport_height.max(1))
    }

    fn half_page(&self) -> usize {
        (self.viewport_height / 2).max(1)
    }

    pub fn tool_needs_approval(&self, tool: &ToolCall) -> bool {
        // Fast path: when running inside a landlock sandbox, permission
        // prompts add friction without security value — auto-approve
        // unless the user has explicitly opted out via config.
        if crate::is_sandbox_active() {
            let auto_allow = crate::config::load_arc()
                .permission_automation
                .as_ref()
                .map(|pa| pa.auto_allow_if_sandboxed)
                .unwrap_or(true);
            if auto_allow {
                tracing::debug!(
                    target: "jfc::app",
                    tool_kind = tool.kind.label(),
                    result = false,
                    reason = "sandbox_active",
                    "tool_needs_approval"
                );
                return false;
            }
        }

        // Permission mode takes priority
        match self.permission_mode.auto_approves(tool) {
            PermissionDecision::Approved => return false,
            // Denied tools don't need a *prompt* — but they must not be
            // dispatched either. The StreamTool handler checks
            // `tool_denied_by_mode` before routing and short-circuits
            // denied tools into a Failed transcript entry.
            PermissionDecision::Denied(_) => return false,
            PermissionDecision::NeedsClassifier => return false, // auto-mode classifier handles
            PermissionDecision::NeedsPrompt => {}
        }

        let name = tool.kind.label();
        if self.always_approved.iter().any(|n| n == name) {
            tracing::debug!(
                target: "jfc::app",
                tool_kind = name,
                result = false,
                reason = "always_approved",
                "tool_needs_approval"
            );
            return false;
        }
        if self.session_approved.iter().any(|n| n == name) {
            tracing::debug!(
                target: "jfc::app",
                tool_kind = name,
                result = false,
                reason = "session_approved",
                "tool_needs_approval"
            );
            return false;
        }
        let result = matches!(
            tool.kind,
            ToolKind::Bash | ToolKind::Write | ToolKind::Edit | ToolKind::ApplyPatch
        );
        tracing::debug!(
            target: "jfc::app",
            tool_kind = name,
            result,
            "tool_needs_approval"
        );
        result
    }

    /// Check if a tool should be auto-denied by the current permission mode.
    pub fn tool_denied_by_mode(&self, tool: &ToolCall) -> Option<&'static str> {
        let result = match self.permission_mode.auto_approves(tool) {
            PermissionDecision::Denied(reason) => Some(reason),
            _ => None,
        };
        tracing::debug!(
            target: "jfc::app",
            tool_kind = tool.kind.label(),
            mode = ?self.permission_mode,
            denied = result.is_some(),
            "tool_denied_by_mode"
        );
        result
    }

    /// Cancel a running background task by ID. Marks it as cancelled
    /// and signals the underlying cancellation token if available.
    #[allow(dead_code)]
    pub fn cancel_background_task(&mut self, task_id: &str) {
        use crate::types::TaskLifecycle;
        if let Some(bt) = self.background_tasks.get_mut(task_id) {
            bt.status = TaskLifecycle::Cancelled;
        }
    }

    /// Scan the task store for newly-completed tasks and record their
    /// completion instant so the footer can fade them out after 30 s.
    pub fn sync_task_completions(&mut self) {
        use jfc_session::TaskStatus;
        for task in self.task_store.list(jfc_session::DeletedFilter::Exclude) {
            if task.status == TaskStatus::Completed
                && !self.task_completion_times.contains_key(&task.id)
            {
                self.task_completion_times
                    .insert(task.id.clone(), Instant::now());
            }
        }
        // Prune entries for tasks that are no longer completed (e.g. re-opened).
        let store = &self.task_store;
        self.task_completion_times.retain(|id, _| {
            store
                .get(id)
                .is_some_and(|t| t.status == TaskStatus::Completed)
        });
    }
}

fn stream_watchdog_timeout_secs() -> Option<u64> {
    if std::env::var("JFC_DISABLE_STREAM_WATCHDOG")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
    {
        return None;
    }
    match std::env::var("JFC_STREAM_WATCHDOG_TIMEOUT_SECS") {
        Ok(raw) => raw.trim().parse::<u64>().ok().filter(|&secs| secs != 0),
        Err(_) => Some(STREAM_WATCHDOG_TIMEOUT_SECS),
    }
}
