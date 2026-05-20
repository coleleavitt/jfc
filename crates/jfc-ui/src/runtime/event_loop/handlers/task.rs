//! `TaskEvent::*` handlers — subagent streaming, background task
//! registration, progress, completion, and failure.

use crate::app::{self, App};
use crate::runtime::{EventSender, factory_mode_enabled, maybe_continue_task_factory};
use crate::types::*;
use crate::{stream, types};

use super::super::guards::streaming_assistant_mut;

/// Handle `TaskEvent::AgentChunk { task_id, text }`.
pub(crate) fn handle_agent_chunk(app: &mut App, task_id: crate::ids::TaskId, text: String) {
    // Subagent emitted a streaming text chunk — append to its
    // task's message log so the task view shows live output
    // rather than the "No messages yet" empty state. v126
    // pipes nested-stream chunks the same way so the user
    // can drill into a running agent and see what it's doing.
    app.last_active_agent_task = Some(task_id.as_str().to_owned());
    crate::daemon::record_background_agent_log(task_id.as_str(), &text);
    if let Some(bt) = app.background_tasks.get_mut(task_id.as_str()) {
        // Coalesce with the previous chunk when both came in
        // rapid succession AND the previous entry doesn't end
        // with a newline — so a single conceptual paragraph
        // streamed across many chunks renders as one paragraph
        // instead of one entry per delta.
        let coalesce = bt
            .messages
            .last()
            .map(|s| !s.ends_with('\n') && !s.starts_with('['))
            .unwrap_or(false);
        if coalesce {
            if let Some(last) = bt.messages.last_mut() {
                last.push_str(&text);
            }
            // Also coalesce into the structured chat_messages.
            let chat_coalesce = bt
                .chat_messages
                .last()
                .map(|m| m.role == types::Role::Assistant)
                .unwrap_or(false);
            if chat_coalesce {
                if let Some(msg) = bt.chat_messages.last_mut() {
                    if let Some(types::MessagePart::Text(t)) = msg.parts.last_mut() {
                        t.push_str(&text);
                    } else {
                        msg.parts.push(types::MessagePart::Text(text));
                    }
                }
            } else {
                bt.chat_messages.push(types::ChatMessage::assistant(text));
            }
        } else {
            bt.messages.push(text.clone());
            // Start a new assistant message in the structured log.
            bt.chat_messages.push(types::ChatMessage::assistant(text));
        }
    }
}

