use crate::{
    app::{BackgroundAgentCompletion, BackgroundTask},
    daemon::{BackgroundAgentInfo, BackgroundAgentStatus, DaemonPaths},
    ids::TaskId,
    types::{MessagePart, TaskLifecycle},
};

use super::agent_log_parser::parse_agent_log_to_chat_messages;
use crate::app::EngineState;

const BACKGROUND_RESULT_SESSION_ID: &str = "__daemon__";
const BACKGROUND_RESULT_KIND: &str = "background_result";

pub fn sync_detached_background_tasks_from_daemon(state: &mut EngineState) -> bool {
    sync_detached_background_tasks_from_daemon_with_paths(state, &DaemonPaths::default_user())
}

/// Persist a background agent's full result to a retrievable artifact so the
/// parent can read the complete report even when the inline completion reminder
/// is truncated. Returns the DB artifact handle, or `None` if it could not be written
/// (best-effort — never blocks the reminder).
pub fn persist_background_result(task_id: &str, body: &str) -> Option<std::path::PathBuf> {
    let safe: String = task_id
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    jfc_knowledge::block_on_knowledge(async {
        jfc_knowledge::KnowledgeStore::open_default()
            .await
            .ok()?
            .upsert_session_artifact(
                BACKGROUND_RESULT_SESSION_ID,
                BACKGROUND_RESULT_KIND,
                &safe,
                body,
            )
            .await
            .ok()
    })?;
    Some(std::path::PathBuf::from(format!(
        "db:background-result:{safe}"
    )))
}

