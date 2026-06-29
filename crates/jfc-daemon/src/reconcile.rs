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
    arm_worker_launch_epoch, load_background_agent_launch, mark_background_agent_spawn_failed,
    reap_worker_process, record_background_agent_worker_pid, spawn_worker_process,
};

pub(crate) fn reconcile_background_agents(paths: &DaemonPaths) -> std::io::Result<DaemonState> {
    let (state, stale_logs, respawns, pruned_logs) = with_state_lock(
        paths,
        || -> std::io::Result<_> {
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
                        && let Ok(launch) = load_background_agent_launch(paths, &launch_path)
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
                    && let Ok(launch) = load_background_agent_launch(paths, &launch_path)
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
                            format!(
                                "stale: owning process {pid} exited before reporting completion"
                            )
                        }
                    }
                    None => "stale: no owning process recorded".to_owned(),
                };
                agent.error = Some(reason.clone());
                stale_logs.push((agent.log_path.clone(), format!("[Failed] {reason}")));
                changed = true;
            }

            // Retention: bound the number of terminal background-agent rows kept
            // in the state file. Workflow sub-agents (`bgwf_*:agent_N`) finish in
            // large bursts, and without a cap the `background_agents` map grew
            // without bound (the audit found 203 rows / 1 MB state, ~half phantom).
            // Keep the most recent `max_terminal_agents()` terminal rows; prune the
            // rest and delete their log files outside the lock.
            let pruned_logs = prune_terminal_agents(&mut state, &mut changed);

            if changed {
                save_state(paths, &state)?;
            }
            Ok((state, stale_logs, respawns, pruned_logs))
        },
    )?;

    for (path, reason) in stale_logs {
        append_log_line(&path, &reason);
    }
    // Delete log files for pruned agents and any orphaned agent logs/tmp
    // sidecars beyond the retention window. Best-effort: failures are logged
    // by the cleanup routine, never fatal to reconciliation.
    for path in pruned_logs {
        let _ = std::fs::remove_file(&path);
    }
    cleanup_agent_log_dir(paths);
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

/// Maximum terminal background-agent rows to retain in the state file.
/// Override with `JFC_DAEMON_MAX_TERMINAL_AGENTS` (0 disables pruning).
fn max_terminal_agents() -> usize {
    std::env::var("JFC_DAEMON_MAX_TERMINAL_AGENTS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(64)
}

/// Age after which an agent log file in the agents/ dir is eligible for
/// deletion. Override with `JFC_DAEMON_AGENT_LOG_TTL_SECS` (0 disables).
fn agent_log_ttl() -> Option<Duration> {
    let secs = std::env::var("JFC_DAEMON_AGENT_LOG_TTL_SECS")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(7 * 24 * 60 * 60); // 7 days
    (secs > 0).then(|| Duration::from_secs(secs))
}

/// Drop the oldest terminal agent rows beyond the retention bound, returning
/// the log-file paths of the pruned rows so the caller can delete them outside
/// the state lock. Running (non-terminal) rows are never pruned.
fn prune_terminal_agents(state: &mut DaemonState, changed: &mut bool) -> Vec<PathBuf> {
    let keep = max_terminal_agents();
    if keep == 0 {
        return Vec::new();
    }
    let mut terminal: Vec<(String, SystemTime)> = state
        .background_agents
        .iter()
        .filter(|(_, a)| a.status.is_terminal())
        .map(|(id, a)| (id.clone(), a.completed_at.unwrap_or(a.updated_at)))
        .collect();
    if terminal.len() <= keep {
        return Vec::new();
    }
    // Newest-first, then drop everything past `keep`.
    terminal.sort_by_key(|(_, completed_at)| std::cmp::Reverse(*completed_at));
    let mut pruned_logs = Vec::new();
    for (id, _) in terminal.into_iter().skip(keep) {
        if let Some(agent) = state.background_agents.remove(&id) {
            pruned_logs.push(agent.log_path);
            *changed = true;
        }
    }
    pruned_logs
}

/// Delete stale files in the agents/ log directory beyond the TTL. Covers
/// orphaned `.log` files (whose state row was already pruned) and leftover
/// `.launch.json` sidecars. Best-effort; never fatal.
fn cleanup_agent_log_dir(paths: &DaemonPaths) {
    let Some(ttl) = agent_log_ttl() else {
        return;
    };
    let dir = paths.log_dir.join("agents");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return;
    };
    let now = SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let modified = meta.modified().unwrap_or(now);
        if now
            .duration_since(modified)
            .is_ok_and(|elapsed| elapsed >= ttl)
        {
            let _ = std::fs::remove_file(&path);
        }
    }
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
