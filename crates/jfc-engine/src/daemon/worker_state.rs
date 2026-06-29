use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::logs::append_log_line;
use super::state::{BackgroundAgentStatus, DaemonPaths, load_state, save_state, with_state_lock};

pub(super) fn mark_background_agent_spawn_failed(
    paths: &DaemonPaths,
    id: &str,
    error: &str,
) -> std::io::Result<()> {
    let log_path = with_state_lock(paths, || -> std::io::Result<Option<PathBuf>> {
        let mut state = load_state(paths).unwrap_or_default();
        let now = SystemTime::now();
        let Some(agent) = state.background_agents.get_mut(id) else {
            return Ok(None);
        };
        agent.status = BackgroundAgentStatus::Failed;
        agent.updated_at = now;
        agent.completed_at = Some(now);
        agent.error = Some(error.to_owned());
        let log_path = agent.log_path.clone();
        save_state(paths, &state)?;
        Ok(Some(log_path))
    })?;
    if let Some(log_path) = log_path {
        append_log_line(&log_path, &format!("[Failed] {error}"));
    }
    Ok(())
}

pub(super) fn record_background_agent_launch_path(
    paths: &DaemonPaths,
    id: &str,
    launch_path: &Path,
) -> std::io::Result<()> {
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state(paths).unwrap_or_default();
        if let Some(agent) = state.background_agents.get_mut(id) {
            agent.launch_path = Some(launch_path.to_path_buf());
            agent.updated_at = SystemTime::now();
        }
        save_state(paths, &state)
    })
}
