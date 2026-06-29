//! `ToolEvent::OutputChunk`, `ToolEvent::Result`, and `ToolEvent::AllComplete`
//! handlers — streaming tool output, tool completion, and continuation logic.

use std::sync::Arc;

use crate::app::EngineState;
use crate::runtime::{
    CompactionEvent, EngineEvent, EventSender, dispatch_goal_evaluator_if_active,
    drain_queued_prompts, maybe_continue_task_factory,
};
use crate::types::*;
use crate::{stream, types};

/// Hard byte cap on the *live* streaming tool-output preview built up by
/// `handle_output_chunk`. The full output already lands on disk (bash task
/// log file) and the model-facing result is separately capped by the bash
/// tool's `inline_output_bytes`; this preview exists only so the renderer
/// can show a tail while a command runs. Without a cap, a chatty
/// long-running command (e.g. a sync job printing TSV rows for hours) grows
/// the in-memory transcript without bound — observed live as hundreds of MB
/// of retained tool output in a long session's heap.
pub(crate) const LIVE_OUTPUT_PREVIEW_CAP_BYTES: usize = 64 * 1024;

/// Drop oldest content (front) so `s` stays under `cap` bytes, cutting on a
/// char boundary at a line break where possible so the preview tail starts
/// on a whole line.
fn trim_front_to_cap(s: &mut String, cap: usize) {
    if s.len() <= cap {
        return;
    }
    let excess = s.len() - cap;
    let mut split = s.ceil_char_boundary(excess);
    // Prefer cutting just past the next newline so the kept tail starts
    // at a line boundary (cheap scan over at most one line).
    if let Some(nl) = s[split..].find('\n') {
        split += nl + 1;
    }
    s.drain(..split);
}

fn execution_result_output(result: &crate::runtime::ExecutionResult) -> ToolOutput {
    if let Some(diff) = result.diff.clone() {
        ToolOutput::Diff(diff)
    } else if LargeText::should_collapse(&result.output) {
        ToolOutput::LargeText(LargeText::new(result.output.clone()))
    } else {
        ToolOutput::Text(result.output.clone())
    }
}