fn sync_detached_background_tasks_from_daemon_with_paths(
    state: &mut EngineState,
    paths: &DaemonPaths,
) -> bool {
    let Some((daemon_state, mtime)) =
        crate::daemon::load_state_if_changed(paths, state.last_detached_state_mtime)
    else {
        return false;
    };

    state.last_detached_state_mtime = Some(mtime);
    let session_id = state.current_session_id.as_ref().map(|id| id.to_string());
    let mut changed = false;
    for (id, agent) in &daemon_state.background_agents {
        if agent.launch_path.is_none() {
            continue;
        }
        if let Some(ref sid) = session_id {
            if agent.parent_session_id.as_deref() != Some(sid.as_str()) {
                continue;
            }
        } else {
            continue;
        }

        let new_status = lifecycle_from_daemon_status(agent.status);
        let completed_at = background_agent_completed_at(agent, new_status);
        // Skip the per-poll log read for an agent already settled in the same
        // terminal state locally: its log file is frozen, so re-reading the
        // last 200 lines every second is pure I/O waste. A burst session can
        // hold 100+ such agents, so this is the bulk of the per-second sync
        // cost the render-thread poll was paying. Non-terminal, transitioning,
        // or not-yet-seen agents still read (the log may have grown).
        let log_is_frozen = state.background_tasks.get(id).is_some_and(|e| {
            new_status.is_terminal() && e.status == new_status && e.status.is_terminal()
        });
        let messages = if log_is_frozen {
            None
        } else {
            Some(crate::daemon::read_last_lines(&agent.log_path, 200))
        };
        let mut terminal_completion = None;
        let entry = state
            .background_tasks
            .entry(id.clone())
            .or_insert_with(|| BackgroundTask {
                task_id: TaskId::from(id.clone()),
                description: agent.description.clone(),
                status: new_status,
                started_at: instant_from_system_time(agent.started_at),
                completed_at,
                summary: agent.summary.clone(),
                error: agent.error.clone(),
                last_tool: agent.last_tool.clone(),
                last_tool_info: agent.last_tool_info.clone(),
                recent_activities: Vec::new(),
                messages: Vec::new(),
                chat_messages: Vec::new(),
                tool_use_count: agent.tool_use_count,
                latest_input_tokens: agent.latest_input_tokens,
                latest_cache_read_tokens: agent.latest_cache_read_tokens,
                latest_cache_write_tokens: agent.latest_cache_write_tokens,
                cumulative_output_tokens: agent.cumulative_output_tokens,
                model_used: agent.model.clone(),
                agent_messages: Vec::new(),
                max_input_tokens: None,
                budget_killed: false,
                parent_task_id: None,
                workflow_progress: None,
                last_activity_at: std::time::Instant::now(),
            });

        if entry.description != agent.description {
            entry.description = agent.description.clone();
            changed = true;
        }
        if entry.status != new_status {
            let was_terminal = entry.status.is_terminal();
            entry.status = new_status;
            if !was_terminal && new_status.is_terminal() {
                terminal_completion = Some(background_agent_completion(id, agent, new_status));
                // Real terminal transition observed this process — unblocks
                // the Case-2 auto-wake. (Restored prior-session agents arrive
                // already-terminal and skip this branch, so `--continue` won't
                // fire an unsolicited summary turn at startup.)
                state.observed_bg_terminal_transition_this_process = true;
            }
            changed = true;
        }
        if entry.completed_at != completed_at {
            entry.completed_at = completed_at;
            changed = true;
        }
        if entry.tool_use_count != agent.tool_use_count {
            entry.tool_use_count = agent.tool_use_count;
            changed = true;
        }
        if entry.latest_input_tokens != agent.latest_input_tokens {
            entry.latest_input_tokens = agent.latest_input_tokens;
            changed = true;
        }
        if entry.latest_cache_read_tokens != agent.latest_cache_read_tokens {
            entry.latest_cache_read_tokens = agent.latest_cache_read_tokens;
            changed = true;
        }
        if entry.latest_cache_write_tokens != agent.latest_cache_write_tokens {
            entry.latest_cache_write_tokens = agent.latest_cache_write_tokens;
            changed = true;
        }
        if entry.cumulative_output_tokens != agent.cumulative_output_tokens {
            entry.cumulative_output_tokens = agent.cumulative_output_tokens;
            changed = true;
        }
        if entry.last_tool != agent.last_tool {
            entry.last_tool = agent.last_tool.clone();
            changed = true;
        }
        if entry.last_tool_info != agent.last_tool_info {
            entry.last_tool_info = agent.last_tool_info.clone();
            changed = true;
        }
        if entry.summary != agent.summary {
            entry.summary = agent.summary.clone();
            changed = true;
        }
        if entry.error != agent.error {
            entry.error = agent.error.clone();
            changed = true;
        }
        if entry.model_used != agent.model {
            entry.model_used = agent.model.clone();
            changed = true;
        }
        // `messages` is `None` when the log read was skipped (frozen terminal
        // agent) — leave the cached messages/chat_messages untouched in that
        // case.
        if let Some(messages) = messages.as_ref() {
            if entry.messages != *messages {
                entry.messages = messages.clone();
                entry.chat_messages = parse_agent_log_to_chat_messages(messages);
                changed = true;
            }
            // Detached/daemon-launched agents never see live `EngineEvent`s, so
            // `chat_messages` stays empty unless we reconstruct it from the
            // persisted log. The parser is the sole writer for detached
            // agents (live events for attached ones are filtered out by the
            // early `continue` above), so the two writers never race.
            if entry.chat_messages.is_empty() && !messages.is_empty() {
                entry.chat_messages = parse_agent_log_to_chat_messages(messages);
                changed = true;
            }
        }

        // Any observable change from the poll counts as activity — keeps
        // the fan's `stalled Ns` flag honest for detached/daemon agents,
        // which never emit live `EngineEvent`s and so wouldn't otherwise
        // refresh `last_activity_at` between polls. Done before the
        // status-parts call below (which re-borrows `state`) so `entry`'s
        // borrow doesn't span it.
        if changed {
            entry.last_activity_at = std::time::Instant::now();
        }

        if let Some(completion) = terminal_completion {
            // Background agents run detached from the foreground turn, so the
            // turn-complete notification never covers them. Fire a desktop
            // notification here — this is the only signal a user focused
            // elsewhere gets that a long-running agent finished. Gated by the
            // same `JFC_DISABLE_NOTIFICATIONS` env as every other notify call.
            crate::notifications::notify_background_agent_done(
                completion.status.label(),
                &completion.description,
                &completion.body,
            );
            // SubagentStop hook: fires on every terminal transition so
            // external scripts can react (CI, Slack, logging, etc.).
            crate::hooks::fire_async(
                crate::hooks::HookPoint::SubagentStop,
                &crate::hooks::HookContext::for_agent(
                    &completion.description,
                    state
                        .current_session_id
                        .as_ref()
                        .map(|s| s.as_str())
                        .unwrap_or("<no-session>"),
                )
                .with_extra("task_id", completion.task_id.to_string())
                .with_extra("status", completion.status.label()),
            );
            state.queue_background_agent_completion(completion);
        }

        if update_task_status_parts_for_background_agent(state, id, new_status, agent) {
            changed = true;
        }
    }
    changed
}