/// Handle `TaskEvent::Started { ... }`.
pub(crate) fn handle_task_started(
    app: &mut App,
    task_id: crate::ids::TaskId,
    description: String,
    model_used: Option<String>,
    max_input_tokens: Option<u64>,
    is_detached: bool,
    parent_task_id: Option<String>,
) {
    tracing::info!(
        target: "jfc::task",
        %task_id, %description, ?model_used, is_detached,
        ?parent_task_id,
        "TaskStarted"
    );
    use types::{TaskLifecycle, TaskStatusPart};
    // If this delegation is linked to a queued todo, flip
    // that todo to InProgress now so the task panel reflects
    // that an agent has picked it up.
    if let Some(ref ptid) = parent_task_id {
        let linked_model = model_used
            .clone()
            .or_else(|| Some(app.model.as_str().to_owned()));
        if let Err(e) = app.task_store.update(
            ptid,
            jfc_session::TaskPatch {
                status: Some(jfc_session::TaskStatus::InProgress),
                metadata: Some(serde_json::json!({
                    "agent_task_id": task_id.as_str(),
                    "model": linked_model,
                })),
                ..Default::default()
            },
        ) {
            tracing::warn!(
                target: "jfc::task",
                parent_task_id = %ptid,
                error = %e,
                "TaskStarted: failed to mark linked task in_progress"
            );
        }
    }
    app.background_tasks.insert(
        task_id.as_str().to_owned(),
        app::BackgroundTask {
            task_id: task_id.clone(),
            description: description.clone(),
            status: TaskLifecycle::Running,
            started_at: std::time::Instant::now(),
            summary: None,
            error: None,
            last_tool: None,
            messages: Vec::new(),
            chat_messages: Vec::new(),
            tool_use_count: 0,
            latest_input_tokens: 0,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: 0,
            model_used: model_used
                .clone()
                .or_else(|| Some(app.model.as_str().to_owned())),
            agent_messages: Vec::new(),
            max_input_tokens,
            budget_killed: false,
            parent_task_id: parent_task_id.clone(),
        },
    );
    // Only register detached workers into the daemon
    // roster. For detached agents the worker process
    // already wrote pid + launch_path via
    // `record_background_agent_started_at`; we still call
    // the registry here so the UI-side launch metadata
    // (description / model) refreshes, but the PID-write
    // contract in `record_background_agent_started_at`
    // prevents the UI's own PID from clobbering the
    // worker's. Foreground teammates / in-process
    // subagents are tracked exclusively via
    // `app.background_tasks` — registering them in the
    // daemon would make the reconciler mark them stale
    // when the UI exits (the user-visible "Done" /
    // "Failed" labels in the screenshots).
    let model_for_part = model_used
        .clone()
        .or_else(|| Some(app.model.as_str().to_owned()));
    if is_detached {
        crate::daemon::record_background_agent_started(
            task_id.as_str(),
            &description,
            model_used.or_else(|| Some(app.model.as_str().to_owned())),
            None,
        );
    }
    let part = MessagePart::TaskStatus(TaskStatusPart {
        task_id,
        description,
        status: TaskLifecycle::Running,
        summary: None,
        error: None,
        elapsed_ms: None,
        model: model_for_part,
    });
    if let Some(msg) = streaming_assistant_mut(app) {
        msg.parts.push(part);
    } else if let Some(msg) = app.messages.last_mut() {
        msg.parts.push(part);
    }
}

/// Handle `TaskEvent::Progress { ... }`.
pub(crate) fn handle_task_progress(
    app: &mut App,
    task_id: crate::ids::TaskId,
    last_tool: Option<String>,
    elapsed_ms: u64,
    tool_use_count: Option<u32>,
    input_tokens: Option<u64>,
    cache_read_tokens: Option<u64>,
    cache_write_tokens: Option<u64>,
    output_tokens: Option<u64>,
) {
    let mut usage_update: Option<(String, u32, u32, u32, u32)> = None;
    if let Some(bt) = app.background_tasks.get_mut(task_id.as_str()) {
        if let Some(ref tool) = last_tool {
            let elapsed_s = elapsed_ms / 1000;
            let entry = format!("[{elapsed_s}s] {tool}");
            bt.messages.push(entry.clone());
            // Push a muted user-role note into the structured log
            // so the MessageView renderer can show tool activity
            // inline with the assistant's text output.
            bt.chat_messages.push(types::ChatMessage::user(entry));
        }
        bt.last_tool = last_tool.clone();
        if let Some(n) = tool_use_count {
            bt.tool_use_count = n;
        }
        if let Some(n) = input_tokens {
            bt.latest_input_tokens = n;
        }
        if let Some(n) = cache_read_tokens {
            bt.latest_cache_read_tokens = n;
        }
        if let Some(n) = cache_write_tokens {
            bt.latest_cache_write_tokens = n;
        }
        if let Some(n) = output_tokens {
            // Cumulative — sum across every round-trip,
            // matching v131's `cumulativeOutputTokens` field.
            bt.cumulative_output_tokens = bt.cumulative_output_tokens.saturating_add(n);
        }
        if let Some(model) = bt.model_used.clone() {
            let input = input_tokens.unwrap_or_default();
            let output = output_tokens.unwrap_or_default();
            let cache_read = cache_read_tokens.unwrap_or_default();
            let cache_write = cache_write_tokens.unwrap_or_default();
            if input > 0 || output > 0 || cache_read > 0 || cache_write > 0 {
                usage_update = Some((
                    model,
                    input.min(u32::MAX as u64) as u32,
                    output.min(u32::MAX as u64) as u32,
                    cache_read.min(u32::MAX as u64) as u32,
                    cache_write.min(u32::MAX as u64) as u32,
                ));
            }
        }
    }
    crate::daemon::record_background_agent_progress(
        task_id.as_str(),
        last_tool.as_deref(),
        tool_use_count,
        input_tokens,
        cache_read_tokens,
        cache_write_tokens,
        output_tokens,
    );
    if let Some((model, input, output, cache_read, cache_write)) = usage_update {
        app.usage_by_model.entry(model).or_default().add_delta(
            input,
            output,
            cache_read,
            cache_write,
        );
    }
    for msg in &mut app.messages {
        for part in &mut msg.parts {
            if let MessagePart::TaskStatus(ts) = part
                && ts.task_id == task_id
            {
                ts.elapsed_ms = Some(elapsed_ms);
            }
        }
    }
}

