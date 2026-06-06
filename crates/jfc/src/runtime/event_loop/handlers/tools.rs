//! `ToolEvent::OutputChunk`, `ToolEvent::Result`, and `ToolEvent::AllComplete`
//! handlers — streaming tool output, tool completion, and continuation logic.

use std::sync::Arc;

use crate::app::App;
use crate::runtime::{
    AppEvent, CompactionEvent, EventSender, dispatch_goal_evaluator_if_active,
    drain_queued_prompts, maybe_continue_task_factory,
};
use crate::types::*;
use crate::{session, stream, types};

/// Handle `ToolEvent::OutputChunk { tool_id, chunk }`.
pub(crate) fn handle_output_chunk(app: &mut App, tool_id: crate::ids::ToolId, chunk: String) {
    // Append streaming output to the tool's live preview.
    // This fires line-by-line for bash commands, giving
    // real-time visibility into long-running processes.
    for msg in &mut app.messages {
        for part in &mut msg.parts {
            if let MessagePart::Tool(tc) = part
                && tc.id == tool_id
            {
                // Append to existing output or create new
                match &mut tc.output {
                    ToolOutput::Text(s) => {
                        s.push_str(&chunk);
                        s.push('\n');
                    }
                    _ => {
                        tc.output = ToolOutput::Text(format!("{chunk}\n"));
                    }
                }
                break;
            }
        }
    }

    // v132 Marsh (mid-stream bash output to model):
    // accumulate the chunk into a pending buffer that
    // stream.rs prepends as a `<system-reminder>` on
    // the *next* outbound request. Not strictly mid-
    // stream (the API call is already in flight) but
    // ensures the model sees what bash printed by the
    // time it next gets the wheel — close enough for
    // the "I see the error, stop" feedback loop in
    // agentic loops where each tool reply re-enters
    // the model.
    if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::Marsh) {
        let _ = tool_id;
        crate::feature_gates::marsh_push(chunk);
    }
}

