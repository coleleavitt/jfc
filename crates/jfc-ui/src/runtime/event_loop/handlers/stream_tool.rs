//! `StreamEvent::Tool`, `ToolEvent::ClassifierDecision`, and
//! `StreamEvent::ServerToolResult` handlers — tool announcement routing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::app::{App, PendingApproval};
use crate::runtime::{AppEvent, EventSender, ToolEvent};
use crate::types::*;
use crate::{toast, toast::ToastKind};

use super::super::guards::streaming_assistant_mut;

/// One-time flag: ensures the "Tools auto-approved (sandboxed)" toast
/// is only pushed once per process lifetime.
static SANDBOX_TOAST_SHOWN: AtomicBool = AtomicBool::new(false);

/// Push a one-time toast informing the user that tools are being auto-approved
/// because the process is running inside a landlock sandbox.
fn maybe_show_sandbox_toast(app: &mut App) {
    if crate::is_sandbox_active() && !SANDBOX_TOAST_SHOWN.swap(true, Ordering::Relaxed) {
        toast::push_with_cap(
            &mut app.toasts,
            toast::Toast::new(ToastKind::Info, "Tools auto-approved (sandboxed)"),
        );
    }
}

/// Handle a new tool announced by the stream layer.
pub(crate) async fn handle_stream_tool(app: &mut App, tx: &EventSender, tool: ToolCall) {
    app.record_stream_activity();
    // Trace every StreamTool entry so next-run diagnostics show
    // exactly which routing path each tool took. Without this,
    // tools that take the auto-mode or no-approval branches are
    // invisible in logs (only the approval path was traced),
    // making bugs like "tool stuck Pending" undiagnosable.
    tracing::info!(
        target: "jfc::ui::tool",
        tool_kind = tool.kind.label(),
        tool_id = %tool.id,
        auto_mode = app.auto_mode.enabled,
        needs_approval = app.tool_needs_approval(&tool),
        streaming_idx = ?app.streaming_assistant_idx,
        "StreamTool received"
    );
    // Guard 1: a tool that arrived already terminal (the stream
    // layer builds `ToolCall::new_failed` for malformed provider
    // input — bad JSON or schema mismatch) must NOT be dispatched.
    // Dispatching it routes a `kind`/`input` mismatch into
    // `execute_tool`, which falls through to the catch-all arm and
    // clobbers the original diagnostic with a misleading error.
    // Just record it in the transcript so the model sees the
    // tool_result it can react to.
    if tool.status.is_terminal() {
        tracing::info!(
            target: "jfc::ui::tool",
            tool_kind = tool.kind.label(),
            tool_id = %tool.id,
            status = tool.status.label(),
            "route=terminal_on_arrival (no dispatch)"
        );
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::Tool(tool));
        }
    } else if let Some(reason) = app.tool_denied_by_mode(&tool) {
        // Guard 2: the active permission mode auto-denies this
        // tool (e.g. Plan mode blocking a Write, or an
        // UnknownTool in any mode). `tool_needs_approval` returns
        // false for `Denied`, so without this guard the tool
        // would fall into the no-approval auto-dispatch branch
        // and execute anyway. Mark it Failed with the denial
        // reason and record it instead.
        tracing::info!(
            target: "jfc::ui::tool",
            tool_kind = tool.kind.label(),
            tool_id = %tool.id,
            reason,
            "route=denied_by_mode (no dispatch)"
        );
        let mut tool = tool;
        let _ = tool.mark_failed();
        tool.output = ToolOutput::Text(format!("Denied by permission mode: {reason}"));
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::Tool(tool));
        }
    } else if app.auto_mode.enabled {
        // v126 auto-mode: when enabled, every tool call is sent to a
        // classifier LLM that returns block/allow with a reason. The
        // user is never prompted. Disabled (default) → original flow.
        tracing::info!(
            target: "jfc::ui::tool",
            tool_id = %tool.id,
            "route=auto_mode_classifier"
        );
        // Mark a classifier verdict as in-flight so stream_done holds the
        // turn open until it lands (see App::pending_classifications). The
        // matching decrement is in handle_classifier_decision; the verdict is
        // dropped (no decrement) only on cancellation, which the turn-start
        // reset cleans up.
        app.pending_classifications += 1;
        let provider = Arc::clone(&app.provider);
        let model = app.model.clone();
        let cfg = app.auto_mode.clone();
        let history = app.messages.clone();
        let tx_cls = tx.clone();
        let tool_for_task = tool.clone();
        // wg-async: classifier issues a provider call
        // (often 2-5s). Race against cancellation so an
        // ESC×2 unblocks the user-visible tool decision
        // instead of letting it land in a cancelled turn.
        let cancel_cls = app.cancel_token.clone();
        tokio::spawn(async move {
            let decision = tokio::select! {
                biased;
                _ = cancel_cls.cancelled() => return,
                d = crate::auto_mode::classify(
                    provider.as_ref(),
                    &model,
                    &cfg,
                    &history,
                    &tool_for_task,
                ) => d,
            };
            let _ = tx_cls
                .send(AppEvent::Tool(ToolEvent::ClassifierDecision {
                    tool: tool_for_task,
                    blocked: decision.should_block(),
                    reason: decision.reason,
                }))
                .await;
        });
    } else if app.tool_needs_approval(&tool) {
        // Insert the tool into the assistant message *immediately*
        // with status Pending so the user can SEE that the model
        // wants to call N tools — without this, only the assistant
        // text rendered and queued tools were invisible until each
        // got dispatched. The dispatch path mutates the same
        // ToolCall entry by id when ToolResult arrives, flipping
        // status to Complete/Failed and setting output.
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::Tool(tool.clone()));
        }
        // First approvable tool fills `pending_approval`; every
        // subsequent one queues behind it. The decide-handlers in
        // input.rs pop the next from `approval_queue` after each
        // verdict so the modal cycles through them in order.
        let kind_label = tool.kind.label();
        let tool_id = tool.id.clone();
        if app.pending_approval.is_none() {
            tracing::info!(
                target: "jfc::ui::approval",
                tool_kind = kind_label,
                tool_id = %tool_id,
                "modal_opened"
            );
            app.pending_approval = Some(PendingApproval { tool, selected: 0 });
        } else {
            tracing::info!(
                target: "jfc::ui::approval",
                tool_kind = kind_label,
                tool_id = %tool_id,
                queue_depth = app.approval_queue.len() + 1,
                "queued_behind_modal"
            );
            app.approval_queue.push_back(tool);
        }
    } else {
        // Sandbox auto-approval toast (first time only per session).
        maybe_show_sandbox_toast(app);
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::Tool(tool.clone()));
        }
        // Streaming tool execution v2 (gate: streaming-tool-exec):
        // Start executing this tool immediately instead of waiting for
        // StreamDone. This eliminates the perceived "queuing" where tools
        // only run after the model finishes its entire response. Safe
        // because tool_use blocks are complete at content_block_stop —
        // name, ID, and input JSON are all finalized.
        //
        // When the gate is off, tools accumulate in pending_tool_calls and
        // are dispatched as a batch when StreamDone arrives.
        if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::StreamingToolExec) {
            tracing::info!(
                target: "jfc::ui::tool",
                tool_kind = tool.kind.label(),
                tool_id = %tool.id,
                pending_total = app.pending_tool_calls.len() + 1,
                "route=eager_dispatch (streaming-tool-exec ON, no approval needed)"
            );
            // Track the id in pre_dispatched_tool_ids so stream_done doesn't
            // re-dispatch it; still push to pending_tool_calls so the turn-
            // complete logic knows there are outstanding tools.
            app.pre_dispatched_tool_ids
                .insert(tool.id.as_str().to_owned());
            app.pending_tool_calls.push(tool.clone());
            app.in_flight_eager_dispatches += 1;
            let calls = vec![tool];
            crate::runtime::update_task_activities(app, &calls);
            crate::stream::dispatch_tools_batched(
                calls,
                tx,
                Arc::clone(&app.dedup_cache),
                Some(Arc::clone(&app.task_store)),
                app.team_context.team_name.clone(),
                app.current_session_id
                    .as_ref()
                    .map(|id| id.as_str().to_owned()),
                Arc::clone(&app.provider),
                app.model.clone(),
                app.teammate_event_tx.clone(),
                app.cancel_token.clone(),
            );
        } else {
            tracing::info!(
                target: "jfc::ui::tool",
                tool_kind = tool.kind.label(),
                tool_id = %tool.id,
                pending_total = app.pending_tool_calls.len() + 1,
                "route=deferred_dispatch (streaming-tool-exec OFF, no approval needed)"
            );
            app.pending_tool_calls.push(tool);
        }
    }
}