/// Handle `TaskEvent::Completed { task_id, summary, elapsed_ms }`.
pub(crate) fn handle_task_completed(
    app: &mut App,
    task_id: crate::ids::TaskId,
    summary: String,
    elapsed_ms: u64,
) {
    tracing::info!(
        target: "jfc::task",
        %task_id, elapsed_ms,
        summary_len = summary.len(),
        "TaskCompleted"
    );
    use types::TaskLifecycle;
    let mut linked_task_id: Option<String> = None;
    if let Some(bt) = app.background_tasks.get_mut(task_id.as_str()) {
        bt.status = TaskLifecycle::Completed;
        bt.summary = Some(summary.clone());
        let elapsed_s = elapsed_ms / 1000;
        let entry = format!("[{elapsed_s}s] ✓ done — {summary}");
        bt.messages.push(entry.clone());
        bt.chat_messages.push(types::ChatMessage::assistant(entry));
        linked_task_id = bt.parent_task_id.clone();
    }
    // If the model linked this delegation to a queued todo
    // via `parent_task_id`, mark that todo Completed in the
    // TaskStore. Without this, a foreground subagent could
    // finish cleanly while its queued task stayed
    // `in_progress` — the Task tool result and the
    // persistent todo were never connected.
    if let Some(ref ptid) = linked_task_id
        && let Err(e) = app.task_store.update(
            ptid,
            jfc_session::TaskPatch {
                status: Some(jfc_session::TaskStatus::Completed),
                ..Default::default()
            },
        )
    {
        tracing::warn!(
            target: "jfc::task",
            parent_task_id = %ptid,
            error = %e,
            "TaskCompleted: failed to mark linked task completed"
        );
    }
    crate::daemon::record_background_agent_finished(
        task_id.as_str(),
        crate::daemon::BackgroundAgentStatus::Completed,
        &summary,
    );
    for msg in &mut app.messages {
        for part in &mut msg.parts {
            if let MessagePart::TaskStatus(ts) = part
                && ts.task_id == task_id
            {
                ts.status = TaskLifecycle::Completed;
                ts.summary = Some(summary.clone());
                ts.elapsed_ms = Some(elapsed_ms);
            }
        }
    }
}

