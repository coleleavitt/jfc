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

fn emit_in_progress(tx: &EventSender, action: &str, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    let _ = tx.try_send(AppEvent::Tool(ToolEvent::SetInProgressToolUseIds {
        action: action.to_owned(),
        ids,
    }));
}

fn emit_deferred_tool_use(tx: &EventSender, tool: &ToolCall, reason: &str) {
    let _ = tx.try_send(AppEvent::Tool(ToolEvent::DeferredToolUse {
        id: tool.id.as_str().to_owned(),
        name: tool.kind.label().to_owned(),
        input_preview: tool.input.summary(),
        reason: reason.to_owned(),
    }));
}

fn tool_ids(tools: &[ToolCall]) -> Vec<String> {
    tools
        .iter()
        .map(|tool| tool.id.as_str().to_owned())
        .collect()
}

fn dispatch_tool_batch(app: &mut App, tx: &EventSender, calls: Vec<ToolCall>, eager: bool) {
    if calls.is_empty() {
        return;
    }
    if eager {
        app.in_flight_eager_dispatches += 1;
        for tool in &calls {
            app.pre_dispatched_tool_ids
                .insert(tool.id.as_str().to_owned());
        }
    }
    app.in_flight_tool_batches += 1;
    emit_in_progress(tx, "add", tool_ids(&calls));
    crate::runtime::update_task_activities(app, &calls);
    crate::stream::dispatch_tools_batched(
        calls,
        crate::stream::ToolBatchDispatch {
            tx: tx.clone(),
            dedup: Arc::clone(&app.dedup_cache),
            task_store: Some(Arc::clone(&app.task_store)),
            active_team_name: app.team_context.team_name.clone(),
            current_session_id: app
                .current_session_id
                .as_ref()
                .map(|id| id.as_str().to_owned()),
            provider: Arc::clone(&app.provider),
            model: app.model.clone(),
            teammate_event_tx: app.teammate_event_tx.clone(),
            local_advisor: crate::stream::LocalAdvisorDispatchContext::from_app(app),
            cancel: app.cancel_token.clone(),
        },
    );
}

/// Dispatch the ordered prefix of pending tools that is safe to run before
/// the stream ends. The prefix stops at the first side-effecting tool so
/// Bash/Edit/Write/ApplyPatch preserve model order and run through the normal
/// scheduler once earlier eager work has settled.
pub(crate) fn dispatch_eager_safe_prefix(app: &mut App, tx: &EventSender) -> Vec<String> {
    if !crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::StreamingToolExec) {
        return Vec::new();
    }
    if app.in_flight_tool_batches > 0
        || app.pending_approval.is_some()
        || !app.approval_queue.is_empty()
        || app.pending_classifications > 0
    {
        return Vec::new();
    }
    let prefix_len = app
        .pending_tool_calls
        .iter()
        .take_while(|tool| crate::scheduler::is_concurrency_safe(&tool.kind))
        .count();
    if prefix_len == 0 {
        return Vec::new();
    }

    let calls: Vec<_> = app.pending_tool_calls.drain(..prefix_len).collect();
    let ids = tool_ids(&calls);
    tracing::info!(
        target: "jfc::ui::tool",
        n = calls.len(),
        ids = ?ids,
        "route=eager_dispatch_prefix (streaming-tool-exec ON)"
    );
    dispatch_tool_batch(app, tx, calls, true);
    ids
}

/// Dispatch all remaining pending tools after the provider stream has ended.
/// If an eager prefix is still running, leave the queue intact; the matching
/// AllComplete handler will call this after the prefix settles, preserving
/// model order between eager read-only tools and later side-effecting tools.
pub(crate) fn dispatch_pending_after_stream(app: &mut App, tx: &EventSender) -> bool {
    if app.pending_tool_calls.is_empty()
        || app.in_flight_tool_batches > 0
        || app.in_flight_eager_dispatches > 0
    {
        return false;
    }

    let all_calls = std::mem::take(&mut app.pending_tool_calls);
    let calls: Vec<_> = all_calls
        .into_iter()
        .filter(|tool| !app.pre_dispatched_tool_ids.contains(tool.id.as_str()))
        .collect();
    if calls.is_empty() {
        crate::runtime::send_critical(tx, AppEvent::Tool(ToolEvent::AllComplete));
        return false;
    }

    tracing::info!(
        target: "jfc::stream",
        n = calls.len(),
        kinds = ?calls.iter().map(|tool| tool.kind.label()).collect::<Vec<_>>(),
        "stream_done dispatching ordered pending tool batch"
    );
    dispatch_tool_batch(app, tx, calls, false);
    true
}

