//! Durable daemon worker control plane.
//!
//! Claude's hosted worker model has a queue/lease protocol. JFC keeps this
//! local: CLI callers append control records to daemon state, and the daemon
//! applies them from its tick loop under the same state lock used by workers.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::logs::append_log_line;
use super::reconcile::available_memory_mb;
use super::runtime::uuid_short;
use super::state::{
    DaemonPaths, DaemonState, WorkerControlKind, WorkerControlRecord, WorkerControlStatus,
    load_state_for_update, save_state, with_state_lock,
};
use super::worker::{resolve_worker_exe, takeover_background_agent_worker_with_paths};

#[derive(Debug, Clone)]
pub struct WorkerControlRequest {
    pub kind: WorkerControlKind,
    pub agent_id: Option<String>,
    pub target_pid: Option<u32>,
    pub worker_exe: Option<PathBuf>,
    pub force: bool,
    pub reason: Option<String>,
}

pub fn request_worker_control(
    paths: &DaemonPaths,
    req: WorkerControlRequest,
) -> std::io::Result<String> {
    with_state_lock(paths, || -> std::io::Result<String> {
        let mut state = load_state_for_update(paths)?;
        let now = SystemTime::now();
        let id = format!("ctrl-{}", uuid_short());
        state.worker_controls.push(WorkerControlRecord {
            id: id.clone(),
            kind: req.kind,
            status: WorkerControlStatus::Pending,
            requested_at: now,
            updated_at: now,
            agent_id: req.agent_id,
            target_pid: req.target_pid,
            worker_exe: req.worker_exe,
            force: req.force,
            reason: req.reason,
            result_pid: None,
            message: None,
        });
        save_state(paths, &state)?;
        Ok(id)
    })
}

pub fn worker_controls_string(paths: &DaemonPaths) -> String {
    let state = super::state::load_state(paths).unwrap_or_default();
    let mut out = String::new();
    out.push_str("worker controls:\n");
    if state.worker_controls.is_empty() {
        out.push_str("  (none)\n");
        return out;
    }
    for rec in state.worker_controls.iter().rev().take(50) {
        out.push_str(&format!(
            "  {} {:?} {:?} agent={} pid={} result={} {}\n",
            rec.id,
            rec.kind,
            rec.status,
            rec.agent_id.as_deref().unwrap_or("-"),
            rec.target_pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            rec.result_pid
                .map(|p| p.to_string())
                .unwrap_or_else(|| "-".to_owned()),
            rec.message.as_deref().unwrap_or("")
        ));
    }
    out
}

pub fn apply_worker_control_requests(paths: &DaemonPaths, state: &mut DaemonState) -> bool {
    let pending: Vec<String> = state
        .worker_controls
        .iter()
        .filter(|rec| rec.status == WorkerControlStatus::Pending)
        .map(|rec| rec.id.clone())
        .collect();
    if pending.is_empty() {
        return false;
    }

    let mut changed = false;
    for id in pending {
        if mark_running(paths, &id).is_err() {
            continue;
        }
        let result = apply_one(paths, &id);
        let _ = finish(paths, &id, result);
        changed = true;
    }

    if let Ok(fresh) = load_state_for_update(paths) {
        *state = fresh;
    }
    changed
}

fn apply_one(paths: &DaemonPaths, id: &str) -> std::io::Result<ApplyResult> {
    let rec = with_state_lock(paths, || -> std::io::Result<WorkerControlRecord> {
        let state = load_state_for_update(paths)?;
        state
            .worker_controls
            .iter()
            .find(|rec| rec.id == id)
            .cloned()
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "control not found"))
    })?;

    match rec.kind {
        WorkerControlKind::PrepareSpare => prepare_spare(paths, rec.worker_exe.as_deref()),
        WorkerControlKind::BinaryTakeover | WorkerControlKind::RestartOnUpgrade => {
            binary_takeover(paths, rec.worker_exe.as_deref(), rec.kind)
        }
        WorkerControlKind::RetireLowMemory => retire_low_memory(paths),
        WorkerControlKind::Takeover => {
            let agent_id = rec.agent_id.as_deref().ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::InvalidInput, "takeover needs agent_id")
            })?;
            let reason = rec.reason.as_deref().unwrap_or("manual takeover");
            let pid =
                takeover_background_agent_worker_with_paths(paths, agent_id, rec.force, reason)?;
            Ok(ApplyResult {
                pid: Some(pid),
                message: format!("takeover started worker pid {pid}"),
            })
        }
    }
}