/// Handle `TaskEvent::Failed { task_id, error }`.
pub(crate) async fn handle_task_failed(
    app: &mut App,
    tx: &EventSender,
    task_id: crate::ids::TaskId,
    error: String,
) {
    tracing::warn!(
        target: "jfc::task",
        %task_id,
        error_preview = %&error[..error.len().min(200)],
        "TaskFailed"
    );
    use types::TaskLifecycle;
    let was_cancelled = error
        .trim_start()
        .to_ascii_lowercase()
        .starts_with("cancelled:");
    let mut linked_task_id: Option<String> = None;
    if let Some(bt) = app.background_tasks.get_mut(task_id.as_str()) {
        bt.status = if was_cancelled {
            TaskLifecycle::Cancelled
        } else {
            TaskLifecycle::Failed
        };
        bt.error = Some(error.clone());
        let prefix = if was_cancelled { "cancelled" } else { "failed" };
        let entry = format!("[{prefix}] {error}");
        bt.messages.push(entry.clone());
        bt.chat_messages.push(types::ChatMessage::assistant(entry));
        linked_task_id = bt.parent_task_id.clone();
    }
    // Propagate the failure to the linked queued todo. A
    // cancelled agent leaves the task Pending (so the queue
    // can retry it); a genuine failure marks it Failed so
    // the cascade / replan logic below can react.
    if let Some(ref ptid) = linked_task_id {
        let next_status = if was_cancelled {
            jfc_session::TaskStatus::Pending
        } else {
            jfc_session::TaskStatus::Failed
        };
        if let Err(e) = app.task_store.update(
            ptid,
            jfc_session::TaskPatch {
                status: Some(next_status),
                ..Default::default()
            },
        ) {
            tracing::warn!(
                target: "jfc::task",
                parent_task_id = %ptid,
                error = %e,
                "TaskFailed: failed to update linked task status"
            );
        }
    }
    crate::daemon::record_background_agent_finished(
        task_id.as_str(),
        if was_cancelled {
            crate::daemon::BackgroundAgentStatus::Cancelled
        } else {
            crate::daemon::BackgroundAgentStatus::Failed
        },
        &error,
    );
    for msg in &mut app.messages {
        for part in &mut msg.parts {
            if let MessagePart::TaskStatus(ts) = part
                && ts.task_id == task_id
            {
                ts.status = if was_cancelled {
                    TaskLifecycle::Cancelled
                } else {
                    TaskLifecycle::Failed
                };
                ts.error = Some(error.clone());
            }
        }
    }

    // Adaptive re-planning: cascade failure to dependent tasks
    // and inject a system_reminder to prompt the model to re-plan.
    if !was_cancelled && factory_mode_enabled() {
        let cascaded_ids = app.task_store.cascade_failure(task_id.as_str());
        let subject = app
            .task_store
            .get(task_id.as_str())
            .map(|t| t.subject.clone())
            .unwrap_or_default();
        let cascaded_str = if cascaded_ids.is_empty() {
            "none".to_string()
        } else {
            cascaded_ids
                .iter()
                .map(|id| id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        };
        let reminder = format!(
            "Task {task_id} ({subject}) failed: {error}. Dependent tasks [{cascaded_str}] have been cancelled. \
             Review the failure and either:\n\
             1. Fix the issue and re-create the failed task with TaskCreate\n\
             2. Revise the plan by creating replacement tasks\n\
             3. Mark the remaining work as not needed via TaskUpdate(status=deleted)"
        );
        // Auto-create a replan task so the factory can pick it up
        if let Some(replan) = app.task_store.create_replan_task(task_id.as_str()) {
            tracing::info!(
                target: "jfc::tasks::factory",
                failed_id = %task_id,
                replan_id = %replan.id,
                "auto-created replan task for failed task"
            );
        }
        crate::system_reminder::append_to_last_user(&mut app.messages, &reminder);
        maybe_continue_task_factory(app, tx).await;
    }
    // After a background task reaches terminal state, check
    // if ALL background tasks are now done. If so AND the
    // main turn is waiting (turn_started_at is set, no tools
    // pending, should_continue_loop), trigger the agentic
    // continuation. This fixes the "last task stays green"
    // bug where all agents complete but the leader never
    // resumes because no AllToolsComplete fires after the
    // last TaskCompleted event.
    let all_bg_done = app
        .background_tasks
        .values()
        .all(|bt| bt.status.is_terminal());
    if all_bg_done
        && app.turn_started_at.is_some()
        && app.pending_tool_calls.is_empty()
        && app.pending_approval.is_none()
        && app.approval_queue.is_empty()
        && !app.is_streaming
        && stream::should_continue_loop(&app.messages)
    {
        tracing::info!(
            target: "jfc::task",
            "all background tasks terminal — triggering agentic continuation"
        );
        stream::continue_agentic_loop(app, tx).await;
    } else if all_bg_done
        && app.turn_started_at.is_some()
        && app.pending_tool_calls.is_empty()
        && !app.is_streaming
        && !stream::should_continue_loop(&app.messages)
    {
        // All done and model already emitted EndTurn — just
        // clear the turn timer so the spinner stops.
        tracing::debug!(
            target: "jfc::task",
            "all background tasks terminal, turn complete — clearing turn_started_at"
        );
        app.turn_started_at = None;
    }
}