/// Handle a new tool announced by the stream layer.
pub(crate) async fn handle_stream_tool(app: &mut App, tx: &EventSender, tool: Box<ToolCall>) {
    app.record_stream_activity();
    app.stream_lifecycle = None;
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
    app.exploration_state.record_tool_call(&tool);
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
            msg.parts.push(MessagePart::tool_boxed(tool));
        }
    } else if matches!(
        tool.kind,
        ToolKind::ServerWebSearch | ToolKind::ServerCodeExecution | ToolKind::ServerAdvisor
    ) {
        // Anthropic server-side tools already executed inside the provider's
        // sampling loop. Record the server_tool_use block and wait for the
        // paired server_tool_result event; do not run approval/classifier/local
        // dispatch paths.
        tracing::info!(
            target: "jfc::ui::tool",
            tool_kind = tool.kind.label(),
            tool_id = %tool.id,
            "route=server_side_tool (no local dispatch)"
        );
        emit_in_progress(tx, "add", vec![tool.id.as_str().to_owned()]);
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::tool_boxed(tool));
        }
    } else if matches!(tool.kind, ToolKind::AskUserQuestion) {
        // AskUserQuestion is neither dispatched nor approval-gated: it opens an
        // interactive modal whose selection becomes the tool_result (replacing
        // the old "post text, treat the next user message as the answer" stub
        // in dispatch.rs). At most one question is ever pending — it's a
        // turn-ending tool — so a second concurrent one, or a malformed
        // `options` array, is recorded as a failed tool_result so the tool_use
        // stays paired and the loop can continue.
        if app.pending_question.is_some() {
            let mut tool = tool;
            tool.status = ToolStatus::Failed;
            tool.output = ToolOutput::Text(
                "A question is already awaiting an answer; only one AskUserQuestion \
                 may be open at a time."
                    .to_owned(),
            );
            if let Some(msg) = streaming_assistant_mut(app) {
                msg.parts.push(MessagePart::tool_boxed(tool));
            }
        } else if let Some(pending) = crate::input::build_pending_question(&tool) {
            if let Some(msg) = streaming_assistant_mut(app) {
                msg.parts
                    .push(MessagePart::tool_boxed(Box::new((*tool).clone())));
            }
            emit_deferred_tool_use(tx, &tool, "awaiting_user_answer");
            tracing::info!(
                target: "jfc::ui::question",
                tool_id = %tool.id,
                questions = pending.items.len(),
                "route=ask_user_question (modal opened)"
            );
            app.pending_question = Some(pending);
        } else {
            let mut tool = tool;
            tool.status = ToolStatus::Failed;
            tool.output = ToolOutput::Text(
                "AskUserQuestion requires a non-empty `options` array.".to_owned(),
            );
            if let Some(msg) = streaming_assistant_mut(app) {
                msg.parts.push(MessagePart::tool_boxed(tool));
            }
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
            msg.parts.push(MessagePart::tool_boxed(tool));
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
        emit_deferred_tool_use(tx, &tool, "awaiting_classifier");
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
            msg.parts
                .push(MessagePart::tool_boxed(Box::new((*tool).clone())));
        }
        emit_deferred_tool_use(tx, &tool, "awaiting_approval");
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
            app.pending_approval = Some(PendingApproval {
                tool: *tool,
                selected: 0,
            });
        } else {
            tracing::info!(
                target: "jfc::ui::approval",
                tool_kind = kind_label,
                tool_id = %tool_id,
                queue_depth = app.approval_queue.len() + 1,
                "queued_behind_modal"
            );
            app.approval_queue.push_back(*tool);
        }
    } else {
        // Sandbox auto-approval toast (first time only per session).
        maybe_show_sandbox_toast(app);
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts
                .push(MessagePart::tool_boxed(Box::new((*tool).clone())));
        }
        // Streaming tool execution v2 (gate: streaming-tool-exec):
        // Queue every local tool in model order, then eagerly dispatch only
        // the safe prefix when no prior batch is in flight. Side-effecting
        // tools remain queued for StreamDone (or for the previous eager prefix
        // to settle) so the scheduler's sequential guarantees still hold.
        if crate::feature_gates::is_enabled(crate::feature_gates::FeatureGate::StreamingToolExec) {
            let current_tool_id = tool.id.as_str().to_owned();
            let current_tool_is_safe = crate::scheduler::is_concurrency_safe(&tool.kind);
            tracing::info!(
                target: "jfc::ui::tool",
                tool_kind = tool.kind.label(),
                tool_id = %tool.id,
                pending_total = app.pending_tool_calls.len() + 1,
                safe_for_eager = current_tool_is_safe,
                "route=eager_queue (streaming-tool-exec ON, no approval needed)"
            );
            app.pending_tool_calls.push((*tool).clone());
            let dispatched_ids = dispatch_eager_safe_prefix(app, tx);
            if !dispatched_ids.iter().any(|id| id == &current_tool_id) {
                let reason = if current_tool_is_safe {
                    "queued_for_eager_slot"
                } else {
                    "queued_for_ordered_stream_done"
                };
                emit_deferred_tool_use(tx, &tool, reason);
            }
        } else {
            tracing::info!(
                target: "jfc::ui::tool",
                tool_kind = tool.kind.label(),
                tool_id = %tool.id,
                pending_total = app.pending_tool_calls.len() + 1,
                "route=deferred_dispatch (streaming-tool-exec OFF, no approval needed)"
            );
            emit_deferred_tool_use(tx, &tool, "queued_for_stream_done");
            app.pending_tool_calls.push(*tool);
        }
    }
}

