use crate::{
    app::{App, BackgroundTask},
    daemon::{BackgroundAgentInfo, BackgroundAgentStatus, DaemonPaths},
    ids::TaskId,
    types::{MessagePart, TaskLifecycle},
};

use super::agent_log_parser::parse_agent_log_to_chat_messages;

pub(crate) fn sync_detached_background_tasks_from_daemon(app: &mut App) -> bool {
    sync_detached_background_tasks_from_daemon_with_paths(app, &DaemonPaths::default_user())
}

fn sync_detached_background_tasks_from_daemon_with_paths(
    app: &mut App,
    paths: &DaemonPaths,
) -> bool {
    let Some((state, mtime)) =
        crate::daemon::load_state_if_changed(paths, app.last_detached_state_mtime)
    else {
        return false;
    };

    app.last_detached_state_mtime = Some(mtime);
    let session_id = app.current_session_id.as_ref().map(|id| id.to_string());
    let mut changed = false;
    for (id, agent) in &state.background_agents {
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
        let messages = crate::daemon::read_last_lines(&agent.log_path, 200);
        let entry = app
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
            });

        if entry.description != agent.description {
            entry.description = agent.description.clone();
            changed = true;
        }
        if entry.status != new_status {
            let was_terminal = entry.status.is_terminal();
            entry.status = new_status;
            if !was_terminal && new_status.is_terminal() {
                app.background_tasks_completed_since_last_turn += 1;
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
        if entry.messages != messages {
            entry.messages = messages.clone();
            entry.chat_messages = parse_agent_log_to_chat_messages(&messages);
            changed = true;
        }
        // Detached/daemon-launched agents never see live `AppEvent`s, so
        // `chat_messages` stays empty unless we reconstruct it from the
        // persisted log. The parser is the sole writer for detached
        // agents (live events for attached ones are filtered out by the
        // early `continue` above), so the two writers never race.
        if entry.chat_messages.is_empty() && !messages.is_empty() {
            entry.chat_messages = parse_agent_log_to_chat_messages(&messages);
            changed = true;
        }

        if update_task_status_parts_for_background_agent(app, id, new_status, agent) {
            changed = true;
        }
    }
    changed
}

fn update_task_status_parts_for_background_agent(
    app: &mut App,
    id: &str,
    status: TaskLifecycle,
    agent: &BackgroundAgentInfo,
) -> bool {
    let task_id = TaskId::from(id.to_owned());
    let mut changed = false;
    for msg in &mut app.messages {
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

pub(crate) fn restore_persistent_background_agents(app: &mut App) {
    let paths = DaemonPaths::default_user();
    let session_id = app.current_session_id.as_ref().map(|id| id.as_str());
    for agent in crate::daemon::background_agents_for_restore(&paths, session_id, 20) {
        let status = lifecycle_from_daemon_status(agent.status);
        let completed_at = background_agent_completed_at(&agent, status);
        let messages = crate::daemon::read_last_lines(&agent.log_path, 200);
        let chat_messages = parse_agent_log_to_chat_messages(&messages);
        app.background_tasks.insert(
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

    fn app_for_session(session_id: &str) -> App {
        let mut app = App::new(Arc::new(StubProvider), "test-model");
        app.current_session_id = Some(crate::ids::SessionId::new(session_id));
        app
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
        }
    }

    #[test]
    fn daemon_sync_reparses_chat_messages_when_log_reaches_terminal_normal() {
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = DaemonPaths::new(dir.path());
        let session_id = "ses_test";
        let task_id = "task-detached";
        let mut app = app_for_session(session_id);

        let agent = agent_info(&paths, task_id, session_id, BackgroundAgentStatus::Running);
        let log_path = agent.log_path.clone();
        std::fs::create_dir_all(log_path.parent().expect("log parent")).expect("mkdir");
        std::fs::write(&log_path, "initial output\n").expect("write log");
        let mut state = DaemonState::default();
        state.background_agents.insert(task_id.to_owned(), agent);
        save_state(&paths, &state).expect("save running state");

        assert!(sync_detached_background_tasks_from_daemon_with_paths(
            &mut app, &paths
        ));
        let bt = app.background_tasks.get(task_id).expect("background task");
        assert_eq!(bt.status, TaskLifecycle::Running);
        assert!(bt.completed_at.is_none());
        assert!(bt.chat_messages.iter().any(|msg| {
            msg.role == Role::Assistant
                && matches!(msg.parts.as_slice(), [MessagePart::Text(text)] if text.contains("initial output"))
        }));

        let now = SystemTime::now();
        let mut completed = app
            .background_tasks
            .get(task_id)
            .unwrap()
            .description
            .clone();
        completed.push_str(" done");
        let agent = state.background_agents.get_mut(task_id).expect("agent");
        agent.status = BackgroundAgentStatus::Completed;
        agent.updated_at = now;
        agent.completed_at = Some(now);
        agent.summary = Some(completed);
        std::fs::write(&log_path, "initial output\n[Completed] done\n").expect("write log");
        save_state(&paths, &state).expect("save completed state");

        app.last_detached_state_mtime = None;
        assert!(sync_detached_background_tasks_from_daemon_with_paths(
            &mut app, &paths
        ));
        let bt = app.background_tasks.get(task_id).expect("background task");
        assert_eq!(bt.status, TaskLifecycle::Completed);
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
}