/// Handle `ToolEvent::Result { tool_id, result }`.
pub(crate) fn handle_tool_result(
    app: &mut App,
    tx: &EventSender,
    tool_id: crate::ids::ToolId,
    result: crate::runtime::ExecutionResult,
) {
    tracing::info!(
        target: "jfc::stream",
        tool_id = %tool_id,
        is_error = result.is_error(),
        output_len = result.output.len(),
        "tool_result received"
    );
    app.exploration_state.record_tool_result(result.is_error());
    let _ = tx.try_send(AppEvent::Tool(
        crate::runtime::ToolEvent::SetInProgressToolUseIds {
            action: "remove".to_owned(),
            ids: vec![tool_id.as_str().to_owned()],
        },
    ));
    let mut found = false;
    for msg in &mut app.messages {
        for part in &mut msg.parts {
            if let MessagePart::Tool(tc) = part
                && tc.id == tool_id
            {
                // Use the typestate-style transition
                // helpers — they refuse to revive a
                // terminal tool (Failed → Completed
                // would be a logic bug, e.g. a stale
                // ToolResult arriving after a denial).
                // On invalid transition we log + leave
                // the existing terminal status alone,
                // since the second result is the
                // duplicate, not the first.
                let transition = if result.is_error() {
                    tc.mark_failed()
                } else {
                    tc.mark_completed()
                };
                if let Err(err) = transition {
                    tracing::warn!(
                        target: "jfc::event_loop",
                        tool_id = %tc.id.as_str(),
                        from = ?err.from,
                        to = ?err.to,
                        "ToolResult: refusing to revive terminal tool — \
                         keeping prior status",
                    );
                    return;
                }
                // Stamp wall-clock duration as soon as
                // the result lands. The renderer reads
                // `tc.elapsed_ms` to draw a muted
                // "[2.3s]" badge after the title. Falls
                // back to None if `started_at` was lost
                // (e.g., resumed session) — the badge
                // just doesn't appear in that case.
                if let Some(start) = tc.started_at {
                    tc.elapsed_ms = Some(start.elapsed().as_millis() as u64);
                }
                // Tool authors can attach a structured
                // DiffView (Edit, Write-overwrite) so
                // the renderer shows colorized hunks
                // instead of a flat success string.
                tc.output = if let Some(diff) = result.diff.clone() {
                    ToolOutput::Diff(diff)
                } else if LargeText::should_collapse(&result.output) {
                    ToolOutput::LargeText(LargeText::new(result.output.clone()))
                } else {
                    ToolOutput::Text(result.output.clone())
                };
                if matches!(tc.output, ToolOutput::LargeText(_)) {
                    tc.display.collapse();
                }
                // Fresh tool output → reset the
                // path-yank cursor so the next
                // `Ctrl+L` starts from the newest
                // file:line ref.
                app.path_yank_cursor = 0;
                if result.is_error() {
                    crate::notifications::notify_tool_failed(tc.kind.label(), &result.output);
                }
                let new_status = tc.status;
                // Record files this turn touched (Edit/Write) so `/turn-diff`
                // can scope a diff to just this agentic step. Only on success.
                if matches!(new_status, ToolStatus::Completed) {
                    match &tc.input {
                        crate::types::ToolInput::Edit { file_path, .. }
                        | crate::types::ToolInput::Write { file_path, .. } => {
                            app.turn_edited_files.insert(file_path.clone());
                        }
                        _ => {}
                    }
                }
                // Reset plan verification when new tasks are
                // created so the next factory cycle re-verifies.
                if matches!(tc.kind, ToolKind::TaskCreate)
                    && matches!(new_status, ToolStatus::Completed)
                {
                    app.plan_verified_this_batch = false;
                }
                found = true;
                break;
            }
        }
        if found {
            // If the tool result carries attachments (e.g. a
            // PDF loaded by the Read tool), push them onto the
            // assistant message that owns the tool call. They'll
            // be serialized as ProviderContent::Attachment blocks
            // in the next provider request via per-message
            // ownership — no global queue needed.
            if !result.attachments.is_empty() {
                for msg in &mut app.messages {
                    if matches!(msg.role, types::Role::Assistant)
                        && msg
                            .parts
                            .iter()
                            .any(|p| matches!(p, MessagePart::Tool(tc) if tc.id == tool_id))
                    {
                        tracing::debug!(
                            target: "jfc::stream",
                            tool_id = %tool_id,
                            count = result.attachments.len(),
                            "promoting tool result attachments to owning message"
                        );
                        msg.attachments.extend(result.attachments.clone());
                        break;
                    }
                }
            }
            break;
        }
    }
    if !found {
        tracing::warn!(
            target: "jfc::event_loop",
            tool_id = %tool_id,
            is_error = result.is_error(),
            output_len = result.output.len(),
            "ToolResult did not match any assistant tool block",
        );
    }
    // Session save is deferred to AllToolsComplete so we write
    // once per batch, not once per tool result. This eliminates
    // the 650+ disk writes per agentic run observed in profiling.
}

pub(crate) fn handle_set_in_progress_tool_use_ids(app: &mut App, action: String, ids: Vec<String>) {
    tracing::debug!(
        target: "jfc::tool_state",
        action = %action,
        ids = ?ids,
        "set_in_progress_tool_use_ids"
    );
    app.set_in_progress_tool_use_ids(&action, &ids);
}

pub(crate) fn handle_deferred_tool_use(
    app: &mut App,
    id: String,
    name: String,
    input_preview: String,
    reason: String,
) {
    tracing::debug!(
        target: "jfc::tool_state",
        id = %id,
        name = %name,
        reason = %reason,
        "deferred_tool_use"
    );
    app.record_deferred_tool_use(id, name, input_preview, reason);
}

pub(crate) fn handle_tool_use_summary(
    app: &mut App,
    summary: String,
    preceding_tool_use_ids: Vec<String>,
) {
    tracing::debug!(
        target: "jfc::tool_state",
        summary = %summary,
        ids = ?preceding_tool_use_ids,
        "tool_use_summary"
    );
    app.record_tool_use_summary(summary, preceding_tool_use_ids);
}