/// Handle `ToolEvent::OutputChunk { tool_id, chunk }`.
pub fn handle_output_chunk(state: &mut EngineState, tool_id: crate::ids::ToolId, chunk: String) {
    // Append streaming output to the tool's live preview.
    // This fires line-by-line for bash commands, giving
    // real-time visibility into long-running processes.
    for msg in &mut state.messages {
        for part in &mut msg.parts {
            if let MessagePart::Tool(tc) = part
                && tc.id == tool_id
            {
                // Append to existing output or create new
                match &mut tc.output {
                    ToolOutput::Text(s) => {
                        s.push_str(&chunk);
                        s.push('\n');
                        trim_front_to_cap(s, LIVE_OUTPUT_PREVIEW_CAP_BYTES);
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
pub fn handle_tool_result(
    state: &mut EngineState,
    _tx: &EventSender,
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
    let tool_id_str = tool_id.as_str().to_owned();
    // Tool completion is already being handled on the engine thread with
    // mutable state access. Clear in-progress bookkeeping directly instead of
    // enqueueing a best-effort `SetInProgressToolUseIds(remove)` event back
    // onto the same bounded channel; if that secondary event is dropped under
    // load, `has_interruptible_work()` stays true after the turn has ended.
    state.set_in_progress_tool_use_ids("remove", std::slice::from_ref(&tool_id_str));
    state
        .exploration_state
        .record_tool_result(result.is_error());
    let mut found = false;
    let mut tool_output_arrived = false;
    let mut bash_output_to_fold: Option<(crate::ids::ToolId, String, ToolOutput)> = None;
    for msg in &mut state.messages {
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
                tc.output = execution_result_output(&result);
                if matches!(tc.output, ToolOutput::LargeText(_)) {
                    tc.display.collapse();
                }
                if let ToolInput::BashOutput { task_id, .. } = &tc.input {
                    bash_output_to_fold = Some((tc.id.clone(), task_id.clone(), tc.output.clone()));
                }
                // Fresh tool output → the frontend resets its path-yank
                // cursor so the next `Ctrl+L` starts from the newest ref
                // (effect pushed after the loop — borrowck).
                tool_output_arrived = true;
                if result.is_error() && !matches!(tc.kind, ToolKind::BashOutput) {
                    crate::notifications::notify_tool_failed(tc.kind.label(), &result.output);
                }
                let new_status = tc.status;
                // Record files this turn touched (Edit/Write) so `/turn-diff`
                // can scope a diff to just this agentic step. Only on success.
                if matches!(new_status, ToolStatus::Completed) {
                    match &tc.input {
                        crate::types::ToolInput::Edit { file_path, .. }
                        | crate::types::ToolInput::Write { file_path, .. } => {
                            state.turn_edited_files.insert(file_path.clone());
                        }
                        crate::types::ToolInput::MultiEdit { file_path, .. } => {
                            state.turn_edited_files.insert(file_path.clone());
                        }
                        crate::types::ToolInput::ApplyPatch { patch } => {
                            state
                                .turn_edited_files
                                .extend(crate::auto_review::apply_patch_paths(patch));
                        }
                        _ => {}
                    }
                }
                // Reset plan verification when new tasks are
                // created so the next factory cycle re-verifies.
                if matches!(tc.kind, ToolKind::TaskCreate)
                    && matches!(new_status, ToolStatus::Completed)
                {
                    state.plan_verified_this_batch = false;
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
                for msg in &mut state.messages {
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
    if let Some((source_tool_id, task_id, output)) = bash_output_to_fold
        && fold_bash_output_into_originating_bash(
            &mut state.messages,
            &source_tool_id,
            &task_id,
            output,
        )
    {
        tracing::debug!(
            target: "jfc::event_loop",
            task_id = %task_id,
            "folded BashOutput result into originating Bash tool",
        );
    }
    if tool_output_arrived {
        state.push_effect(crate::app::EngineEffect::ToolOutputArrived);
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

/// Handle a late completion from a tool that had already returned a
/// background-started result. This updates the existing visible block in place
/// instead of creating a model-visible BashOutput/TaskOutput exchange.
pub fn handle_background_tool_result(
    state: &mut EngineState,
    tool_id: crate::ids::ToolId,
    result: crate::runtime::ExecutionResult,
) {
    tracing::info!(
        target: "jfc::stream",
        tool_id = %tool_id,
        is_error = result.is_error(),
        output_len = result.output.len(),
        "background tool_result update received"
    );
    let mut found = false;
    for msg in &mut state.messages {
        for part in &mut msg.parts {
            let MessagePart::Tool(tc) = part else {
                continue;
            };
            if tc.id != tool_id {
                continue;
            }
            if !matches!(tc.input, ToolInput::Bash { .. }) {
                tracing::warn!(
                    target: "jfc::event_loop",
                    tool_id = %tool_id,
                    kind = %tc.kind.label(),
                    "background result targeted a non-Bash tool; ignoring"
                );
                return;
            }
            if result.is_error() {
                tc.status = ToolStatus::Failed;
            } else {
                tc.status = ToolStatus::Completed;
            }
            if let Some(start) = tc.started_at {
                tc.elapsed_ms = Some(start.elapsed().as_millis() as u64);
            }
            tc.output = execution_result_output(&result);
            if matches!(tc.output, ToolOutput::LargeText(_)) {
                tc.display.collapse();
            }
            found = true;
            break;
        }
        if found {
            break;
        }
    }
    if found {
        state.push_effect(crate::app::EngineEffect::ToolOutputArrived);
        if result.is_error() {
            crate::notifications::notify_tool_failed(ToolKind::Bash.label(), &result.output);
        }
    } else {
        tracing::warn!(
            target: "jfc::event_loop",
            tool_id = %tool_id,
            is_error = result.is_error(),
            output_len = result.output.len(),
            "BackgroundResult did not match any assistant Bash tool block",
        );
    }
}

fn fold_bash_output_into_originating_bash(
    messages: &mut [ChatMessage],
    source_tool_id: &crate::ids::ToolId,
    task_id: &str,
    output: ToolOutput,
) -> bool {
    for msg in messages.iter_mut().rev() {
        for part in msg.parts.iter_mut().rev() {
            let MessagePart::Tool(tool) = part else {
                continue;
            };
            if &tool.id == source_tool_id || !matches!(tool.input, ToolInput::Bash { .. }) {
                continue;
            }
            if background_task_id_from_output(&tool.output).as_deref() == Some(task_id) {
                tool.output = output;
                return true;
            }
        }
    }
    false
}

fn background_task_id_from_output(output: &ToolOutput) -> Option<String> {
    let text = match output {
        ToolOutput::Text(text) => text.as_str(),
        ToolOutput::LargeText(text) => text.content.as_str(),
        _ => return None,
    };
    text.lines()
        .find_map(|line| line.strip_prefix("task_id: "))
        .map(str::trim)
        .filter(|task_id| task_id.starts_with("bash_"))
        .map(ToOwned::to_owned)
}

pub fn handle_set_in_progress_tool_use_ids(
    state: &mut EngineState,
    action: String,
    ids: Vec<String>,
) {
    tracing::debug!(
        target: "jfc::tool_state",
        action = %action,
        ids = ?ids,
        "set_in_progress_tool_use_ids"
    );
    state.set_in_progress_tool_use_ids(&action, &ids);
}

pub fn handle_deferred_tool_use(
    state: &mut EngineState,
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
    state.record_deferred_tool_use(id, name, input_preview, reason);
}

pub fn handle_tool_use_summary(
    state: &mut EngineState,
    summary: String,
    preceding_tool_use_ids: Vec<String>,
) {
    tracing::debug!(
        target: "jfc::tool_state",
        summary = %summary,
        ids = ?preceding_tool_use_ids,
        "tool_use_summary"
    );
    state.record_tool_use_summary(summary, preceding_tool_use_ids);
}

fn last_assistant_has_unresolved_tool(state: &EngineState) -> bool {
    state
        .messages
        .iter()
        .rev()
        .find(|msg| msg.role == Role::Assistant)
        .is_some_and(|msg| {
            msg.parts
                .iter()
                .any(|part| matches!(part, MessagePart::Tool(tool) if !tool.status.is_terminal()))
        })
}

fn completed_tool_batch_summary(state: &EngineState) -> Option<(String, Vec<String>)> {
    let last_assistant = state
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

pub fn should_recheck_completion_after_tool_result(state: &EngineState) -> bool {
    !state.is_streaming
        && state.pending_classifications == 0
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        && state.in_flight_eager_dispatches == 0
        && state.in_flight_tool_batches == 0
        && state.compacting_started_at.is_none()
        // A sibling tool's result must not re-drive continuation while an
        // AskUserQuestion modal is still open; only the answer/decline path
        // (which clears `pending_question` first) resumes the turn.
        && state.pending_question.is_none()
        && stream::should_continue_loop(&state.messages)
}

fn post_response_compaction_tokens(state: &EngineState) -> usize {
    let unqueued: Vec<_> = state
        .messages
        .iter()
        .filter(|message| !message.queued)
        .cloned()
        .collect();
    crate::context_accounting::estimate_transcript_tokens(&unqueued)
}

fn post_response_compaction_level(state: &EngineState) -> crate::compact::CompactLevel {
    crate::compact::compact_level_with_output(
        post_response_compaction_tokens(state),
        state.max_context_tokens,
        state.max_output_tokens,
    )
}

/// Handle `ToolEvent::AllComplete` — all tools in the current batch finished.
pub async fn handle_all_complete(state: &mut EngineState, tx: &EventSender) {
    // Decrement the dispatch counters if there are outstanding ones.
    state.in_flight_eager_dispatches = state.in_flight_eager_dispatches.saturating_sub(1);
    state.in_flight_tool_batches = state.in_flight_tool_batches.saturating_sub(1);
    if state.in_flight_tool_batches == 0 && state.in_progress_tool_use_ids.is_empty() {
        state.active_tool_calls.clear();
    }
    tracing::info!(
        target: "jfc::stream",
        message_count = state.messages.len(),
        model = %state.model,
        pending_approvals = state.approval_queue.len() + usize::from(state.pending_approval.is_some()),
        pending_tool_calls = state.pending_tool_calls.len(),
        pending_classifications = state.pending_classifications,
        in_flight_eager = state.in_flight_eager_dispatches,
        in_flight_batches = state.in_flight_tool_batches,
        "ToolEvent::AllComplete"
    );
    if state.is_streaming {
        // A late AllComplete after cancellation may still find queued safe
        // tools. Dispatching is OK: the shared cancellation token makes the
        // scheduler report them as cancelled instead of running side effects.
        let dispatched = super::stream_tool::dispatch_eager_safe_prefix(state, tx);
        if !dispatched.is_empty() {
            tracing::debug!(
                target: "jfc::stream",
                ids = ?dispatched,
                "AllToolsComplete: dispatched next eager-safe prefix"
            );
            return;
        }
    } else if super::stream_tool::dispatch_pending_after_stream(state, tx) {
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
    let turn_truly_complete = state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        && state.pending_classifications == 0
        && state.in_flight_eager_dispatches == 0
        && state.in_flight_tool_batches == 0
        // An open AskUserQuestion modal keeps the turn paused: only the
        // answer/decline path (submit_question/decline_question) may resume it.
        && state.pending_question.is_none();
    if !turn_truly_complete {
        tracing::debug!(
            target: "jfc::stream",
            "AllToolsComplete: batch finished but turn still has pending tools — deferring side effects"
        );
        return;
    }
    if last_assistant_has_unresolved_tool(state) {
        tracing::warn!(
            target: "jfc::stream",
            "AllToolsComplete arrived before all tool results were recorded — waiting for ToolResult recheck"
        );
        return;
    }
    if let Some((summary, preceding_tool_use_ids)) = completed_tool_batch_summary(state) {
        let _ = tx.try_send(EngineEvent::Tool(crate::runtime::ToolEvent::UseSummary {
            summary,
            preceding_tool_use_ids,
        }));
    }
    // Save session once per completed tool batch (not per tool) —
    // debounced: in agentic bursts (many quick tool batches) this was a
    // full-transcript deep clone per batch; the trailing save still
    // lands the newest state within MIN_SAVE_INTERVAL.
    crate::runtime::session_save::request_save(state);

    // Slop Guard aggregation: scan the last assistant
    // message's tool results for slop_guard findings.
    // If any are present, inject a system-reminder so
    // the model sees the aggregate findings on its next turn.
    {
        let marker = crate::tools::SLOP_GUARD_MARKER;
        let mut aggregate_findings: Vec<String> = Vec::new();
        if let Some(last_assistant) = state
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
            crate::system_reminder::append_to_last_user(&mut state.messages, &reminder_body);
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
    let manual = std::mem::take(&mut state.force_compact_pending);
    // Guard: don't spawn another compact if one is already in flight.
    // Without this, every AllToolsComplete while context > threshold
    // spawns a NEW compact task — if the provider doesn't support
    // compaction (returns Unsupported), the tasks pile up at ~12/sec
    // and spam 79K+ WARN lines per session. Only `manual` (/compact)
    // bypasses the guard to let the user force a retry.
    if state.compacting_started_at.is_some() && !manual {
        tracing::debug!(
            target: "jfc::compact",
            "skipping post-response compact — one already in flight"
        );
    } else if state.compact_suppressed && !manual {
        tracing::debug!(
            target: "jfc::compact",
            "skipping post-response compact — suppressed after permanent failure"
        );
    } else {
        if !manual {
            let level = post_response_compaction_level(state);
            let saved_tokens = crate::compact::microcompact_if_helpful(
                &mut state.messages,
                &mut state.tool_ctx.approx_tokens,
                level,
            );
            if saved_tokens > 0 {
                tracing::info!(
                    target: "jfc::compact::micro",
                    saved_tokens,
                    new_est = state.tool_ctx.approx_tokens,
                    "post-response microcompaction applied"
                );
            }
        }
        let level = post_response_compaction_level(state);
        if manual
            || matches!(
                level,
                crate::compact::CompactLevel::Compact | crate::compact::CompactLevel::Blocked
            )
        {
            if manual {
                // /compact is the user's explicit override — clear
                // BOTH the suppression flag AND the rapid-refill
                // counter. Otherwise a previously tripped breaker
                // would still fast-fail this manual attempt.
                state.compact_suppressed = false;
                state.tool_ctx.rapid_refill_count = 0;
            }
            tracing::info!(
                target: "jfc::compact",
                manual,
                compaction_tokens = post_response_compaction_tokens(state),
                cache_inclusive_tokens = state.tool_ctx.approx_tokens,
                level = ?level,
                model = %state.model,
                max_context_tokens = state.max_context_tokens,
                message_count = state.messages.len(),
                rapid_refill_count = state.tool_ctx.rapid_refill_count,
                "post-response compaction triggered"
            );
            // Set the compaction guard synchronously so the agentic
            // loop continuation check (below) sees it immediately.
            // The CompactionStarted event still fires for the UI
            // spinner, but the guard must be synchronous to prevent
            // the race where continue_agentic_loop fires before the
            // async event is processed.
            state.compacting_started_at = Some(std::time::Instant::now());
            state.compacting_output_chars = 0;
            state.compacting_attempt_baseline = 0;
            state.compacting_last_progress = 0;
            let _ = tx
                .send(EngineEvent::Compaction(CompactionEvent::Started))
                .await;
            let messages = state.messages.clone();
            let provider = Arc::clone(&state.provider);
            let model = state.model.clone();
            let mut tool_ctx = state.tool_ctx.clone();
            let window = state.max_context_tokens;
            let state_max_output_tokens = state.max_output_tokens;
            let tx_compact = tx.clone();
            let progress_tx = tx_compact.clone();
            let session_id_for_compact = state
                .current_session_id
                .as_ref()
                .map(|s| s.as_str().to_owned())
                .unwrap_or_else(|| "<no-session>".to_owned());
            let on_progress: crate::compact::CompactProgressCb = Box::new(move |chars| {
                // CompactionProgress is non-critical; next progress update supersedes.
                let _ = progress_tx.try_send(EngineEvent::Compaction(CompactionEvent::Progress {
                    output_chars: chars,
                }));
            });
            // wg-async: compact holds critical state (the full
            // message slice + an outbound tx). Race the long
            // provider call against `cancelled()` so ESC×2
            // mid-compact doesn't leave it running for ~30s
            // sending CompactionDone into a stale state.
            let cancel_compact = state.cancel_token.clone();
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
                // Fire BeforeCompact hook
                crate::hooks::fire(
                    crate::hooks::HookPoint::BeforeCompact,
                    &crate::hooks::HookContext::for_session(&session_id_for_compact),
                );
                let result = tokio::select! {
                    biased;
                    _ = cancel_compact.cancelled() => {
                        tracing::info!(
                            target: "jfc::compact",
                            "compaction cancelled via token"
                        );
                        let _ = tx_compact
                            .send(EngineEvent::Compaction(CompactionEvent::Failed {
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
                        state_max_output_tokens,
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
                            .send(EngineEvent::Compaction(CompactionEvent::Done {
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
                            .send(EngineEvent::Compaction(CompactionEvent::Failed {
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
                            .send(EngineEvent::Compaction(CompactionEvent::Failed {
                                reason:
                                    "Nothing to compact yet — only one conversation turn so far. \
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
                            .send(EngineEvent::Compaction(CompactionEvent::Failed {
                                reason: "Circuit breaker tripped — compaction keeps refilling"
                                    .into(),
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
                            .send(EngineEvent::Compaction(CompactionEvent::Failed {
                                reason: format!("Exhausted {attempts} compaction attempts"),
                                calibrated_tokens: None,
                                transient: false,
                            }))
                            .await;
                    }
                }
                // Fire AfterCompact hook
                crate::hooks::fire(
                    crate::hooks::HookPoint::AfterCompact,
                    &crate::hooks::HookContext::for_session(&session_id_for_compact),
                );
            });
        }
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
    if state
        .interrupt_flag
        .load(std::sync::atomic::Ordering::SeqCst)
        || state.cancel_token.is_cancelled()
    {
        tracing::info!(
            target: "jfc::stream",
            "agentic loop NOT continuing — user requested interrupt"
        );
        // Clear so the next user submission starts fresh —
        // both the legacy flag and the (possibly cancelled)
        // token need refreshing for the next spawn cycle.
        state
            .interrupt_flag
            .store(false, std::sync::atomic::Ordering::SeqCst);
        state.cancel_token = tokio_util::sync::CancellationToken::new();
        state.is_streaming = false;
        state.streaming_started_at = None;
        state.last_stream_event_at = None;
        state.streaming_last_token_at = None;
        state.token_rate_samples.clear();
        state.token_rate_sample_thinking = None;
        state.thinking_started_at = None;
        state.thinking_ended_at = None;
        state.streaming_text.clear();
        state.streaming_reasoning.clear();
        state.streaming_response_bytes = 0;
        state.streaming_response_baseline = 0;
        state.streaming_thinking_tokens = 0;
        state.streaming_assistant_idx = None;
        state.clear_active_stream_scope();
        state.current_stream_request = None;
        state.stream_lifecycle = None;
        state.turn_started_at = None;
    } else if state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.compacting_started_at.is_none()
        && state.pending_question.is_none()
        && stream::should_continue_loop(&state.messages)
    {
        // Fan-out consolidation: if multiple parallel agent
        // tasks completed in this batch, inject a summary
        // so the model sees a coherent digest before responding.
        if let Some(last_assistant) = state
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
                    &mut state.messages,
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
        // state.pending_pause_turn_resume docs and cli.js
        // v142:622686 for the protocol.
        //
        // Single-shot: clear the flag before dispatching
        // so a follow-up turn that doesn't pause_turn
        // returns to the normal continue_agentic_loop
        // path. If the resumed turn ALSO pause_turns,
        // its own Done handler re-latches the flag.
        if state.pending_pause_turn_resume {
            state.pending_pause_turn_resume = false;
            tracing::info!(
                target: "jfc::stream",
                "mixed-mode pause_turn: local tools complete, resuming server-side sampling loop"
            );
            stream::continue_after_pause_turn(state, tx).await;
        } else if !state.queued_prompts.is_empty() {
            // Drain whenever ANY prompt is queued (meta OR non-meta).
            // Previously this only fired for non-meta prompts, which
            // meant slash commands (`/tasks`, `/market`, local actions)
            // submitted mid-stream would sit in the queue until the
            // entire agentic loop concluded — making the TUI appear
            // to silently ignore them.
            tracing::info!(
                target: "jfc::stream",
                queued = state.queued_prompts.len(),
                "agentic loop yielding to queued prompt before continuation"
            );
            drain_queued_prompts(state, tx).await;
        } else {
            tracing::info!(
                target: "jfc::stream",
                "agentic loop continuing — tools complete, no pending approvals"
            );
            stream::continue_agentic_loop(state, tx).await;
        }
    } else if !state.is_streaming
        && state.pending_approval.is_none()
        && state.approval_queue.is_empty()
        && state.pending_tool_calls.is_empty()
        && state.pending_question.is_none()
    {
        tracing::debug!(
            target: "jfc::stream",
            "turn fully ended — draining queued prompts"
        );
        // Turn fully ended (model stopped, no more agentic loop
        // iterations, no pending tools). Clear turn_started_at
        // so the spinner stops, then drain any prompts the user
        // typed during streaming.
        state.turn_started_at = None;
        // /goal stop-hook: if a goal is active, the agent
        // doesn't truly get to stop here. Fire the
        // evaluator in the background; the agentic loop
        // re-enters when the verdict lands (see
        // GoalEvent::Verdict). Bail before draining
        // queued prompts so a queued prompt can't race
        // ahead of the verdict and unset the goal mid-eval.
        if dispatch_goal_evaluator_if_active(state, tx) {
            tracing::info!(
                target: "jfc::goal",
                "goal evaluator dispatched on EndTurn — deferring drain"
            );
        } else {
            drain_queued_prompts(state, tx).await;
            maybe_continue_task_factory(state, tx).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::app::EngineState;
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

    fn test_app_with_tool(status: ToolStatus) -> (EngineState, crate::ids::ToolId) {
        test_app_with_tool_call(
            status,
            ToolKind::Bash,
            ToolInput::Generic {
                summary: String::new(),
            },
        )
    }

    fn test_app_with_tool_call(
        status: ToolStatus,
        kind: ToolKind,
        input: ToolInput,
    ) -> (EngineState, crate::ids::ToolId) {
        let tool_id = crate::ids::ToolId::from("tool-1");
        let mut tool = ToolCall::new_pending(tool_id.clone(), kind, input);
        tool.status = status;

        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.messages.push(ChatMessage::user("run tool".into()));
        state
            .messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::tool(tool)]));
        state.turn_started_at = Some(Instant::now());
        state.is_streaming = false;
        (state, tool_id)
    }

    fn pending_question(tool_id: &str) -> crate::app::PendingQuestion {
        use crate::app::{PendingQuestion, QuestionItem, QuestionOption};
        PendingQuestion {
            tool_id: crate::ids::ToolId::from(tool_id),
            items: vec![QuestionItem {
                question: "Pick one?".to_owned(),
                header: "Pick".to_owned(),
                options: vec![QuestionOption {
                    label: "A".to_owned(),
                    description: String::new(),
                    preview: None,
                }],
                multi_select: false,
                selected: 0,
                chosen: std::collections::BTreeSet::new(),
                other_text: String::new(),
                answer: None,
            }],
            current: 0,
            editing_other: false,
        }
    }

    #[test]
    fn post_response_compaction_ignores_cache_inflated_usage_regression() {
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.max_context_tokens = 200_000;
        state.max_output_tokens = Some(64_000);
        state.tool_ctx.approx_tokens = 172_170;
        state.messages = (0..20)
            .map(|idx| ChatMessage::user(format!("message-{idx} {}", "x".repeat(12_000))))
            .collect();

        let cache_inclusive_level = crate::compact::compact_level_with_output(
            state.tool_ctx.approx_tokens,
            state.max_context_tokens,
            state.max_output_tokens,
        );

        assert!(matches!(
            cache_inclusive_level,
            crate::compact::CompactLevel::Compact | crate::compact::CompactLevel::Blocked
        ));
        assert!(
            post_response_compaction_tokens(&state)
                < crate::compact::compact_threshold_with_output(
                    state.max_context_tokens,
                    state.max_output_tokens,
                ),
            "local transcript should still be below compact threshold"
        );
        assert_eq!(
            post_response_compaction_level(&state),
            crate::compact::CompactLevel::Ok,
            "post-response compaction must not fire just because prompt-cache usage is high"
        );
    }

    #[tokio::test]
    async fn all_complete_blocks_while_question_pending_regression() {
        // A completed sibling tool's AllComplete must NOT continue the turn or
        // drain queued prompts while an AskUserQuestion modal is open — only the
        // answer/decline path may resume it. Without the pending_question guard
        // the continuation gate (should_continue_loop is true here because the
        // sibling tool is terminal) drains the queued prompt mid-question.
        let (mut state, _tool_id) = test_app_with_tool(ToolStatus::Completed);
        state.pending_question = Some(pending_question("q1"));
        state
            .queued_prompts
            .push_later("next prompt".to_owned(), false, Vec::new());
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_all_complete(&mut state, &tx).await;

        assert_eq!(
            state.queued_prompts.len(),
            1,
            "queued prompt must not drain while a question is open"
        );
        assert!(!state.is_streaming, "turn must stay paused, not re-stream");
        assert!(
            state.pending_question.is_some(),
            "question must remain pending until answered/declined"
        );
    }

    #[test]
    fn successful_apply_patch_records_turn_edited_files_normal() {
        let patch = "*** Begin Patch\n*** Update File: src/lib.rs\n@@\n-old\n+new\n*** Add File: src/new.rs\n+new\n*** End Patch\n";
        let (mut state, tool_id) = test_app_with_tool_call(
            ToolStatus::Pending,
            ToolKind::ApplyPatch,
            ToolInput::ApplyPatch {
                patch: patch.to_owned(),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_tool_result(
            &mut state,
            &tx,
            tool_id,
            crate::runtime::ExecutionResult::success("ok"),
        );

        assert!(state.turn_edited_files.contains("src/lib.rs"));
        assert!(state.turn_edited_files.contains("src/new.rs"));
    }

    #[test]
    fn successful_multi_edit_records_turn_edited_file_normal() {
        let (mut state, tool_id) = test_app_with_tool_call(
            ToolStatus::Pending,
            ToolKind::MultiEdit,
            ToolInput::MultiEdit {
                file_path: "src/lib.rs".to_owned(),
                edits: serde_json::json!([]),
            },
        );
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_tool_result(
            &mut state,
            &tx,
            tool_id,
            crate::runtime::ExecutionResult::success("ok"),
        );

        assert!(state.turn_edited_files.contains("src/lib.rs"));
    }

    #[test]
    fn bash_output_result_mutates_originating_bash_tool_regression() {
        let task_id = "bash_fe28c4f9154a";
        let bash_id = crate::ids::ToolId::from("bash-tool");
        let bash_output_id = crate::ids::ToolId::from("bash-output-tool");
        let mut bash = ToolCall::new_pending(
            bash_id,
            ToolKind::Bash,
            ToolInput::Bash {
                command: "echo hello".into(),
                timeout: None,
                workdir: None,
                run_in_background: Some(true),
                suppress_output: None,
            },
        );
        bash.status = ToolStatus::Completed;
        bash.output = ToolOutput::Text(format!(
            "Command running in background.\ntask_id: {task_id}\nstatus: running"
        ));
        let bash_output = ToolCall::new_pending(
            bash_output_id.clone(),
            ToolKind::BashOutput,
            ToolInput::BashOutput {
                task_id: task_id.into(),
                offset: None,
                limit: None,
                block: None,
                timeout: None,
                wait_up_to: None,
            },
        );
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state.messages.push(ChatMessage::assistant_parts(vec![
            MessagePart::tool(bash),
            MessagePart::tool(bash_output),
        ]));
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let result = concat!(
            "retrieval_status: success\n",
            "task_id: bash_fe28c4f9154a\n",
            "status: completed exit=0\n",
            "\n",
            "hello\n",
        );

        handle_tool_result(
            &mut state,
            &tx,
            bash_output_id,
            crate::runtime::ExecutionResult::success(result),
        );

        let assistant = state.messages.last().expect("assistant message");
        let mut tools = assistant.parts.iter().filter_map(|part| match part {
            MessagePart::Tool(tool) => Some(tool.as_ref()),
            _ => None,
        });
        let bash = tools.next().expect("bash tool");
        let bash_output = tools.next().expect("bash output tool");
        assert!(
            matches!(&bash.output, ToolOutput::Text(text) if text.contains("retrieval_status: success") && text.contains("hello")),
            "Bash tool should own folded output, got {:?}",
            bash.output
        );
        assert!(
            matches!(&bash_output.output, ToolOutput::Text(text) if text.contains("retrieval_status: success") && text.contains("hello")),
            "BashOutput tool should still preserve model-facing result, got {:?}",
            bash_output.output
        );
    }

    #[test]
    fn background_bash_result_updates_original_tool_without_taskoutput_regression() {
        let bash_id = crate::ids::ToolId::from("bash-tool");
        let mut bash = ToolCall::new_pending(
            bash_id.clone(),
            ToolKind::Bash,
            ToolInput::Bash {
                command: "printf done".into(),
                timeout: None,
                workdir: None,
                run_in_background: Some(true),
                suppress_output: None,
            },
        );
        bash.status = ToolStatus::Completed;
        bash.output = ToolOutput::Text(
            "Command running in background.\ntask_id: bash_1177aa44beef\nstatus: running".into(),
        );
        let mut state = EngineState::new(Arc::new(TestProvider), "test-model");
        state.task_store = jfc_session::TaskStore::in_memory();
        state
            .messages
            .push(ChatMessage::assistant_parts(vec![MessagePart::tool(bash)]));

        handle_background_tool_result(
            &mut state,
            bash_id,
            crate::runtime::ExecutionResult::success("background-done"),
        );

        let assistant = state.messages.last().expect("assistant message");
        let tools: Vec<_> = assistant
            .parts
            .iter()
            .filter_map(|part| match part {
                MessagePart::Tool(tool) => Some(tool.as_ref()),
                _ => None,
            })
            .collect();
        assert_eq!(
            tools.len(),
            1,
            "must not create a TaskOutput/BashOutput block"
        );
        assert_eq!(tools[0].status, ToolStatus::Completed);
        assert!(
            matches!(&tools[0].output, ToolOutput::Text(text) if text == "background-done"),
            "Bash tool should own final output, got {:?}",
            tools[0].output
        );
    }

    #[tokio::test]
    async fn all_complete_waits_for_out_of_order_tool_result() {
        let (mut state, _tool_id) = test_app_with_tool(ToolStatus::Pending);
        state.in_flight_tool_batches = 1;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_all_complete(&mut state, &tx).await;

        assert_eq!(state.in_flight_tool_batches, 0);
        assert!(state.turn_started_at.is_some());
        assert!(!should_recheck_completion_after_tool_result(&state));
    }

    #[test]
    fn late_tool_result_rechecks_completion_after_batch_signal_race() {
        let (mut state, tool_id) = test_app_with_tool(ToolStatus::Pending);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_tool_result(
            &mut state,
            &tx,
            tool_id,
            crate::runtime::ExecutionResult::success("ok"),
        );

        assert!(should_recheck_completion_after_tool_result(&state));
    }

    #[tokio::test]
    async fn tool_result_clears_in_progress_bookkeeping_directly_regression() {
        let (mut state, tool_id) = test_app_with_tool(ToolStatus::Pending);
        state.set_in_progress_tool_use_ids("add", &[tool_id.as_str().to_owned()]);
        assert!(state.has_interruptible_work());
        let (tx, _rx) = tokio::sync::mpsc::channel(1);

        handle_tool_result(
            &mut state,
            &tx,
            tool_id.clone(),
            crate::runtime::ExecutionResult::failure("Tool cancelled by user"),
        );

        assert!(
            !state.in_progress_tool_use_ids.contains(tool_id.as_str()),
            "tool completion must not depend on a queued remove event"
        );
    }

    #[test]
    fn stale_success_result_does_not_overwrite_failed_tool_output() {
        let (mut state, tool_id) = test_app_with_tool(ToolStatus::Failed);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        if let Some(MessagePart::Tool(tool)) = state.messages[1].parts.get_mut(0) {
            tool.output = ToolOutput::Text("Denied by permission mode".to_owned());
        }

        handle_tool_result(
            &mut state,
            &tx,
            tool_id,
            crate::runtime::ExecutionResult::success("late stale result"),
        );

        let MessagePart::Tool(tool) = &state.messages[1].parts[0] else {
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
        let (mut state, _tool_id) = test_app_with_tool(ToolStatus::Pending);
        handle_deferred_tool_use(
            &mut state,
            "tool-1".into(),
            "Bash".into(),
            "ls".into(),
            "awaiting_approval".into(),
        );
        assert_eq!(state.deferred_tool_uses.len(), 1);

        handle_set_in_progress_tool_use_ids(&mut state, "add".into(), vec!["tool-1".into()]);
        assert!(state.in_progress_tool_use_ids.contains("tool-1"));
        assert!(state.deferred_tool_uses.is_empty());

        handle_set_in_progress_tool_use_ids(&mut state, "remove".into(), vec!["tool-1".into()]);
        assert!(!state.in_progress_tool_use_ids.contains("tool-1"));
    }

    #[test]
    fn completed_tool_batch_summary_single_tool_normal() {
        let (state, _tool_id) = test_app_with_tool(ToolStatus::Completed);
        let (summary, ids) =
            completed_tool_batch_summary(&state).expect("completed tool should summarize");
        assert!(summary.starts_with("Ran"), "{summary}");
        assert_eq!(ids, vec!["tool-1"]);
    }

    #[test]
    fn output_chunk_preview_is_capped_robust() {
        let (mut state, tool_id) = test_app_with_tool(ToolStatus::Running);
        // Push far more than the cap: 4096 chunks × ~100 bytes ≈ 400 KB.
        let chunk = "x".repeat(99);
        for _ in 0..4096 {
            handle_output_chunk(&mut state, tool_id.clone(), chunk.clone());
        }
        let MessagePart::Tool(tool) = &state.messages[1].parts[0] else {
            panic!("expected tool part");
        };
        let ToolOutput::Text(s) = &tool.output else {
            panic!("expected text output");
        };
        assert!(
            s.len() <= LIVE_OUTPUT_PREVIEW_CAP_BYTES,
            "live preview must stay capped: {} > {}",
            s.len(),
            LIVE_OUTPUT_PREVIEW_CAP_BYTES
        );
        // Tail (newest output) is what's kept.
        assert!(s.ends_with("x\n"));
    }

    #[test]
    fn trim_front_to_cap_respects_char_boundaries_robust() {
        // Multi-byte chars at the cut point must not panic.
        let mut s = "é".repeat(1000);
        trim_front_to_cap(&mut s, 100);
        assert!(s.len() <= 100);
        assert!(s.chars().all(|c| c == 'é'));
        // Under-cap input untouched.
        let mut small = String::from("abc");
        trim_front_to_cap(&mut small, 100);
        assert_eq!(small, "abc");
    }
}