/// Handle the auto-mode classifier's verdict on a tool call.
pub(crate) async fn handle_classifier_decision(
    app: &mut App,
    tx: &EventSender,
    mut tool: ToolCall,
    blocked: bool,
    reason: String,
) {
    app.pending_classifications = app.pending_classifications.saturating_sub(1);
    if blocked {
        tool.status = ToolStatus::Failed;
        tool.output = ToolOutput::Text(format!(
            "Auto-mode classifier blocked this tool call.\n\nReason: {reason}"
        ));
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::Tool(tool));
        }
    } else {
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::Tool(tool.clone()));
        }
        app.pending_tool_calls.push(tool);
    }

    // If the stream already finished while verdicts were outstanding (so
    // stream_done held the turn open via `pending_classifications`) and this
    // was the last one, drive the turn forward now — otherwise the approved
    // tools sit in pending_tool_calls forever and the loop stalls. While the
    // stream is still active (`is_streaming`), defer: stream_done will
    // dispatch normally once it ends with the counter back at 0.
    let resolved_while_idle = !app.is_streaming
        && app.pending_classifications == 0
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty();
    if !resolved_while_idle {
        return;
    }
    if !app.pending_tool_calls.is_empty() {
        let calls = std::mem::take(&mut app.pending_tool_calls);
        crate::runtime::update_task_activities(app, &calls);
        crate::stream::dispatch_tools_batched(
            calls,
            tx,
            Arc::clone(&app.dedup_cache),
            Some(Arc::clone(&app.task_store)),
            app.team_context.team_name.clone(),
            app.current_session_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
            Arc::clone(&app.provider),
            app.model.clone(),
            app.teammate_event_tx.clone(),
            app.cancel_token.clone(),
        );
    } else {
        // Every tool was blocked — no batch to run, but the loop must still
        // continue so the model sees the blocked tool_results it produced.
        crate::runtime::send_critical(tx, AppEvent::Tool(ToolEvent::AllComplete));
    }
}