/// Handle the auto-mode classifier's verdict on a tool call.
pub(crate) async fn handle_classifier_decision(
    app: &mut App,
    tx: &EventSender,
    mut tool: Box<ToolCall>,
    blocked: bool,
    reason: String,
) {
    app.pending_classifications = app.pending_classifications.saturating_sub(1);
    if blocked {
        emit_in_progress(tx, "remove", vec![tool.id.as_str().to_owned()]);
        tool.status = ToolStatus::Failed;
        tool.output = ToolOutput::Text(format!(
            "Auto-mode classifier blocked this tool call.\n\nReason: {reason}"
        ));
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts.push(MessagePart::tool_boxed(tool));
        }
    } else {
        if let Some(msg) = streaming_assistant_mut(app) {
            msg.parts
                .push(MessagePart::tool_boxed(Box::new((*tool).clone())));
        }
        emit_deferred_tool_use(tx, &tool, "queued_for_stream_done");
        app.pending_tool_calls.push(*tool);
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
        app.in_flight_tool_batches += 1;
        emit_in_progress(
            tx,
            "add",
            calls
                .iter()
                .map(|tool| tool.id.as_str().to_owned())
                .collect(),
        );
        crate::stream::dispatch_tools_batched(
            calls,
            crate::stream::ToolBatchDispatch {
                tx: tx.clone(),
                dedup: Arc::clone(&app.dedup_cache),
                task_store: Some(Arc::clone(&app.task_store)),
                active_team_name: app.team_context.team_name.clone(),
                current_session_id: app
                    .current_session_id
                    .as_ref()
                    .map(|id| id.as_str().to_owned()),
                provider: Arc::clone(&app.provider),
                model: app.model.clone(),
                teammate_event_tx: app.teammate_event_tx.clone(),
                local_advisor: crate::stream::LocalAdvisorDispatchContext::from_app(app),
                cancel: app.cancel_token.clone(),
            },
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
    tx: &EventSender,
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
    emit_in_progress(tx, "remove", vec![tool_use_id.as_str().to_owned()]);
    if matches!(tool_kind, jfc_provider::ServerToolResultKind::Advisor) {
        tracing::info!(
            target: "jfc::advisor",
            tool_use_id = %tool_use_id,
            content_preview = %content.to_string().chars().take(200).collect::<String>(),
            "tengu_advisor_tool_result"
        );
    }
    let mut applied = false;
    if let Some(idx) = app.streaming_assistant_idx
        && let Some(msg) = app.messages.get_mut(idx)
    {
        for part in msg.parts.iter_mut() {
            if let MessagePart::Tool(tc) = part
                && tc.id == tool_use_id
            {
                // Tool was set Running on the
                // server_tool_use block; flip to
                // Completed now that the paired
                // result has arrived. mark_completed
                // is idempotent if it ever fires
                // twice from a duplicate event.
                if let Err(err) = tc.mark_completed() {
                    tracing::warn!(
                        target: "jfc::stream",
                        tool_use_id = %tc.id.as_str(),
                        from = ?err.from,
                        to = ?err.to,
                        "server_tool_result refused terminal transition — keeping prior output",
                    );
                    applied = true;
                    break;
                }
                tc.output = ToolOutput::ServerToolResult {
                    tool_kind: tool_kind.clone(),
                    content: content.clone(),
                };
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;

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

    struct StreamingToolExecGuard;

    impl StreamingToolExecGuard {
        fn enable() -> Self {
            crate::feature_gates::set(crate::feature_gates::FeatureGate::StreamingToolExec, true);
            Self
        }
    }

    impl Drop for StreamingToolExecGuard {
        fn drop(&mut self) {
            crate::feature_gates::set(crate::feature_gates::FeatureGate::StreamingToolExec, false);
        }
    }

    fn test_app() -> App {
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.task_store = jfc_session::TaskStore::in_memory();
        app
    }

    fn glob_tool(id: &str) -> ToolCall {
        ToolCall::new_pending(
            crate::ids::ToolId::from(id),
            ToolKind::Glob,
            ToolInput::Glob {
                pattern: "Cargo.toml".to_owned(),
                path: None,
            },
        )
    }

    fn bash_tool(id: &str) -> ToolCall {
        ToolCall::new_pending(
            crate::ids::ToolId::from(id),
            ToolKind::Bash,
            ToolInput::Bash {
                command: "echo hi".to_owned(),
                timeout: None,
                workdir: None,
                run_in_background: None,
            },
        )
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial]
    async fn eager_prefix_does_not_dispatch_unsafe_front_regression() {
        let _guard = StreamingToolExecGuard::enable();
        let mut app = test_app();
        app.pending_tool_calls = vec![bash_tool("b1"), glob_tool("g1")];
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let dispatched = dispatch_eager_safe_prefix(&mut app, &tx);

        assert!(dispatched.is_empty());
        assert_eq!(app.pending_tool_calls.len(), 2);
        assert_eq!(app.in_flight_eager_dispatches, 0);
        assert_eq!(app.in_flight_tool_batches, 0);
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial]
    async fn eager_prefix_drains_only_safe_prefix_before_unsafe_regression() {
        let _guard = StreamingToolExecGuard::enable();
        let mut app = test_app();
        app.pending_tool_calls = vec![glob_tool("g1"), glob_tool("g2"), bash_tool("b1")];
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let dispatched = dispatch_eager_safe_prefix(&mut app, &tx);

        assert_eq!(dispatched, vec!["g1".to_owned(), "g2".to_owned()]);
        assert_eq!(app.pending_tool_calls.len(), 1);
        assert_eq!(app.pending_tool_calls[0].id.as_str(), "b1");
        assert_eq!(app.in_flight_eager_dispatches, 1);
        assert_eq!(app.in_flight_tool_batches, 1);
        assert!(app.pre_dispatched_tool_ids.contains("g1"));
        assert!(app.pre_dispatched_tool_ids.contains("g2"));
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial]
    async fn eager_prefix_waits_for_auto_mode_classifier_regression() {
        let _guard = StreamingToolExecGuard::enable();
        let mut app = test_app();
        app.pending_classifications = 1;
        app.pending_tool_calls = vec![glob_tool("g1")];
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let dispatched = dispatch_eager_safe_prefix(&mut app, &tx);

        assert!(dispatched.is_empty());
        assert_eq!(app.pending_tool_calls.len(), 1);
        assert_eq!(app.pending_tool_calls[0].id.as_str(), "g1");
        assert_eq!(app.in_flight_eager_dispatches, 0);
        assert_eq!(app.in_flight_tool_batches, 0);
        assert!(app.pre_dispatched_tool_ids.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial]
    async fn classifier_decision_with_streaming_tool_exec_queues_while_streaming_regression() {
        let _guard = StreamingToolExecGuard::enable();
        let mut app = test_app();
        app.auto_mode.enabled = true;
        app.is_streaming = true;
        app.pending_classifications = 1;
        app.messages.push(ChatMessage::assistant_parts(Vec::new()));
        app.streaming_assistant_idx = Some(0);
        let (tx, mut rx) = tokio::sync::mpsc::channel(8);

        handle_classifier_decision(
            &mut app,
            &tx,
            Box::new(glob_tool("g1")),
            false,
            "allowed".to_owned(),
        )
        .await;

        assert_eq!(app.pending_classifications, 0);
        assert_eq!(app.pending_tool_calls.len(), 1);
        assert_eq!(app.pending_tool_calls[0].id.as_str(), "g1");
        assert_eq!(app.in_flight_eager_dispatches, 0);
        assert_eq!(app.in_flight_tool_batches, 0);
        assert!(app.pre_dispatched_tool_ids.is_empty());

        let event = rx
            .try_recv()
            .expect("classifier should emit deferred tool use");
        assert!(matches!(
            event,
            AppEvent::Tool(ToolEvent::DeferredToolUse { reason, .. })
                if reason == "queued_for_stream_done"
        ));
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test(flavor = "current_thread")]
    #[serial_test::serial]
    async fn classifier_decision_after_stream_dispatches_ordered_batch_not_eager_regression() {
        let _guard = StreamingToolExecGuard::enable();
        let mut app = test_app();
        app.auto_mode.enabled = true;
        app.is_streaming = false;
        app.pending_classifications = 1;
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        handle_classifier_decision(
            &mut app,
            &tx,
            Box::new(glob_tool("g1")),
            false,
            "allowed".to_owned(),
        )
        .await;

        assert_eq!(app.pending_classifications, 0);
        assert!(app.pending_tool_calls.is_empty());
        assert_eq!(app.in_flight_eager_dispatches, 0);
        assert_eq!(app.in_flight_tool_batches, 1);
        assert!(app.pre_dispatched_tool_ids.is_empty());
    }
}
