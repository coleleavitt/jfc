//! Background-agent reconciliation pass.
//!
//! `reconcile_background_agents` is called from every CLI query path and
//! from the daemon's main loop. It walks every `Running` agent and:
//!
//! 1. If the owning PID is gone, requests one respawn from the persisted
//!    `BackgroundAgentLaunch` (capped at `respawn_count < 1`) unless the
//!    agent was cancelled.
//! 2. Otherwise marks the agent `Failed` with an explanatory error.
//!
//! The old `/proc/<pid>/cmdline` repair pass is gone now that the PID
//! write contract in `registry::record_background_agent_started_at`
//! prevents the UI from clobbering the worker's PID in the first place.

use std::path::PathBuf;
use std::time::SystemTime;

use super::logs::{append_log_line, background_agent_log_path};
use super::pid::process_is_running;
use super::state::{
    BackgroundAgentLaunch, BackgroundAgentStatus, DaemonPaths, DaemonState, load_state, save_state,
    with_state_lock,
};
use super::worker::{
    mark_background_agent_spawn_failed, reap_worker_process, record_background_agent_worker_pid,
    spawn_worker_process,
};

pub(super) fn reconcile_background_agents(paths: &DaemonPaths) -> std::io::Result<DaemonState> {
    let (state, stale_logs, respawns) = with_state_lock(paths, || -> std::io::Result<_> {
        let mut state = load_state(paths).unwrap_or_default();
        let now = SystemTime::now();
        let mut changed = false;
        let mut stale_logs: Vec<(PathBuf, String)> = Vec::new();
        let mut respawns: Vec<(String, PathBuf, BackgroundAgentLaunch)> = Vec::new();

        for agent in state.background_agents.values_mut() {
            if agent.status != BackgroundAgentStatus::Running {
                continue;
            }
            let owner_alive = agent.pid.map(process_is_running).unwrap_or(false);
            if owner_alive {
                continue;
            }
            let previous_pid = agent.pid;
            if !agent.cancel_requested
                && agent.respawn_count < 1
                && let Some(launch_path) = agent.launch_path.clone()
                && let Ok(launch_json) = std::fs::read_to_string(&launch_path)
                && let Ok(launch) = serde_json::from_str::<BackgroundAgentLaunch>(&launch_json)
            {
                agent.respawn_count = agent.respawn_count.saturating_add(1);
                agent.updated_at = now;
                agent.pid = None;
                stale_logs.push((
                    agent.log_path.clone(),
                    format!(
                        "[respawn-requested] previous pid {:?} exited; restarting worker",
                        previous_pid
                    ),
                ));
                respawns.push((agent.id.clone(), launch_path, launch));
                changed = true;
                continue;
            }
            agent.status = BackgroundAgentStatus::Failed;
            agent.updated_at = now;
            agent.completed_at = Some(now);
            agent.cancel_requested = false;
            let reason = match agent.pid {
                Some(pid) => {
                    format!("stale: owning process {pid} exited before reporting completion")
                }
                None => "stale: no owning process recorded".to_owned(),
            };
            agent.error = Some(reason.clone());
            stale_logs.push((agent.log_path.clone(), format!("[Failed] {reason}")));
            changed = true;
        }

        if changed {
            save_state(paths, &state)?;
        }
        Ok((state, stale_logs, respawns))
    })?;

    for (path, reason) in stale_logs {
        append_log_line(&path, &reason);
    }
    for (id, launch_path, launch) in respawns {
        match spawn_worker_process(&launch_path, &launch) {
            Ok(child) => {
                let pid = child.id();
                record_background_agent_worker_pid(paths, &id, pid, &launch_path)?;
                reap_worker_process(child);
                append_log_line(
                    &background_agent_log_path(paths, &id),
                    &format!("[respawned] pid={pid}"),
                );
            }
            Err(e) => {
                mark_background_agent_spawn_failed(paths, &id, &format!("respawn failed: {e}"))?;
            }
        }
    }

    Ok(state)
}
