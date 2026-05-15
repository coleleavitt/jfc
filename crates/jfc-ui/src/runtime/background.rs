use crate::{
    app::{App, BackgroundTask},
    daemon::{BackgroundAgentInfo, BackgroundAgentStatus, DaemonPaths},
    ids::TaskId,
    types::{MessagePart, TaskLifecycle},
};

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
        let messages = crate::daemon::read_last_lines(&agent.log_path, 200);
        let entry = app
            .background_tasks
            .entry(id.clone())
            .or_insert_with(|| BackgroundTask {
                task_id: TaskId::from(id.clone()),
                description: agent.description.clone(),
                status: new_status,
                started_at: instant_from_system_time(agent.started_at),
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
                max_input_tokens: None,
                budget_killed: false,
                parent_task_id: None,
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
            entry.messages = messages;
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
        let messages = crate::daemon::read_last_lines(&agent.log_path, 200);
        app.background_tasks.insert(
            agent.id.clone(),
            BackgroundTask {
                task_id: TaskId::from(agent.id),
                description: agent.description,
                status,
                started_at: std::time::Instant::now(),
                summary: agent.summary,
                error: agent.error,
                last_tool: None,
                messages,
                chat_messages: Vec::new(),
                tool_use_count: agent.tool_use_count,
                latest_input_tokens: agent.latest_input_tokens,
                latest_cache_read_tokens: 0,
                latest_cache_write_tokens: 0,
                cumulative_output_tokens: agent.cumulative_output_tokens,
                model_used: agent.model,
                max_input_tokens: None,
                budget_killed: false,
                parent_task_id: None,
            },
        );
    }
}

fn lifecycle_from_daemon_status(status: BackgroundAgentStatus) -> TaskLifecycle {
    match status {
        BackgroundAgentStatus::Running => TaskLifecycle::Running,
        BackgroundAgentStatus::Completed => TaskLifecycle::Completed,
        BackgroundAgentStatus::Failed => TaskLifecycle::Failed,
        BackgroundAgentStatus::Cancelled => TaskLifecycle::Cancelled,
    }
}