fn last_assistant_has_unresolved_tool(app: &App) -> bool {
    app.messages
        .iter()
        .rev()
        .find(|msg| msg.role == Role::Assistant)
        .is_some_and(|msg| {
            msg.parts
                .iter()
                .any(|part| matches!(part, MessagePart::Tool(tool) if !tool.status.is_terminal()))
        })
}

fn completed_tool_batch_summary(app: &App) -> Option<(String, Vec<String>)> {
    let last_assistant = app
        .messages
        .iter()
        .rev()
        .find(|msg| msg.role == Role::Assistant)?;
    let tools: Vec<&ToolCall> = last_assistant
        .parts
        .iter()
        .filter_map(|part| match part {
            MessagePart::Tool(tool) if tool.status.is_terminal() => Some(tool.as_ref()),
            _ => None,
        })
        .collect();
    if tools.is_empty() {
        return None;
    }
    let ids = tools
        .iter()
        .map(|tool| tool.id.as_str().to_owned())
        .collect::<Vec<_>>();
    let summary = if tools.len() == 1 {
        let tool = tools[0];
        let verb = match &tool.kind {
            ToolKind::Read => "Read",
            ToolKind::Edit | ToolKind::MultiEdit | ToolKind::Write | ToolKind::ApplyPatch => {
                "Edited"
            }
            ToolKind::Bash => "Ran",
            ToolKind::Grep | ToolKind::Glob | ToolKind::Search => "Searched",
            ToolKind::Task | ToolKind::TaskCreate | ToolKind::TaskStop => "Managed task",
            ToolKind::Advisor | ToolKind::ServerAdvisor => "Reviewed",
            _ => tool.kind.label(),
        };
        let detail = truncate_summary(&tool.input.summary(), 42);
        if detail.is_empty() {
            verb.to_owned()
        } else {
            truncate_summary(&format!("{verb} {detail}"), 80)
        }
    } else {
        let mut labels = Vec::<&str>::new();
        for tool in &tools {
            let label = tool.kind.label();
            if !labels.contains(&label) {
                labels.push(label);
            }
        }
        truncate_summary(
            &format!("Ran {} tools: {}", tools.len(), labels.join(", ")),
            80,
        )
    };
    Some((summary, ids))
}

fn truncate_summary(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_owned();
    }
    let mut out: String = trimmed.chars().take(max_chars.saturating_sub(3)).collect();
    out.push_str("...");
    out
}

pub(crate) fn should_recheck_completion_after_tool_result(app: &App) -> bool {
    !app.is_streaming
        && app.pending_classifications == 0
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty()
        && app.in_flight_eager_dispatches == 0
        && app.in_flight_tool_batches == 0
        && app.compacting_started_at.is_none()
        && stream::should_continue_loop(&app.messages)
}