fn background_agent_completion(
    id: &str,
    agent: &BackgroundAgentInfo,
    status: TaskLifecycle,
) -> BackgroundAgentCompletion {
    let body = agent
        .summary
        .as_deref()
        .or(agent.error.as_deref())
        .unwrap_or("(no output)")
        .to_owned();
    BackgroundAgentCompletion {
        task_id: TaskId::from(id.to_owned()),
        description: agent.description.clone(),
        status,
        body,
    }
}

fn update_task_status_parts_for_background_agent(
    state: &mut EngineState,
    id: &str,
    status: TaskLifecycle,
    agent: &BackgroundAgentInfo,
) -> bool {
    let task_id = TaskId::from(id.to_owned());
    let mut changed = false;
    for msg in &mut state.messages {
        for part in &mut msg.parts {
            if let MessagePart::TaskStatus(status_part) = part
                && status_part.task_id == task_id
            {
                if status_part.status != status {
                    status_part.status = status;
                    changed = true;
                }
                if status_part.summary != agent.summary {
                    status_part.summary = agent.summary.clone();
                    changed = true;
                }
                if status_part.error != agent.error {
                    status_part.error = agent.error.clone();
                    changed = true;
                }
            }
        }
    }
    changed
}

fn instant_from_system_time(t: std::time::SystemTime) -> std::time::Instant {
    let elapsed = std::time::SystemTime::now()
        .duration_since(t)
        .unwrap_or_default();
    std::time::Instant::now()
        .checked_sub(elapsed)
        .unwrap_or_else(std::time::Instant::now)
}

pub fn restore_persistent_background_agents(state: &mut EngineState) {
    let paths = DaemonPaths::default_user();
    let session_id = state.current_session_id.as_ref().map(|id| id.as_str());
    for agent in crate::daemon::background_agents_for_restore(&paths, session_id, 20) {
        let status = lifecycle_from_daemon_status(agent.status);
        let completed_at = background_agent_completed_at(&agent, status);
        let messages = crate::daemon::read_last_lines(&agent.log_path, 200);
        let chat_messages = parse_agent_log_to_chat_messages(&messages);
        state.background_tasks.insert(
            agent.id.clone(),
            BackgroundTask {
                task_id: TaskId::from(agent.id),
                description: agent.description,
                status,
                started_at: std::time::Instant::now(),
                completed_at,
                summary: agent.summary,
                error: agent.error,
                last_tool: agent.last_tool,
                last_tool_info: agent.last_tool_info,
                recent_activities: Vec::new(),
                messages,
                chat_messages,
                tool_use_count: agent.tool_use_count,
                latest_input_tokens: agent.latest_input_tokens,
                latest_cache_read_tokens: agent.latest_cache_read_tokens,
                latest_cache_write_tokens: agent.latest_cache_write_tokens,
                cumulative_output_tokens: agent.cumulative_output_tokens,
                model_used: agent.model,
                agent_messages: Vec::new(),
                max_input_tokens: None,
                budget_killed: false,
                parent_task_id: None,
                workflow_progress: None,
                last_activity_at: std::time::Instant::now(),
            },
        );
    }
}

