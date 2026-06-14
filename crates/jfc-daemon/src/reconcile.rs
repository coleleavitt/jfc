//! Background-agent reconciliation pass.
//!
//! `reconcile_background_agents` is called from every CLI query path and
//! from the daemon's main loop. It walks every `Running` agent and:
//!
//! 1. If the owning PID is gone, requests one respawn from the persisted
//!    `BackgroundAgentLaunch` (capped by `JFC_DAEMON_WORKER_RESPAWN_LIMIT`)
//!    unless the
//!    agent was cancelled.
//! 2. Otherwise marks the agent `Failed` with an explanatory error.
//!
//! The old `/proc/<pid>/cmdline` repair pass is gone now that the PID
//! write contract in `registry::record_background_agent_started_at`
//! prevents the UI from clobbering the worker's PID in the first place.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::logs::{append_log_line, background_agent_log_path};
use super::pid::process_is_running;
use super::state::{
    BackgroundAgentInfo, BackgroundAgentLaunch, BackgroundAgentStatus, DaemonPaths, DaemonState,
    load_state_for_update, save_state, with_state_lock,
};
use super::worker::{
    arm_worker_launch_epoch, mark_background_agent_spawn_failed, reap_worker_process,
    record_background_agent_worker_pid, spawn_worker_process,
};

pub(crate) fn reconcile_background_agents(paths: &DaemonPaths) -> std::io::Result<DaemonState> {
    let (state, stale_logs, respawns) = with_state_lock(paths, || -> std::io::Result<_> {
        let mut state = load_state_for_update(paths)?;
        let now = SystemTime::now();
        let mut changed = false;
        let mut stale_logs: Vec<(PathBuf, String)> = Vec::new();
        let mut respawns: Vec<(String, PathBuf, BackgroundAgentLaunch)> = Vec::new();

        for agent in state.background_agents.values_mut() {
            if agent.status != BackgroundAgentStatus::Running {
                continue;
            }
            let owner_alive = agent.pid.map(process_is_running).unwrap_or(false);
            let previous_pid = agent.pid;
            if owner_alive {
                if !agent.cancel_requested
                    && heartbeat_is_stale(agent, now)
                    && agent.respawn_count < worker_respawn_limit()
                    && !low_memory_respawn_blocked()
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
                            "[takeover-requested] pid {:?} heartbeat stale; starting replacement worker",
                            previous_pid
                        ),
                    ));
                    respawns.push((agent.id.clone(), launch_path, launch));
                    changed = true;
                }
                continue;
            }
            if !agent.cancel_requested
                && agent.respawn_count < worker_respawn_limit()
                && !low_memory_respawn_blocked()
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
                    if low_memory_respawn_blocked() {
                        format!(
                            "stale: owning process {pid} exited before reporting completion; respawn suppressed by low-memory threshold"
                        )
                    } else {
                        format!("stale: owning process {pid} exited before reporting completion")
                    }
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
        let launch = match arm_worker_launch_epoch(paths, &launch_path, launch, true) {
            Ok(launch) => launch,
            Err(e) => {
                mark_background_agent_spawn_failed(paths, &id, &format!("respawn failed: {e}"))?;
                continue;
            }
        };
        match spawn_worker_process(&launch_path, &launch) {
            Ok(child) => {
                let pid = child.id();
                record_background_agent_worker_pid(
                    paths,
                    &id,
                    pid,
                    &launch_path,
                    launch.worker_epoch,
                )?;
                reap_worker_process(child);
                append_log_line(
                    &background_agent_log_path(paths, &id),
                    &format!("[respawned] pid={pid} epoch={}", launch.worker_epoch),
                );
            }
            Err(e) => {
                mark_background_agent_spawn_failed(paths, &id, &format!("respawn failed: {e}"))?;
            }
        }
    }

    Ok(state)
}

fn worker_respawn_limit() -> u32 {
    std::env::var("JFC_DAEMON_WORKER_RESPAWN_LIMIT")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(1)
}

fn heartbeat_is_stale(agent: &BackgroundAgentInfo, now: SystemTime) -> bool {
    let Some(last) = agent.last_heartbeat_at else {
        return false;
    };
    let Some(stale_after) = worker_heartbeat_stale_after() else {
        return false;
    };
    now.duration_since(last)
        .is_ok_and(|elapsed| elapsed >= stale_after)
}

fn worker_heartbeat_stale_after() -> Option<Duration> {
    std::env::var("JFC_DAEMON_WORKER_HEARTBEAT_STALE_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .map(Duration::from_secs)
        .or(Some(Duration::from_secs(120)))
        .filter(|duration| !duration.is_zero())
}

fn low_memory_respawn_blocked() -> bool {
    let Some(threshold_mb) = std::env::var("JFC_DAEMON_LOW_MEM_MB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
    else {
        return false;
    };
    available_memory_mb().is_some_and(|available| available < threshold_mb)
}

#[cfg(target_os = "linux")]
pub(super) fn available_memory_mb() -> Option<u64> {
    let meminfo = std::fs::read_to_string("/proc/meminfo").ok()?;
    for line in meminfo.lines() {
        let Some(rest) = line.strip_prefix("MemAvailable:") else {
            continue;
        };
        let kb = rest.split_whitespace().next()?.parse::<u64>().ok()?;
        return Some(kb / 1024);
    }
    None
}

#[cfg(not(target_os = "linux"))]
pub(super) fn available_memory_mb() -> Option<u64> {
    None
}