fn prepare_spare(paths: &DaemonPaths, preferred: Option<&Path>) -> std::io::Result<ApplyResult> {
    let worker = resolve_worker_exe(preferred)?;
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state_for_update(paths)?;
        state.runtime.worker_exe = Some(worker.clone());
        state.runtime.worker_exe_mtime = std::fs::metadata(&worker)
            .ok()
            .and_then(|meta| meta.modified().ok());
        state.runtime.spare_ready = true;
        state.runtime.spare_checked_at = Some(SystemTime::now());
        save_state(paths, &state)
    })?;
    Ok(ApplyResult {
        pid: None,
        message: format!("spare ready: {}", worker.display()),
    })
}

fn binary_takeover(
    paths: &DaemonPaths,
    preferred: Option<&Path>,
    kind: WorkerControlKind,
) -> std::io::Result<ApplyResult> {
    let worker = resolve_worker_exe(preferred)?;
    let mtime = std::fs::metadata(&worker)
        .ok()
        .and_then(|meta| meta.modified().ok());
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state_for_update(paths)?;
        state.runtime.worker_exe = Some(worker.clone());
        state.runtime.worker_exe_mtime = mtime;
        state.runtime.restart_requested = true;
        state.runtime.restart_reason = Some(format!("{kind:?} requested"));
        save_state(paths, &state)
    })?;
    Ok(ApplyResult {
        pid: None,
        message: format!("restart requested with worker {}", worker.display()),
    })
}

fn retire_low_memory(paths: &DaemonPaths) -> std::io::Result<ApplyResult> {
    let available = available_memory_mb();
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state_for_update(paths)?;
        state.runtime.low_memory_retire_count =
            state.runtime.low_memory_retire_count.saturating_add(1);
        save_state(paths, &state)
    })?;
    Ok(ApplyResult {
        pid: None,
        message: format!("low-memory retirement requested; available_mb={available:?}"),
    })
}

fn mark_running(paths: &DaemonPaths, id: &str) -> std::io::Result<()> {
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state_for_update(paths)?;
        if let Some(rec) = state.worker_controls.iter_mut().find(|rec| rec.id == id)
            && rec.status == WorkerControlStatus::Pending
        {
            rec.status = WorkerControlStatus::Running;
            rec.updated_at = SystemTime::now();
        }
        save_state(paths, &state)
    })
}

struct ApplyResult {
    pid: Option<u32>,
    message: String,
}

fn finish(
    paths: &DaemonPaths,
    id: &str,
    result: std::io::Result<ApplyResult>,
) -> std::io::Result<()> {
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state_for_update(paths)?;
        if let Some(rec) = state.worker_controls.iter_mut().find(|rec| rec.id == id) {
            rec.updated_at = SystemTime::now();
            match result {
                Ok(result) => {
                    rec.status = WorkerControlStatus::Completed;
                    rec.result_pid = result.pid;
                    rec.message = Some(result.message);
                }
                Err(err) => {
                    rec.status = WorkerControlStatus::Failed;
                    rec.message = Some(err.to_string());
                }
            }
        }
        save_state(paths, &state)
    })?;

    if let Some(state) = super::state::load_state(paths)
        && let Some(rec) = state.worker_controls.iter().find(|rec| rec.id == id)
        && let Some(agent_id) = rec.agent_id.as_deref()
        && let Some(agent) = state.background_agents.get(agent_id)
        && let Some(message) = rec.message.as_deref()
    {
        append_log_line(
            &agent.log_path,
            &format!("[worker-control {}] {message}", rec.id),
        );
    }
    Ok(())
}
