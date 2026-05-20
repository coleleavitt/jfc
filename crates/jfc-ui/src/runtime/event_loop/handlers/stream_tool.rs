//! `StreamEvent::Tool`, `ToolEvent::ClassifierDecision`, and
//! `StreamEvent::ServerToolResult` handlers — tool announcement routing.

use std::sync::Arc;

use crate::app::{App, PendingApproval};
use crate::runtime::{AppEvent, EventSender, ToolEvent};
use crate::types::*;

use super::super::guards::streaming_assistant_mut;

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
        tracing::info!(
            target: "jfc::ui::tool",
            tool_kind = tool.kind.label(),
            tool_id = %tool.id,
            pending_total = app.pending_tool_calls.len() + 1,
            "route=auto_dispatch (no approval needed)"
        );
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::Tool(tool.clone()));
        }
        app.pending_tool_calls.push(tool);
    }
}

/// Handle the auto-mode classifier's verdict on a tool call.
pub(crate) fn handle_classifier_decision(
    app: &mut App,
    mut tool: ToolCall,
    blocked: bool,
    reason: String,
) {
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
    let result_bytes = content.to_string().len() as u64;
    app.network_bytes_in = app.network_bytes_in.saturating_add(result_bytes);
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