/// Handle `ToolEvent::AllComplete` — all tools in the current batch finished.
pub(crate) async fn handle_all_complete(app: &mut App, tx: &EventSender) {
    // Decrement the dispatch counters if there are outstanding ones.
    app.in_flight_eager_dispatches = app.in_flight_eager_dispatches.saturating_sub(1);
    app.in_flight_tool_batches = app.in_flight_tool_batches.saturating_sub(1);
    tracing::info!(
        target: "jfc::stream",
        message_count = app.messages.len(),
        model = %app.model,
        pending_approvals = app.approval_queue.len() + usize::from(app.pending_approval.is_some()),
        pending_tool_calls = app.pending_tool_calls.len(),
        pending_classifications = app.pending_classifications,
        in_flight_eager = app.in_flight_eager_dispatches,
        in_flight_batches = app.in_flight_tool_batches,
        "ToolEvent::AllComplete"
    );
    if app.is_streaming {
        // A late AllComplete after cancellation may still find queued safe
        // tools. Dispatching is OK: the shared cancellation token makes the
        // scheduler report them as cancelled instead of running side effects.
        let dispatched = super::stream_tool::dispatch_eager_safe_prefix(app, tx);
        if !dispatched.is_empty() {
            tracing::debug!(
                target: "jfc::stream",
                ids = ?dispatched,
                "AllToolsComplete: dispatched next eager-safe prefix"
            );
            return;
        }
    } else if super::stream_tool::dispatch_pending_after_stream(app, tx) {
        tracing::debug!(
            target: "jfc::stream",
            "AllToolsComplete: dispatched remaining ordered tool batch after eager prefix"
        );
        return;
    }
    // AllToolsComplete is *batch-local*: it fires when
    // the current `dispatch_tools_batched` call finishes
    // its tools. The approval path dispatches one tool at
    // a time, so this event arrives once per approval —
    // not once per turn. Treat the event as authoritative
    // for "the local batch ended" only; defer turn-level
    // side effects (compaction, queued-prompt drain,
    // agentic continuation) until ALL of the following
    // are true:
    //   - no tool waiting on user approval
    //   - no other tools queued for approval
    //   - no pending in-flight tool_calls
    // Otherwise we'd kick off compaction mid-turn (while
    // half the model's tool batch is still queued) and
    // re-stream provider requests against an incomplete
    // transcript.
    let turn_truly_complete = app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty()
        && app.pending_classifications == 0
        && app.in_flight_eager_dispatches == 0
        && app.in_flight_tool_batches == 0;
    if !turn_truly_complete {
        tracing::debug!(
            target: "jfc::stream",
            "AllToolsComplete: batch finished but turn still has pending tools — deferring side effects"
        );
        return;
    }
    if last_assistant_has_unresolved_tool(app) {
        tracing::warn!(
            target: "jfc::stream",
            "AllToolsComplete arrived before all tool results were recorded — waiting for ToolResult recheck"
        );
        return;
    }
    if let Some((summary, preceding_tool_use_ids)) = completed_tool_batch_summary(app) {
        let _ = tx.try_send(AppEvent::Tool(crate::runtime::ToolEvent::UseSummary {
            summary,
            preceding_tool_use_ids,
        }));
    }
    // Save session once per completed tool batch (not per tool).
    if let Some(ref session_id) = app.current_session_id {
        let sid = session_id.clone();
        let msgs = app.messages.clone();
        let cwd = app.cwd.clone();
        let model = app.model.clone();
        tokio::spawn(async move {
            session::save_session(&sid, &msgs, Some(cwd.as_str()), Some(model.as_str())).await;
        });
        app.last_session_save_at = Some(std::time::Instant::now());
    }

    // Slop Guard aggregation: scan the last assistant
    // message's tool results for slop_guard findings.
    // If any are present, inject a system-reminder so
    // the model sees the aggregate findings on its next turn.
    {
        let marker = crate::tools::SLOP_GUARD_MARKER;
        let mut aggregate_findings: Vec<String> = Vec::new();
        if let Some(last_assistant) = app
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
        {
            for part in &last_assistant.parts {
                if let MessagePart::Tool(tc) = part {
                    let output_text = tc.output.to_api_text();
                    if let Some(idx) = output_text.find(marker) {
                        let findings = &output_text[idx + marker.len()..];
                        if !findings.trim().is_empty() {
                            aggregate_findings.push(findings.trim().to_string());
                        }
                    }
                }
            }
        }
        if !aggregate_findings.is_empty() {
            let reminder_body = format!(
                "Slop Guard detected quality issues in your recent edits. \
                 Review and fix these before proceeding:\n\n{}",
                aggregate_findings.join("\n\n---\n\n")
            );
            tracing::debug!(
                target: "jfc::slop_guard",
                finding_count = aggregate_findings.len(),
                "injecting slop_guard system-reminder"
            );
            crate::system_reminder::append_to_last_user(&mut app.messages, &reminder_body);
        }
    }

    // Terminal bell when a tool batch completes — matches
    // v126's `iterm2_with_bell` / `terminal_bell` behavior
    // (cli.js:46704). Many users have iTerm2 / WezTerm /
    // Ghostty configured to badge or notify on bell, so this
    // gives a "your input is needed / a long task finished"
    // hint without us having to hand-roll desktop notifications.
    // Suppress when the user opted out via env (matches
    // v126's `notifications_disabled` setting).
    if !matches!(
        std::env::var("JFC_DISABLE_BELL").as_deref(),
        Ok("1") | Ok("true")
    ) {
        use std::io::Write;
        // Best-effort write — ignore failures; bell is cosmetic.
        let _ = std::io::stderr().write_all(b"\x07");
        let _ = std::io::stderr().flush();
    }
    let manual = std::mem::take(&mut app.force_compact_pending);
    // Guard: don't spawn another compact if one is already in flight.
    // Without this, every AllToolsComplete while context > threshold
    // spawns a NEW compact task — if the provider doesn't support
    // compaction (returns Unsupported), the tasks pile up at ~12/sec
    // and spam 79K+ WARN lines per session. Only `manual` (/compact)
    // bypasses the guard to let the user force a retry.
    if app.compacting_started_at.is_some() && !manual {
        tracing::debug!(
            target: "jfc::compact",
            "skipping post-response compact — one already in flight"
        );
    } else if app.compact_suppressed && !manual {
        tracing::debug!(
            target: "jfc::compact",
            "skipping post-response compact — suppressed after permanent failure"
        );
    } else if manual
        || crate::compact::should_compact(app.tool_ctx.approx_tokens, app.max_context_tokens)
    {
        if manual {
            // /compact is the user's explicit override — clear
            // BOTH the suppression flag AND the rapid-refill
            // counter. Otherwise a previously tripped breaker
            // would still fast-fail this manual attempt.
            app.compact_suppressed = false;
            app.tool_ctx.rapid_refill_count = 0;
        }
        tracing::info!(
            target: "jfc::compact",
            manual,
            model = %app.model,
            max_context_tokens = app.max_context_tokens,
            message_count = app.messages.len(),
            rapid_refill_count = app.tool_ctx.rapid_refill_count,
            "post-response compaction triggered"
        );
        // Set the compaction guard synchronously so the agentic
        // loop continuation check (below) sees it immediately.
        // The CompactionStarted event still fires for the UI
        // spinner, but the guard must be synchronous to prevent
        // the race where continue_agentic_loop fires before the
        // async event is processed.
        app.compacting_started_at = Some(std::time::Instant::now());
        app.compacting_output_chars = 0;
        app.compacting_attempt_baseline = 0;
        app.compacting_last_progress = 0;
        let _ = tx
            .send(AppEvent::Compaction(CompactionEvent::Started))
            .await;
        let messages = app.messages.clone();
        let provider = Arc::clone(&app.provider);
        let model = app.model.clone();
        let mut tool_ctx = app.tool_ctx.clone();
        let window = app.max_context_tokens;
        let tx_compact = tx.clone();
        let progress_tx = tx_compact.clone();
        let on_progress: crate::compact::CompactProgressCb = Box::new(move |chars| {
            // CompactionProgress is non-critical; next progress update supersedes.
            let _ = progress_tx.try_send(AppEvent::Compaction(CompactionEvent::Progress {
                output_chars: chars,
            }));
        });
        // wg-async: compact holds critical state (the full
        // message slice + an outbound tx). Race the long
        // provider call against `cancelled()` so ESC×2
        // mid-compact doesn't leave it running for ~30s
        // sending CompactionDone into a stale state.
        let cancel_compact = app.cancel_token.clone();
        tokio::spawn(async move {
            // Use compaction_model from config if set; otherwise
            // fall back to the session's current model.
            let compact_model_id = crate::config::load_arc()
                .default
                .compaction_model
                .clone()
                .map(jfc_provider::ModelId::new)
                .unwrap_or_else(|| model.clone());
            let options = jfc_provider::StreamOptions::new(compact_model_id.clone());
            tracing::debug!(
                target: "jfc::compact",
                model = %compact_model_id,
                window,
                "spawned post-response compaction task"
            );
            let result = tokio::select! {
                biased;
                _ = cancel_compact.cancelled() => {
                    tracing::info!(
                        target: "jfc::compact",
                        "compaction cancelled via token"
                    );
                    let _ = tx_compact
                        .send(AppEvent::Compaction(CompactionEvent::Failed {
                            reason: "Compaction cancelled by user".into(),
                            calibrated_tokens: None,
                            transient: true,
                        }))
                        .await;
                    return;
                }
                r = crate::compact::compact(
                    &messages,
                    provider.as_ref(),
                    &options,
                    &mut tool_ctx,
                    window,
                    Some(on_progress),
                ) => r,
            };
            match result {
                crate::compact::CompactResult::Success {
                    messages,
                    pre_tokens,
                    post_tokens,
                } => {
                    tracing::info!(
                        target: "jfc::compact",
                        pre_tokens, post_tokens,
                        saved = pre_tokens.saturating_sub(post_tokens),
                        "post-response compaction succeeded — sending CompactionDone"
                    );
                    let _ = tx_compact
                        .send(AppEvent::Compaction(CompactionEvent::Done {
                            messages,
                            tool_ctx,
                            pre_tokens,
                            post_tokens,
                        }))
                        .await;
                }
                crate::compact::CompactResult::Unsupported => {
                    tracing::info!(
                        target: "jfc::compact",
                        "post-response compaction skipped (provider unsupported)"
                    );
                    let _ = tx_compact
                        .send(AppEvent::Compaction(CompactionEvent::Failed {
                            reason: "Provider does not support compaction — \
                 try /clear or switch to a provider with non-streaming support."
                                .into(),
                            calibrated_tokens: None,
                            transient: false, // permanent: provider mismatch won't fix itself
                        }))
                        .await;
                }
                crate::compact::CompactResult::TooFewGroups => {
                    tracing::info!(
                        target: "jfc::compact",
                        "post-response compaction skipped (single user turn)"
                    );
                    // Transient: the next user message creates a
                    // second group, so auto-compaction can fire
                    // again. Don't latch `compact_suppressed` —
                    // otherwise a single huge agentic batch leaves
                    // auto-compact dormant for the rest of the
                    // session until the user remembers /compact.
                    let _ = tx_compact
                        .send(AppEvent::Compaction(CompactionEvent::Failed {
                            reason: "Nothing to compact yet — only one conversation turn so far. \
                     Auto-compact will retry after your next message."
                                .into(),
                            calibrated_tokens: None,
                            transient: true, // transient: more user turns will unblock it
                        }))
                        .await;
                }
                crate::compact::CompactResult::CircuitBreakerTripped => {
                    tracing::warn!(
                        target: "jfc::compact",
                        "post-response compaction: circuit breaker tripped"
                    );
                    let _ = tx_compact
                        .send(AppEvent::Compaction(CompactionEvent::Failed {
                            reason: "Circuit breaker tripped — compaction keeps refilling".into(),
                            calibrated_tokens: None,
                            transient: false,
                        }))
                        .await;
                }
                crate::compact::CompactResult::Exhausted { attempts } => {
                    tracing::warn!(
                        target: "jfc::compact",
                        attempts,
                        "post-response compaction exhausted all attempts"
                    );
                    let _ = tx_compact
                        .send(AppEvent::Compaction(CompactionEvent::Failed {
                            reason: format!("Exhausted {attempts} compaction attempts"),
                            calibrated_tokens: None,
                            transient: false,
                        }))
                        .await;
                }
            }
        });
    }
    // Gate the agentic continuation on the approval pipeline being
    // empty. Without this, dispatching tool 0 fires
    // AllToolsComplete (1 tool finished, last message has 1
    // Complete part → should_continue_loop=true), the loop sends a
    // *new* request, and tools 1..N still queued for approval get
    // inserted into the wrong assistant turn — the conversation
    // visibly stalls. From the v126 log: 5 bash tools synthesized
    // then conversation died after first approval. Holding the
    // continuation here lets the user finish all approvals first.
    if app.interrupt_flag.load(std::sync::atomic::Ordering::SeqCst)
        || app.cancel_token.is_cancelled()
    {
        tracing::info!(
            target: "jfc::stream",
            "agentic loop NOT continuing — user requested interrupt"
        );
        // Clear so the next user submission starts fresh —
        // both the legacy flag and the (possibly cancelled)
        // token need refreshing for the next spawn cycle.
        app.interrupt_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
        app.cancel_token = tokio_util::sync::CancellationToken::new();
        app.is_streaming = false;
        app.streaming_started_at = None;
        app.last_stream_event_at = None;
        app.streaming_last_token_at = None;
        app.token_rate_samples.clear();
        app.thinking_started_at = None;
        app.thinking_ended_at = None;
        app.streaming_text.clear();
        app.streaming_reasoning.clear();
        app.streaming_response_bytes = 0;
        app.streaming_assistant_idx = None;
        app.current_stream_request = None;
        app.stream_lifecycle = None;
        app.turn_started_at = None;
    } else if app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.compacting_started_at.is_none()
        && stream::should_continue_loop(&app.messages)
    {
        // Fan-out consolidation: if multiple parallel agent
        // tasks completed in this batch, inject a summary
        // so the model sees a coherent digest before responding.
        if let Some(last_assistant) = app
            .messages
            .iter()
            .rev()
            .find(|m| m.role == Role::Assistant)
        {
            let task_summaries: Vec<String> = last_assistant
                .parts
                .iter()
                .filter_map(|p| {
                    if let MessagePart::TaskStatus(ts) = p
                        && ts.status.is_terminal()
                    {
                        return ts.summary.clone().or_else(|| ts.error.clone());
                    }
                    None
                })
                .collect();
            if task_summaries.len() >= 2 {
                let task_count = task_summaries.len();
                let consolidated = format!(
                    "{task_count} parallel agents completed this batch. Their results:\n\n{}",
                    task_summaries
                        .iter()
                        .enumerate()
                        .map(|(i, s)| format!(
                            "{}. {}",
                            i + 1,
                            s.chars().take(200).collect::<String>()
                        ))
                        .collect::<Vec<_>>()
                        .join("\n")
                );
                crate::system_reminder::append_to_last_user(
                    &mut app.messages,
                    &format!(
                        "Consolidation of {task_count} parallel agent results:\n\n{consolidated}\n\nSynthesize these results into a coherent response. Deduplicate overlapping findings. Note any contradictions between agents."
                    ),
                );
            }
        }

        // Mixed-mode pause_turn: the original turn's Done
        // event carried StopReason::PauseTurn AND emitted
        // local tools. The dispatch ladder ran the local
        // tools (shadowing the PauseTurn arm); now that
        // they're complete, route through the pause-turn-
        // resume builder so we don't inject the forbidden
        // "Continue from where you left off." filler. See
        // app.pending_pause_turn_resume docs and cli.js
        // v142:622686 for the protocol.
        //
        // Single-shot: clear the flag before dispatching
        // so a follow-up turn that doesn't pause_turn
        // returns to the normal continue_agentic_loop
        // path. If the resumed turn ALSO pause_turns,
        // its own Done handler re-latches the flag.
        if app.pending_pause_turn_resume {
            app.pending_pause_turn_resume = false;
            tracing::info!(
                target: "jfc::stream",
                "mixed-mode pause_turn: local tools complete, resuming server-side sampling loop"
            );
            stream::continue_after_pause_turn(app, tx).await;
        } else if !app.queued_prompts.is_empty() {
            // Drain whenever ANY prompt is queued (meta OR non-meta).
            // Previously this only fired for non-meta prompts, which
            // meant slash commands (`/tasks`, `/market`, local actions)
            // submitted mid-stream would sit in the queue until the
            // entire agentic loop concluded — making the TUI appear
            // to silently ignore them.
            tracing::info!(
                target: "jfc::stream",
                queued = app.queued_prompts.len(),
                "agentic loop yielding to queued prompt before continuation"
            );
            drain_queued_prompts(app, tx).await;
        } else {
            tracing::info!(
                target: "jfc::stream",
                "agentic loop continuing — tools complete, no pending approvals"
            );
            stream::continue_agentic_loop(app, tx).await;
        }
    } else if !app.is_streaming
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && app.pending_tool_calls.is_empty()
    {
        tracing::debug!(
            target: "jfc::stream",
            "turn fully ended — draining queued prompts"
        );
        // Turn fully ended (model stopped, no more agentic loop
        // iterations, no pending tools). Clear turn_started_at
        // so the spinner stops, then drain any prompts the user
        // typed during streaming.
        app.turn_started_at = None;
        // /goal stop-hook: if a goal is active, the agent
        // doesn't truly get to stop here. Fire the
        // evaluator in the background; the agentic loop
        // re-enters when the verdict lands (see
        // GoalEvent::Verdict). Bail before draining
        // queued prompts so a queued prompt can't race
        // ahead of the verdict and unset the goal mid-eval.
        if dispatch_goal_evaluator_if_active(app, tx) {
            tracing::info!(
                target: "jfc::goal",
                "goal evaluator dispatched on EndTurn — deferring drain"
            );
        } else {
            drain_queued_prompts(app, tx).await;
            maybe_continue_task_factory(app, tx).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::{sync::Arc, time::Instant};

    use super::*;
    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

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

    fn test_app_with_tool(status: ToolStatus) -> (App, crate::ids::ToolId) {
        let tool_id = crate::ids::ToolId::from("tool-1");
        let mut tool = ToolCall::new_pending(
            tool_id.clone(),
            ToolKind::Bash,
            ToolInput::Generic {
                summary: String::new(),
            },
        );
        tool.status = status;

        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = jfc_session::TaskStore::in_memory();
        app.messages.push(ChatMessage::user("run tool".into()));
        app.messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::tool(tool)]));
        app.turn_started_at = Some(Instant::now());
        app.is_streaming = false;
        (app, tool_id)
    }

    #[tokio::test]
    async fn all_complete_waits_for_out_of_order_tool_result() {
        let (mut app, _tool_id) = test_app_with_tool(ToolStatus::Pending);
        app.in_flight_tool_batches = 1;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_all_complete(&mut app, &tx).await;

        assert_eq!(app.in_flight_tool_batches, 0);
        assert!(app.turn_started_at.is_some());
        assert!(!should_recheck_completion_after_tool_result(&app));
    }

    #[test]
    fn late_tool_result_rechecks_completion_after_batch_signal_race() {
        let (mut app, tool_id) = test_app_with_tool(ToolStatus::Pending);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_tool_result(
            &mut app,
            &tx,
            tool_id,
            crate::runtime::ExecutionResult::success("ok"),
        );

        assert!(should_recheck_completion_after_tool_result(&app));
    }

    #[test]
    fn stale_success_result_does_not_overwrite_failed_tool_output() {
        let (mut app, tool_id) = test_app_with_tool(ToolStatus::Failed);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        if let Some(MessagePart::Tool(tool)) = app.messages[1].parts.get_mut(0) {
            tool.output = ToolOutput::Text("Denied by permission mode".to_owned());
        }

        handle_tool_result(
            &mut app,
            &tx,
            tool_id,
            crate::runtime::ExecutionResult::success("late stale result"),
        );

        let MessagePart::Tool(tool) = &app.messages[1].parts[0] else {
            panic!("expected tool part");
        };
        assert_eq!(tool.status, ToolStatus::Failed);
        assert!(matches!(
            &tool.output,
            ToolOutput::Text(text) if text == "Denied by permission mode"
        ));
    }

    #[test]
    fn set_in_progress_tool_use_ids_updates_state_normal() {
        let (mut app, _tool_id) = test_app_with_tool(ToolStatus::Pending);
        handle_deferred_tool_use(
            &mut app,
            "tool-1".into(),
            "Bash".into(),
            "ls".into(),
            "awaiting_approval".into(),
        );
        assert_eq!(app.deferred_tool_uses.len(), 1);

        handle_set_in_progress_tool_use_ids(&mut app, "add".into(), vec!["tool-1".into()]);
        assert!(app.in_progress_tool_use_ids.contains("tool-1"));
        assert!(app.deferred_tool_uses.is_empty());

        handle_set_in_progress_tool_use_ids(&mut app, "remove".into(), vec!["tool-1".into()]);
        assert!(!app.in_progress_tool_use_ids.contains("tool-1"));
    }

    #[test]
    fn completed_tool_batch_summary_single_tool_normal() {
        let (app, _tool_id) = test_app_with_tool(ToolStatus::Completed);
        let (summary, ids) =
            completed_tool_batch_summary(&app).expect("completed tool should summarize");
        assert!(summary.starts_with("Ran"), "{summary}");
        assert_eq!(ids, vec!["tool-1"]);
    }
}