fn background_agent_completed_at(
    agent: &BackgroundAgentInfo,
    status: TaskLifecycle,
) -> Option<std::time::Instant> {
    status
        .is_terminal()
        .then(|| instant_from_system_time(agent.completed_at.unwrap_or(agent.updated_at)))
}

fn lifecycle_from_daemon_status(status: BackgroundAgentStatus) -> TaskLifecycle {
    match status {
        BackgroundAgentStatus::Running => TaskLifecycle::Running,
        BackgroundAgentStatus::Completed => TaskLifecycle::Completed,
        BackgroundAgentStatus::Failed => TaskLifecycle::Failed,
        BackgroundAgentStatus::Cancelled => TaskLifecycle::Cancelled,
    }
}

#[cfg(test)]
mod tests {
    use crate::app::EngineState;
    use std::sync::Arc;
    use std::time::SystemTime;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};

    use super::*;
    use crate::daemon::{DaemonState, save_state};
    use crate::types::{MessagePart, Role};

    struct StubProvider;

    #[async_trait::async_trait]
    impl Provider for StubProvider {
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

    impl jfc_provider::seal::Sealed for StubProvider {}

    fn app_for_session(session_id: &str) -> EngineState {
        let mut state = EngineState::new(Arc::new(StubProvider), "test-model");
        state.current_session_id = Some(crate::ids::SessionId::new(session_id));
        state
    }

    fn agent_info(
        paths: &DaemonPaths,
        id: &str,
        session_id: &str,
        status: BackgroundAgentStatus,
    ) -> BackgroundAgentInfo {
        let now = SystemTime::now();
        BackgroundAgentInfo {
            id: id.to_owned(),
            description: "audit worker".to_owned(),
            parent_session_id: Some(session_id.to_owned()),
            status,
            started_at: now,
            updated_at: now,
            completed_at: status.is_terminal().then_some(now),
            pid: Some(std::process::id()),
            worker_epoch: 0,
            last_heartbeat_at: None,
            takeover_count: 0,
            model: Some("test-model".to_owned()),
            worktree_path: None,
            log_path: paths.log_dir.join("agents").join(format!("{id}.log")),
            launch_path: Some(
                paths
                    .log_dir
                    .join("agents")
                    .join(format!("{id}.launch.json")),
            ),
            cancel_requested: false,
            respawn_count: 0,
            summary: None,
            error: None,
            tool_use_count: 0,
            latest_input_tokens: 0,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: 0,
            last_tool: None,
            last_tool_info: None,
        }
    }

    #[test]
    fn daemon_sync_reparses_chat_messages_when_log_reaches_terminal_normal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = DaemonPaths::new(dir.path());
        let session_id = "ses_test";
        let task_id = "task-detached";
        let mut state = app_for_session(session_id);

        let agent = agent_info(&paths, task_id, session_id, BackgroundAgentStatus::Running);
        let log_path = agent.log_path.clone();
        std::fs::create_dir_all(log_path.parent().expect("log parent")).expect("mkdir");
        std::fs::write(&log_path, "initial output\n").expect("write log");
        let mut daemon_state = DaemonState::default();
        daemon_state
            .background_agents
            .insert(task_id.to_owned(), agent);
        save_state(&paths, &daemon_state).expect("save running state");

        assert!(sync_detached_background_tasks_from_daemon_with_paths(
            &mut state, &paths
        ));
        let bt = state
            .background_tasks
            .get(task_id)
            .expect("background task");
        assert_eq!(bt.status, TaskLifecycle::Running);
        assert!(bt.completed_at.is_none());
        assert!(bt.chat_messages.iter().any(|msg| {
            msg.role == Role::Assistant
                && matches!(msg.parts.as_slice(), [MessagePart::Text(text)] if text.contains("initial output"))
        }));
        assert!(state.pending_background_agent_completions.is_empty());

        let now = SystemTime::now();
        let mut completed = state
            .background_tasks
            .get(task_id)
            .unwrap()
            .description
            .clone();
        completed.push_str(" done");
        let agent = daemon_state
            .background_agents
            .get_mut(task_id)
            .expect("agent");
        agent.status = BackgroundAgentStatus::Completed;
        agent.updated_at = now;
        agent.completed_at = Some(now);
        agent.summary = Some(completed);
        std::fs::write(&log_path, "initial output\n[Completed] done\n").expect("write log");
        save_state(&paths, &daemon_state).expect("save completed state");

        state.last_detached_state_mtime = None;
        assert!(sync_detached_background_tasks_from_daemon_with_paths(
            &mut state, &paths
        ));
        let bt = state
            .background_tasks
            .get(task_id)
            .expect("background task");
        assert_eq!(bt.status, TaskLifecycle::Completed);
        assert_eq!(state.background_tasks_completed_since_last_turn, 1);
        assert_eq!(state.pending_background_agent_completions.len(), 1);
        let completion = &state.pending_background_agent_completions[0];
        assert_eq!(completion.description, "audit worker");
        assert_eq!(completion.status, TaskLifecycle::Completed);
        assert_eq!(completion.body, "audit worker done");
        assert!(
            bt.completed_at.is_some(),
            "daemon terminal agents should pin from completion time"
        );
        assert!(bt.chat_messages.iter().any(|msg| {
            matches!(
                msg.parts.as_slice(),
                [MessagePart::TaskStatus(ts)]
                    if ts.status == TaskLifecycle::Completed
                        && ts.summary.as_deref() == Some("done")
            )
        }));
    }

    // Once an agent is settled in a terminal state locally, a later change to
    // its (frozen) log file is NOT re-read on the next poll — the per-second
    // sync skips the I/O for already-done agents. We prove the skip by mutating
    // the log after completion and asserting the cached messages don't change.
    #[test]
    fn daemon_sync_skips_log_read_for_settled_terminal_agent_robust() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = DaemonPaths::new(dir.path());
        let session_id = "ses_frozen";
        let task_id = "task-frozen";
        let mut state = app_for_session(session_id);

        let agent = agent_info(
            &paths,
            task_id,
            session_id,
            BackgroundAgentStatus::Completed,
        );
        let log_path = agent.log_path.clone();
        std::fs::create_dir_all(log_path.parent().expect("log parent")).expect("mkdir");
        std::fs::write(&log_path, "first output\n").expect("write log");
        let mut daemon_state = DaemonState::default();
        daemon_state
            .background_agents
            .insert(task_id.to_owned(), agent);
        save_state(&paths, &daemon_state).expect("save completed state");

        // First poll: agent is seen as Completed and its log is read once.
        assert!(sync_detached_background_tasks_from_daemon_with_paths(
            &mut state, &paths
        ));
        let before: Vec<String> = state
            .background_tasks
            .get(task_id)
            .expect("task")
            .messages
            .clone();
        assert!(before.iter().any(|l| l.contains("first output")));

        // Mutate the (supposedly frozen) log and force a re-poll. Because the
        // agent is already terminal locally with the same status, the log read
        // is skipped and the cached messages stay put.
        std::fs::write(&log_path, "first output\nLEAKED SECOND READ\n").expect("rewrite log");
        state.last_detached_state_mtime = None;
        // Re-save so the state mtime advances and the poll actually runs.
        save_state(&paths, &daemon_state).expect("re-save");
        sync_detached_background_tasks_from_daemon_with_paths(&mut state, &paths);

        let after: Vec<String> = state
            .background_tasks
            .get(task_id)
            .expect("task")
            .messages
            .clone();
        assert_eq!(
            before, after,
            "frozen terminal agent must not re-read its log on a later poll"
        );
        assert!(
            !after.iter().any(|l| l.contains("LEAKED SECOND READ")),
            "skipped log read leaked new content: {after:?}"
        );
    }
}