/// Handle a server-side tool result (e.g. web_search_tool_result).
pub(crate) fn handle_server_tool_result(
    app: &mut App,
    tool_use_id: crate::ids::ToolId,
    tool_kind: jfc_provider::ServerToolResultKind,
    content: serde_json::Value,
) {
    // Anthropic emitted a server_tool_result block (e.g.
    // web_search_tool_result) paired with a previously-
    // dispatched server_tool_use. Find the matching
    // ToolCall on the streaming assistant message and
    // replace its output. Marking the tool Completed
    // here closes out the server-side execution; the
    // result is preserved on `tool.output` as a
    // `ToolOutput::ServerToolResult` for byte-faithful
    // round-trip on resend.
    app.record_stream_activity();
    let mut applied = false;
    if let Some(idx) = app.streaming_assistant_idx
        && let Some(msg) = app.messages.get_mut(idx)
    {
        for part in msg.parts.iter_mut() {
            if let MessagePart::Tool(tc) = part
                && tc.id == tool_use_id
            {
                tc.output = ToolOutput::ServerToolResult {
                    tool_kind: tool_kind.clone(),
                    content: content.clone(),
                };
                // Tool was set Running on the
                // server_tool_use block; flip to
                // Completed now that the paired
                // result has arrived. mark_completed
                // is idempotent if it ever fires
                // twice from a duplicate event.
                let _ = tc.mark_completed();
                applied = true;
                break;
            }
        }
    }
    if !applied {
        // Result arrived without a matching server_tool_use
        // ToolCall on the streaming message. Most likely
        // cause: the user pressed ESCx2 between the
        // server_tool_use start and the result block, the
        // streaming slot was cleared, and the late result
        // landed orphaned. Log loudly so the case is
        // visible in the trace but don't crash the run.
        tracing::warn!(
            target: "jfc::stream",
            tool_use_id = %tool_use_id,
            wire_type = tool_kind.wire_type(),
            streaming_idx = ?app.streaming_assistant_idx,
            "server_tool_result arrived with no matching server_tool_use ToolCall on streaming message"
        );
    }
}
