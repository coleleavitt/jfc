//! `TaskEvent::*` handlers — subagent streaming, background task
//! registration, progress, completion, and failure.

use crate::app::{self, EngineState};
use crate::runtime::{EventSender, factory_mode_enabled, maybe_continue_task_factory};
use crate::types::*;
use crate::{stream, types};

use super::super::guards::streaming_assistant_mut;

/// Handle `TaskEvent::AgentChunk { task_id, text }`.
pub fn handle_agent_chunk(state: &mut EngineState, task_id: crate::ids::TaskId, text: String) {
    // Subagent emitted a streaming text chunk — append to its
    // task's message log so the task view shows live output
    // rather than the "No messages yet" empty state. v126
    // pipes nested-stream chunks the same way so the user
    // can drill into a running agent and see what it's doing.
    state.last_active_agent_task = Some(task_id.as_str().to_owned());
    crate::daemon::record_background_agent_log(task_id.as_str(), &text);
    if let Some(bt) = state.background_tasks.get_mut(task_id.as_str()) {
        bt.append_chunk(text);
    }
}

/// Handle `TaskEvent::Started { ... }`.
pub fn handle_task_started(
    state: &mut EngineState,
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
            .or_else(|| Some(state.model.as_str().to_owned()));
        if let Err(e) = state.task_store.update(
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
    state.background_tasks.insert(
        task_id.as_str().to_owned(),
        app::BackgroundTask {
            task_id: task_id.clone(),
            description: description.clone(),
            status: TaskLifecycle::Running,
            started_at: std::time::Instant::now(),
            completed_at: None,
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
                .or_else(|| Some(state.model.as_str().to_owned())),
            agent_messages: Vec::new(),
            max_input_tokens,
            budget_killed: false,
            parent_task_id,
            workflow_progress: None,
            last_activity_at: std::time::Instant::now(),
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
    // `state.background_tasks` — registering them in the
    // daemon would make the reconciler mark them stale
    // when the UI exits (the user-visible "Done" /
    // "Failed" labels in the screenshots).
    let model_for_part = model_used
        .clone()
        .or_else(|| Some(state.model.as_str().to_owned()));
    if is_detached {
        crate::daemon::record_background_agent_started(
            task_id.as_str(),
            &description,
            model_used.or_else(|| Some(state.model.as_str().to_owned())),
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
    if let Some(msg) = streaming_assistant_mut(state) {
        msg.parts.push(part);
    } else if let Some(msg) = state.messages.last_mut() {
        msg.parts.push(part);
    }
}

/// Handle `TaskEvent::Progress { ... }`.
pub fn handle_task_progress(
    state: &mut EngineState,
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
    if let Some(bt) = state.background_tasks.get_mut(task_id.as_str()) {
        if let Some(ref tool) = last_tool {
            let elapsed_s = elapsed_ms / 1000;
            let entry = format!("[{elapsed_s}s] {tool}");
            bt.push_log(entry.clone());
            // Push a muted user-role note into the structured log
            // so the MessageView renderer can show tool activity
            // inline with the assistant's text output.
            bt.push_chat(types::ChatMessage::user(entry));
        }
        bt.last_tool = last_tool.clone();
        bt.last_activity_at = std::time::Instant::now();
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
        state.usage_by_model.entry(model).or_default().add_delta(
            input,
            output,
            cache_read,
            cache_write,
        );
    }
    for msg in &mut state.messages {
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
pub async fn handle_task_completed(
    state: &mut EngineState,
    tx: &EventSender,
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
    if let Some(bt) = state.background_tasks.get_mut(task_id.as_str()) {
        // A real terminal transition observed in this process — unblocks the
        // Case-2 auto-wake (restored prior-session agents never reach here).
        state.observed_bg_terminal_transition_this_process = true;
        bt.status = TaskLifecycle::Completed;
        bt.completed_at = Some(std::time::Instant::now());
        bt.summary = Some(summary.clone());
        let elapsed_s = elapsed_ms / 1000;
        let entry = format!("[{elapsed_s}s] ✓ done — {summary}");
        bt.push_log(entry.clone());
        bt.push_chat(types::ChatMessage::assistant(entry));
        linked_task_id = bt.parent_task_id.clone();
    }
    // If the model linked this delegation to a queued todo
    // via `parent_task_id`, mark that todo Completed in the
    // TaskStore. Without this, a foreground subagent could
    // finish cleanly while its queued task stayed
    // `in_progress` — the Task tool result and the
    // persistent todo were never connected.
    if let Some(ref ptid) = linked_task_id {
        if let Err(e) = state.task_store.update(
            ptid,
            jfc_session::TaskPatch {
                status: Some(jfc_session::TaskStatus::Completed),
                ..Default::default()
            },
        ) {
            tracing::warn!(
                target: "jfc::task",
                parent_task_id = %ptid,
                error = %e,
                "TaskCompleted: failed to mark linked task completed"
            );
        } else {
            // Parity with the manual TaskDone path (tools/dispatch.rs): a
            // subagent finishing its parent_task_id-linked todo must also
            // advance any plan that linked the task, otherwise plans stall
            // whenever work is delegated instead of done inline.
            crate::tools::advance_linked_plans(&state.task_store, ptid);
        }
    }
    crate::daemon::record_background_agent_finished(
        task_id.as_str(),
        crate::daemon::BackgroundAgentStatus::Completed,
        &summary,
    );
    for msg in &mut state.messages {
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
    // Resume the leader when this was the last in-flight background agent —
    // a successful completion must re-engage the loop just like a failure does.
    maybe_resume_after_background(state, tx).await;
}

/// Handle `TaskEvent::Failed { task_id, error }`.
pub async fn handle_task_failed(
    state: &mut EngineState,
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
    if let Some(bt) = state.background_tasks.get_mut(task_id.as_str()) {
        state.observed_bg_terminal_transition_this_process = true;
        bt.status = if was_cancelled {
            TaskLifecycle::Cancelled
        } else {
            TaskLifecycle::Failed
        };
        bt.completed_at = Some(std::time::Instant::now());
        bt.error = Some(error.clone());
        let prefix = if was_cancelled { "cancelled" } else { "failed" };
        let entry = format!("[{prefix}] {error}");
        bt.push_log(entry.clone());
        bt.push_chat(types::ChatMessage::assistant(entry));
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
        if let Err(e) = state.task_store.update(
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
    for msg in &mut state.messages {
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

    // Proactive failure recovery (Agentic Task Graph, arXiv:2605.11951):
    // classify the failure, retry transient ones under budget, and on hard
    // failure reroute recoverable dependents onto a replan task instead of
    // destroying the subtree. Replaces the old destructive cascade.
    if !was_cancelled && factory_mode_enabled() {
        // Use the linked task id (the queued todo); fall back to the agent id.
        let recover_id = linked_task_id.as_deref().unwrap_or(task_id.as_str());
        let subject = state
            .task_store
            .get(recover_id)
            .map(|t| t.subject)
            .unwrap_or_default();

        let recovery = state.task_store.recover_from_failure(recover_id, &error);
        let reminder = match &recovery {
            jfc_session::FailureRecovery::Retried {
                task_id: rid,
                attempt,
                max_attempts,
            } => format!(
                "Task {rid} ({subject}) failed with a transient error (attempt {attempt}/{max_attempts}): {error}\n\
                 It has been re-queued for another attempt. Continue and retry it — the failure \
                 looked recoverable (timeout/network/lock class)."
            ),
            jfc_session::FailureRecovery::Replanned {
                failed_id,
                replan_id,
                rerouted,
                attempts,
            } => {
                let rerouted_str = if rerouted.is_empty() {
                    "none".to_string()
                } else {
                    rerouted
                        .iter()
                        .map(|id| id.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                };
                format!(
                    "Task {failed_id} ({subject}) hard-failed after {attempts} attempt(s): {error}\n\
                     Created replan task {replan_id}; dependent task(s) [{rerouted_str}] are now \
                     blocked on it (preserved, NOT cancelled) and will unblock when it completes.\n\
                     Work the replan task: diagnose the root cause, then either fix + re-create the \
                     failed task or revise its subtasks. The rerouted dependents resume automatically."
                )
            }
            jfc_session::FailureRecovery::Unknown => format!(
                "Task failed: {error}. (Task {recover_id} not found in the store for recovery.)"
            ),
        };
        crate::system_reminder::append_to_last_user(&mut state.messages, &reminder);
        maybe_continue_task_factory(state, tx).await;
    }
    maybe_resume_after_background(state, tx).await;
}

/// After a background agent reaches a terminal state, resume the leader if
/// every background task is now done and the main turn is idle and waiting.
///
/// This previously lived inline in the *failure* handler only, so a final
/// agent that finished **successfully** left the leader parked until the next
/// manual prompt (no `AllToolsComplete` fires after the last `TaskCompleted`).
/// Sharing it across both terminal paths fixes the "last task stays green,
/// leader never resumes" bug.
pub async fn maybe_resume_after_background(state: &mut EngineState, tx: &EventSender) {
    let all_bg_done = state
        .background_tasks
        .values()
        .all(|bt| bt.status.is_terminal());
    if !all_bg_done {
        return;
    }

    // Case 1: The leader is still inside a turn — apply the existing
    // continuation policy. Respect compaction-in-flight and queued user
    // prompts the same way the post-tool AllComplete path does.
    if state.turn_started_at.is_some() {
        if state.pending_tool_calls.is_empty()
            && state.pending_approval.is_none()
            && state.approval_queue.is_empty()
            && !state.is_streaming
            && state.compacting_started_at.is_none()
            && stream::should_continue_loop(&state.messages)
        {
            if state.queued_prompts.iter().any(|queued| !queued.is_meta) {
                tracing::info!(
                    target: "jfc::task",
                    queued = state.queued_prompts.len(),
                    "all background tasks terminal — yielding to queued user prompt"
                );
                crate::runtime::drain_queued_prompts(state, tx).await;
            } else {
                tracing::info!(
                    target: "jfc::task",
                    "all background tasks terminal — triggering agentic continuation"
                );
                stream::continue_agentic_loop(state, tx).await;
            }
        } else if state.pending_tool_calls.is_empty()
            && !state.is_streaming
            && !stream::should_continue_loop(&state.messages)
        {
            // All done and the model already emitted EndTurn — just clear the
            // turn timer so the spinner stops.
            tracing::debug!(
                target: "jfc::task",
                "all background tasks terminal, turn complete — clearing turn_started_at"
            );
            state.turn_started_at = None;
        }
        return;
    }

    // Case 2: Auto-wake the idle leader.
    //
    // The spawning turn finished long ago (the Task tool returned its
    // "Spawned" result almost immediately, so `turn_started_at` was cleared).
    // Now that every background subagent has reached a terminal state, inject
    // a system-reminder digest of their results and open a fresh turn so the
    // main agent automatically summarizes the work for the user — no manual
    // nudge required.
    //
    // GUARD: only auto-wake when a background agent actually transitioned to
    // terminal *during this process*. On `jfc --continue`,
    // `restore_persistent_background_agents` seeds already-terminal agents
    // from a prior session — `all_bg_done` is trivially true and
    // `turn_started_at` is None, which previously fired an unsolicited
    // (billed) summary turn at startup before the user typed anything. The
    // restored agents never hit a live transition site, so this flag stays
    // false until a real completion happens this process.
    if !state.observed_bg_terminal_transition_this_process {
        tracing::debug!(
            target: "jfc::task::autowake",
            bg_count = state.background_tasks.len(),
            "skipping auto-wake: no background-agent terminal transition observed \
             this process (likely restored-from-continue agents)"
        );
        return;
    }

    // Skip if we have nothing to report (e.g. all bg slots were cancelled
    // before producing a summary), if a stream is already active, or if a
    // compaction is in flight. Any pending approval / pending tool also
    // means the leader is mid-flight and should not be force-woken.
    if state.is_streaming
        || state.compacting_started_at.is_some()
        || state.pending_approval.is_some()
        || !state.approval_queue.is_empty()
        || !state.pending_tool_calls.is_empty()
    {
        return;
    }

    let mut completed_summaries: Vec<String> = Vec::new();
    for bt in state.background_tasks.values() {
        if let Some(ref summary) = bt.summary {
            completed_summaries.push(format!("- {}: {}", bt.description, summary));
        } else if let Some(ref err) = bt.error {
            completed_summaries.push(format!("- {} (failed): {}", bt.description, err));
        }
    }

    if completed_summaries.is_empty() {
        return;
    }

    // If a queued user prompt is sitting in the buffer, prefer draining it —
    // the user's words are higher priority than an auto-summary turn.
    if state.queued_prompts.iter().any(|queued| !queued.is_meta) {
        tracing::info!(
            target: "jfc::task::autowake",
            queued = state.queued_prompts.len(),
            "all background tasks complete — yielding to queued user prompt instead of autowake"
        );
        crate::runtime::drain_queued_prompts(state, tx).await;
        return;
    }

    tracing::info!(
        target: "jfc::task::autowake",
        count = completed_summaries.len(),
        "all background tasks complete — autowaking idle leader to summarize results"
    );

    let reminder = format!(
        "All background subagents have finished their work. Here is the summary of results:\n\n\
         {}\n\n\
         Review these results and write a final, concise summary for the user. \
         If any task failed, explain what went wrong and recommend next steps.",
        completed_summaries.join("\n")
    );

    crate::system_reminder::append_to_last_user(&mut state.messages, &reminder);
    state.take_background_agent_completions();
    state.agentic_turn_count = 0;
    state.turn_started_at = Some(std::time::Instant::now());
    // Consume the transition signal: this auto-wake has now reported every
    // currently-terminal agent. Without clearing it, a Tick landing in the
    // window between here and the stream actually starting could re-enter and
    // fire a *second* digest for the same completions. The flag re-arms the
    // moment a fresh agent reaches terminal (the three live transition sites),
    // so genuinely-later completions still wake the leader.
    state.observed_bg_terminal_transition_this_process = false;
    stream::continue_agentic_loop(state, tx).await;
}

#[cfg(test)]
mod autowake_tests {
    use crate::app::EngineState;
    use std::sync::Arc;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use tokio::sync::mpsc;

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

    fn test_app() -> EngineState {
        EngineState::new(Arc::new(TestProvider), "test-model")
    }

    fn terminal_bg(id: &str, summary: &str) -> crate::app::BackgroundTask {
        crate::app::BackgroundTask {
            task_id: id.into(),
            description: format!("desc-{id}"),
            status: TaskLifecycle::Completed,
            started_at: std::time::Instant::now(),
            completed_at: Some(std::time::Instant::now()),
            summary: Some(summary.to_owned()),
            error: None,
            last_tool: None,
            messages: Vec::new(),
            chat_messages: Vec::new(),
            tool_use_count: 0,
            latest_input_tokens: 0,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: 0,
            model_used: None,
            agent_messages: Vec::new(),
            max_input_tokens: None,
            budget_killed: false,
            parent_task_id: None,
            workflow_progress: None,
            last_activity_at: std::time::Instant::now(),
        }
    }

    // Normal — REGRESSION (the `--continue` startup auto-wake bug): a session
    // restored with already-terminal background agents (and an idle leader,
    // turn_started_at=None) must NOT auto-wake a summary turn. The restored
    // agents never hit a live transition site, so
    // observed_bg_terminal_transition_this_process stays false.
    #[tokio::test]
    async fn continue_with_restored_terminal_agents_does_not_autowake_regression() {
        let mut state = test_app();
        state
            .background_tasks
            .insert("a".into(), terminal_bg("a", "did a thing"));
        state
            .background_tasks
            .insert("b".into(), terminal_bg("b", "did another"));
        // Mirror the `--continue` startup state: idle leader, no live
        // transition observed, restored agents already terminal.
        state.turn_started_at = None;
        state.observed_bg_terminal_transition_this_process = false;
        let msgs_before = state.messages.len();
        let (tx, _rx) = mpsc::channel(8);

        maybe_resume_after_background(&mut state, &tx).await;

        // No synthetic system-reminder turn was opened.
        assert_eq!(
            state.messages.len(),
            msgs_before,
            "restored terminal agents must not trigger an unsolicited summary turn"
        );
        assert!(
            state.turn_started_at.is_none(),
            "no turn should have been started"
        );
    }

    // Normal: once a real terminal transition is observed this process
    // (flag true), the idle-leader auto-wake fires and opens a summary turn.
    #[tokio::test]
    async fn live_terminal_transition_does_autowake_normal() {
        let mut state = test_app();
        state
            .background_tasks
            .insert("a".into(), terminal_bg("a", "did a thing"));
        state.turn_started_at = None;
        state.observed_bg_terminal_transition_this_process = true;
        let (tx, _rx) = mpsc::channel(8);

        maybe_resume_after_background(&mut state, &tx).await;

        // Auto-wake injected the summary reminder and opened a turn.
        assert!(
            state.turn_started_at.is_some(),
            "a real completion this process must auto-wake the leader"
        );
    }

    // Robust — REGRESSION (double-fire window): after auto-wake fires, the
    // transition flag must be cleared so a Tick re-entering before the stream
    // starts cannot fire a *second* digest for the same completions. Simulate
    // the re-entrant Tick by resetting turn_started_at (as if the turn already
    // settled) and calling again — with the flag now false, no new turn opens.
    #[tokio::test]
    async fn autowake_clears_flag_and_second_call_noops_robust() {
        let mut state = test_app();
        state
            .background_tasks
            .insert("a".into(), terminal_bg("a", "did a thing"));
        state.turn_started_at = None;
        state.observed_bg_terminal_transition_this_process = true;
        let (tx, _rx) = mpsc::channel(8);

        // First call: auto-wake fires and consumes the flag.
        maybe_resume_after_background(&mut state, &tx).await;
        assert!(
            !state.observed_bg_terminal_transition_this_process,
            "auto-wake must consume the transition flag"
        );

        // Re-entrant Tick: pretend the turn already settled, call again. The
        // flag is false, so Case 2 short-circuits — no second summary turn.
        state.turn_started_at = None;
        let msgs_before = state.messages.len();
        maybe_resume_after_background(&mut state, &tx).await;
        assert_eq!(
            state.messages.len(),
            msgs_before,
            "a second resume call with the flag cleared must not open another turn"
        );
        assert!(
            state.turn_started_at.is_none(),
            "no second turn should start"
        );
    }
}
