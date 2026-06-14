//! Long-running daemon process + CLI command handlers.
//!
//! - `Daemon` is the in-memory state wrapper used by `jfc daemon start`. It
//!   owns the cron/wakeup tick loop and persists state via `persist()`.
//!   `persist()` deliberately re-reads the on-disk `background_agents`
//!   subtree before writing so it doesn't clobber updates that detached
//!   workers wrote out-of-process — the moral equivalent of file locking
//!   for that single field.
//! - `run_daemon` is the entry point for `jfc daemon start` (cron + wakeup
//!   poll loop with `reconcile_background_agents` every tick).
//! - `status_string` / `list_string` are the CLI rendering helpers; they
//!   call `reconcile_background_agents` before rendering so dead workers
//!   show as `Failed` instead of phantom `Running`.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use super::cron::{CronJob, CronSchedule, describe_schedule, run_cron_command, should_fire_cron};
use super::logs::append_log_line;
use super::pid::{is_daemon_running, remove_pid_file, write_pid_file};
use super::reconcile::{available_memory_mb, reconcile_background_agents};
use super::state::{
    BackgroundAgentStatus, DaemonPaths, DaemonState, ScheduledWakeup, SessionId, SessionInfo,
    SessionStatus, load_state, load_state_for_update, save_state, with_state_lock,
};
use super::worker::{join_worker_reapers, resolve_worker_exe};

// ─────────────────────────────────────────────────────────────────────────────
// Worker tracking
// ─────────────────────────────────────────────────────────────────────────────

/// Metadata for an active background worker tracked in-memory by the daemon.
/// This is *not* persisted — it uses `Instant` for monotonic elapsed time and
/// lives only as long as the daemon process. It complements the on-disk
/// `BackgroundAgentInfo` roster by giving the running daemon a fast, lock-free
/// view of which worker processes are alive.
#[derive(Debug, Clone)]
pub struct WorkerInfo {
    pub label: String,
    pub pid: u32,
    pub cwd: PathBuf,
    pub started_at: Instant,
}

/// Default idle-exit timeout. If the daemon has no cron jobs, no pending
/// wakeups, no active sessions, and no active workers for this long, it
/// shuts itself down. Set `JFC_DAEMON_IDLE_TIMEOUT_SECS=0` to disable.
const DEFAULT_IDLE_TIMEOUT: Duration = Duration::from_secs(30 * 60); // 30 minutes

/// In-memory daemon state + I/O paths.
pub struct Daemon {
    pub paths: DaemonPaths,
    pub state: DaemonState,
    /// In-memory roster of active background workers. Not persisted — only
    /// meaningful while the daemon process is alive.
    workers: Vec<WorkerInfo>,
    /// Instant when the daemon last had meaningful work (workers, cron,
    /// wakeups, or active sessions). Used for idle-exit.
    last_activity: Instant,
}

impl Daemon {
    /// Open (or create) a daemon at the given config directory.
    pub fn new(config_dir: &Path) -> std::io::Result<Self> {
        let paths = DaemonPaths::new(config_dir);
        paths.ensure_dirs()?;

        let mut state = load_state(&paths).unwrap_or_default();
        if state.pid == 0 {
            state.pid = std::process::id();
        }
        if state.started_at == UNIX_EPOCH {
            state.started_at = SystemTime::now();
        }

        Ok(Self {
            paths,
            state,
            workers: Vec::new(),
            last_activity: Instant::now(),
        })
    }

    /// Persist current state to disk (best-effort).
    pub fn persist(&self) {
        // Merge + save under the state lock so we don't race a detached
        // worker's read-modify-write and clobber its background_agents
        // subtree. Skip the save entirely if the on-disk state is corrupt
        // rather than overwriting it with our in-memory copy.
        let _ = with_state_lock(&self.paths, || -> std::io::Result<()> {
            let mut state = self.state.clone();
            let current = load_state_for_update(&self.paths)?;
            // Background workers update their roster/log metadata out-of-process.
            // Preserve that live subtree when the cron daemon persists its own
            // in-memory cron/wakeup/session state.
            state.background_agents = current.background_agents;
            save_state(&self.paths, &state)
        });
    }

    /// Register a new headless session.
    #[allow(dead_code)] // public API surface — used by daemon CLI consumers
    pub fn start_session(
        &mut self,
        description: &str,
        model: Option<String>,
        _working_dir: &Path,
    ) -> SessionId {
        let id = SessionId::new(format!("session-{}", uuid_short()));
        let log_path = self.paths.log_dir.join(format!("{id}.log"));

        let info = SessionInfo {
            id: id.clone(),
            status: SessionStatus::Pending,
            task_description: description.to_string(),
            started_at: SystemTime::now(),
            last_activity: SystemTime::now(),
            tokens_used: 0,
            model,
            worktree: None,
            log_path,
        };

        self.state.sessions.insert(id.clone(), info);
        self.persist();
        id
    }

    /// Update session status.
    #[allow(dead_code)] // public API surface — used by daemon CLI consumers
    pub fn update_session_status(&mut self, id: &str, status: SessionStatus) {
        if let Some(session) = self.state.sessions.get_mut(id) {
            session.status = status;
            session.last_activity = SystemTime::now();
            self.persist();
        }
    }

    /// Add a cron job.
    pub fn add_cron_job(
        &mut self,
        schedule: CronSchedule,
        description: &str,
        command: &str,
    ) -> String {
        let id = format!("cron-{}", uuid_short());
        let job = CronJob {
            id: id.clone(),
            schedule,
            description: description.to_string(),
            command: command.to_string(),
            enabled: true,
            last_run: None,
            created_at: SystemTime::now(),
        };
        self.state.cron_jobs.push(job);
        self.persist();
        id
    }

    /// Remove a cron job by ID. Returns whether anything was removed.
    pub fn remove_cron_job(&mut self, id: &str) -> bool {
        let before = self.state.cron_jobs.len();
        self.state.cron_jobs.retain(|j| j.id != id);
        let removed = self.state.cron_jobs.len() < before;
        if removed {
            self.persist();
        }
        removed
    }

    /// Look up a cron job by id.
    pub fn cron_by_id(&self, id: &str) -> Option<&CronJob> {
        self.state.cron_jobs.iter().find(|j| j.id == id)
    }

    /// Schedule a one-shot wakeup. Returns the wakeup id.
    pub fn schedule_wakeup(&mut self, delay: Duration, prompt: &str, reason: &str) -> String {
        let id = format!("wake-{}", uuid_short());
        let now = SystemTime::now();
        let wake = ScheduledWakeup {
            id: id.clone(),
            prompt: prompt.to_string(),
            reason: reason.to_string(),
            fire_at: now + delay,
            created_at: now,
        };
        self.state.wakeups.push(wake);
        self.persist();
        id
    }

    /// Drain wakeups whose `fire_at` is <= `now`. Each drained wakeup is
    /// also appended to `fired_wakeups` (capped at 100 entries) so the
    /// caller can replay them after a daemon restart without losing the
    /// audit trail.
    pub fn drain_due_wakeups(&mut self, now: SystemTime) -> Vec<ScheduledWakeup> {
        let mut due = Vec::new();
        let mut keep = Vec::with_capacity(self.state.wakeups.len());
        for w in std::mem::take(&mut self.state.wakeups) {
            if w.fire_at <= now {
                due.push(w);
            } else {
                keep.push(w);
            }
        }
        self.state.wakeups = keep;

        if !due.is_empty() {
            self.state.fired_wakeups.extend(due.iter().cloned());
            // Bound the audit log so the state file doesn't grow without bound.
            const MAX_FIRED: usize = 100;
            let len = self.state.fired_wakeups.len();
            if len > MAX_FIRED {
                self.state.fired_wakeups.drain(0..len - MAX_FIRED);
            }
            self.persist();
        }
        due
    }

    /// Tick the cron scheduler. Returns the IDs of jobs whose schedules
    /// fired in this tick (their `last_run` has been advanced).
    pub fn tick_cron(&mut self, now: SystemTime) -> Vec<String> {
        self.tick_cron_with_quiet_check(now, false)
    }

    /// Like `tick_cron` but skips all jobs when `is_quiet_hours` is true.
    /// The caller (engine / daemon loop) is responsible for evaluating quiet
    /// hours from the loaded config via `jfc_config::quiet_hours::is_quiet_hours`.
    pub fn tick_cron_with_quiet_check(
        &mut self,
        now: SystemTime,
        is_quiet_hours: bool,
    ) -> Vec<String> {
        if is_quiet_hours {
            tracing::debug!(
                target: "jfc::daemon::cron",
                "quiet hours active — skipping cron tick"
            );
            return Vec::new();
        }
        let mut fired = Vec::new();
        for job in &mut self.state.cron_jobs {
            if !job.enabled {
                continue;
            }
            if should_fire_cron(job, now) {
                job.last_run = Some(now);
                fired.push(job.id.clone());
            }
        }
        if !fired.is_empty() {
            self.persist();
        }
        fired
    }

    /// Manually fire a cron job (advance `last_run` and return it).
    /// Returns `None` if the id doesn't match any registered job.
    pub fn fire_cron(&mut self, id: &str, now: SystemTime) -> Option<CronJob> {
        let job = self.state.cron_jobs.iter_mut().find(|j| j.id == id)?;
        job.last_run = Some(now);
        let snapshot = job.clone();
        self.persist();
        Some(snapshot)
    }

    /// Clean up completed sessions older than `max_age`.
    #[allow(dead_code)] // public API surface — used by daemon CLI consumers
    pub fn cleanup_old_sessions(&mut self, max_age: Duration) {
        let cutoff = SystemTime::now().checked_sub(max_age).unwrap_or(UNIX_EPOCH);
        self.state.sessions.retain(|_, s| {
            if matches!(
                s.status,
                SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled
            ) {
                s.last_activity > cutoff
            } else {
                true
            }
        });
        self.persist();
    }

    // ─── Worker tracking ────────────────────────────────────────────────

    /// Register an active background worker with the daemon.
    pub fn register_worker(&mut self, info: WorkerInfo) {
        self.last_activity = Instant::now();
        self.workers.push(info);
    }

    /// Deregister a worker by PID. No-op if the PID isn't tracked.
    pub fn deregister_worker(&mut self, pid: u32) {
        self.workers.retain(|w| w.pid != pid);
        self.last_activity = Instant::now();
    }

    /// View all currently-tracked active workers.
    pub fn active_workers(&self) -> &[WorkerInfo] {
        &self.workers
    }

    /// Whether the daemon has any active workers.
    pub fn has_active_workers(&self) -> bool {
        !self.workers.is_empty()
    }

    /// Touch activity timestamp (called on any meaningful work).
    pub fn touch_activity(&mut self) {
        self.last_activity = Instant::now();
    }

    /// Returns `true` if the daemon has been idle long enough to exit
    /// and has no active workers blocking shutdown.
    pub fn should_idle_exit(&self) -> bool {
        if self.has_active_workers() {
            return false;
        }
        let timeout = idle_timeout();
        if timeout.is_zero() {
            // Idle-exit disabled.
            return false;
        }
        self.last_activity.elapsed() >= timeout
    }
}

/// Read the idle-exit timeout from the environment. Zero disables idle-exit.
fn idle_timeout() -> Duration {
    std::env::var("JFC_DAEMON_IDLE_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(DEFAULT_IDLE_TIMEOUT)
}

pub(super) fn uuid_short() -> String {
    // If the system clock is set before UNIX_EPOCH (extreme skew or pre-1970
    // hardware clocks), saturate to ZERO. The resulting id collides with
    // anything else generated under that condition, but we prefer that to a
    // panic in id-generation paths.
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    format!("{:x}{:04x}", t.as_secs() & 0xFFFF_FFFF, t.subsec_millis())
}

// ─────────────────────────────────────────────────────────────────────────────
// CLI entry points (called from main.rs `jfc daemon …`)
// ─────────────────────────────────────────────────────────────────────────────

/// `jfc daemon start` — write PID, run cron + wakeup poll loop forever.
///
/// The loop:
/// 1. Wakes once a second.
/// 2. Calls `tick_cron` and runs the matching commands via `tokio::process`.
/// 3. Drains due wakeups and prints them to stdout (a downstream UI
///    consumer would pipe these into the conversation).
///
/// Cron firing is single-process here (the daemon shells out commands
/// itself rather than spawning a separate worker). For a production
/// deployment you'd want each fire to dispatch to a worker pool; that
/// plumbing isn't in scope for this milestone.
pub async fn run_daemon(paths: DaemonPaths) -> std::io::Result<()> {
    paths.ensure_dirs()?;

    if let Some(pid) = is_daemon_running(&paths) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            format!("daemon already running (pid {pid})"),
        ));
    }

    write_pid_file(&paths)?;
    let mut daemon = Daemon::new(&paths.base_dir)?;
    daemon.state.pid = std::process::id();
    daemon.state.started_at = SystemTime::now();
    daemon.state.runtime.restart_requested = false;
    daemon.state.runtime.restart_reason = None;
    let _ = refresh_runtime_info(&mut daemon);
    daemon.persist();

    tracing::info!(
        target: "jfc::daemon",
        pid = daemon.state.pid,
        state_file = %paths.state_file.display(),
        "daemon started"
    );

    // SIGTERM / SIGINT — best-effort graceful shutdown. On non-unix
    // platforms only SIGINT (ctrl_c) is wired.
    let shutdown = shutdown_signal();
    tokio::pin!(shutdown);

    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut restart_requested = false;

    // The per-tick work is now an ordered roster of services (see
    // `crate::svcs::DaemonServices`) rather than an inline block. The roster
    // preserves the historical order exactly — reconcile → runtime-info →
    // memory → control → worker-sync → cron → wakeup → idle-check — and
    // signals loop exits via `TickOutcome`.
    let mut services = crate::svcs::DaemonServices::new();

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = SystemTime::now();
                match services.run_tick(&mut daemon, now).await {
                    crate::svcs::TickOutcome::Continue => {}
                    crate::svcs::TickOutcome::Restart => {
                        restart_requested = true;
                        break;
                    }
                    crate::svcs::TickOutcome::IdleExit => {
                        tracing::info!(
                            target: "jfc::daemon",
                            idle_secs = daemon.last_activity.elapsed().as_secs(),
                            "idle timeout reached with no active workers, exiting"
                        );
                        break;
                    }
                }
            }
            _ = &mut shutdown => {
                tracing::info!(target: "jfc::daemon", "shutdown signal, exiting");
                break;
            }
        }
    }

    // Join any finished worker-reaper threads so we don't exit while a
    // child.wait() is mid-flight (would otherwise leave a zombie).
    join_worker_reapers();

    if restart_requested {
        remove_pid_file(&paths);
        daemon.persist();
        if let Err(err) = spawn_replacement_daemon(&paths, &daemon.state) {
            tracing::error!(
                target: "jfc::daemon",
                error = %err,
                "failed to spawn replacement daemon during takeover"
            );
        }
        return Ok(());
    }

    remove_pid_file(&paths);
    daemon.persist();
    Ok(())
}

fn spawn_replacement_daemon(paths: &DaemonPaths, state: &DaemonState) -> std::io::Result<()> {
    let exe = state
        .runtime
        .worker_exe
        .clone()
        .or_else(|| std::env::current_exe().ok())
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "daemon binary not found")
        })?;
    if !exe.is_file() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("daemon binary does not exist: {}", exe.display()),
        ));
    }

    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .arg("start")
        .env("JFC_DAEMON_TAKEOVER_PARENT", std::process::id().to_string())
        .env("JFC_DAEMON_TAKEOVER_STATE", &paths.state_file)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }

    let child = cmd.spawn()?;
    tracing::warn!(
        target: "jfc::daemon",
        replacement_pid = child.id(),
        "spawned replacement daemon during binary takeover"
    );
    Ok(())
}

/// Future that resolves on SIGTERM (unix) or ctrl_c (cross-platform),
/// whichever comes first. Hoisted out of `run_daemon` so the
/// `tokio::select!` body stays cfg-clean.
async fn shutdown_signal() {
    #[cfg(unix)]
    {
        let mut term =
            match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
                Ok(s) => s,
                Err(_) => {
                    let _ = tokio::signal::ctrl_c().await;
                    return;
                }
            };
        tokio::select! {
            _ = term.recv() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}

/// Sync the daemon's in-memory worker roster from the persisted
/// `background_agents` state. Agents with `Running` status and a live PID
/// are registered; previously-tracked workers whose PID vanished or moved
/// to terminal status are deregistered.
pub(crate) fn sync_workers_from_state(daemon: &mut Daemon, state: &DaemonState) {
    use super::state::BackgroundAgentStatus;

    // Collect PIDs that are running per persisted state.
    let running_pids: std::collections::HashSet<u32> = state
        .background_agents
        .values()
        .filter(|a| a.status == BackgroundAgentStatus::Running)
        .filter_map(|a| a.pid)
        .collect();

    // Remove workers whose PIDs are no longer running in persisted state.
    let had_workers = daemon.has_active_workers();
    daemon.workers.retain(|w| running_pids.contains(&w.pid));

    // Register any new running agents we don't already track.
    let tracked_pids: std::collections::HashSet<u32> =
        daemon.workers.iter().map(|w| w.pid).collect();

    for agent in state.background_agents.values() {
        if agent.status != BackgroundAgentStatus::Running {
            continue;
        }
        let Some(pid) = agent.pid else { continue };
        if tracked_pids.contains(&pid) {
            continue;
        }
        daemon.workers.push(WorkerInfo {
            label: agent.description.clone(),
            pid,
            cwd: agent
                .worktree_path
                .clone()
                .unwrap_or_else(|| PathBuf::from(".")),
            started_at: Instant::now(),
        });
    }

    // If we gained or still have workers, touch activity so idle-exit
    // doesn't fire while work is in progress.
    if daemon.has_active_workers() {
        daemon.touch_activity();
    } else if had_workers {
        // Workers just finished — reset activity so idle timer starts fresh.
        daemon.touch_activity();
    }
}

/// `jfc daemon stop` — send SIGTERM to the PID file and clean up.
pub fn stop_daemon(paths: &DaemonPaths) -> std::io::Result<()> {
    let pid = match is_daemon_running(paths) {
        Some(p) => p,
        None => {
            // Stale or missing — wipe the file so subsequent starts don't fail.
            remove_pid_file(paths);
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no running daemon",
            ));
        }
    };

    #[cfg(unix)]
    {
        let result = std::process::Command::new("kill")
            .args(["-TERM", &pid.to_string()])
            .output()?;
        if !result.status.success() {
            return Err(std::io::Error::other(format!(
                "kill failed: {}",
                String::from_utf8_lossy(&result.stderr)
            )));
        }
    }

    Ok(())
}

/// `jfc daemon status` — render a one-paragraph status string.
pub fn status_string(paths: &DaemonPaths) -> String {
    use super::state::BackgroundAgentStatus;

    let running = is_daemon_running(paths);
    let state = reconcile_background_agents(paths).unwrap_or_default();
    let uptime = SystemTime::now()
        .duration_since(state.started_at)
        .unwrap_or_default()
        .as_secs();

    let mut s = String::new();
    s.push_str(&format!(
        "daemon: {}\n",
        match running {
            Some(pid) => format!("running (pid {pid}, uptime {uptime}s)"),
            None => "stopped".into(),
        }
    ));
    s.push_str(&format!(
        "sessions: {} ({} active)\n",
        state.sessions.len(),
        state
            .sessions
            .values()
            .filter(|s| matches!(s.status, SessionStatus::Running | SessionStatus::Idle))
            .count()
    ));
    s.push_str(&format!(
        "cron jobs: {} (enabled {})\n",
        state.cron_jobs.len(),
        state.cron_jobs.iter().filter(|j| j.enabled).count()
    ));
    s.push_str(&format!(
        "scheduled wakeups: {} pending, {} fired\n",
        state.wakeups.len(),
        state.fired_wakeups.len()
    ));
    if !state.worker_controls.is_empty() {
        let pending = state
            .worker_controls
            .iter()
            .filter(|rec| {
                matches!(
                    rec.status,
                    super::state::WorkerControlStatus::Pending
                        | super::state::WorkerControlStatus::Running
                )
            })
            .count();
        s.push_str(&format!(
            "worker controls: {} total, {} active\n",
            state.worker_controls.len(),
            pending
        ));
    }
    if state.runtime.worker_exe.is_some()
        || state.runtime.spare_ready
        || state.runtime.restart_requested
        || state.runtime.low_memory_retire_count > 0
    {
        let worker_exe = state
            .runtime
            .worker_exe
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "(unknown)".to_owned());
        s.push_str(&format!(
            "runtime: worker={} spare_ready={} restart_requested={} low_memory_retirements={}\n",
            worker_exe,
            state.runtime.spare_ready,
            state.runtime.restart_requested,
            state.runtime.low_memory_retire_count
        ));
        if let Some(reason) = state.runtime.restart_reason.as_deref() {
            s.push_str(&format!("  restart reason: {reason}\n"));
        }
    }

    let active_agents: Vec<_> = state
        .background_agents
        .values()
        .filter(|a| a.status == BackgroundAgentStatus::Running)
        .collect();
    s.push_str(&format!(
        "background agents: {} ({} active)\n",
        state.background_agents.len(),
        active_agents.len()
    ));

    if !active_agents.is_empty() {
        s.push_str(&format!(
            "  {} bg worker(s) running:\n",
            active_agents.len()
        ));
        for a in &active_agents {
            let cwd = a
                .worktree_path
                .as_deref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| ".".into());
            s.push_str(&format!(
                "    `{}` (pid {}) in {}\n",
                a.description,
                a.pid.unwrap_or(0),
                cwd
            ));
        }
    }
    s
}

pub(crate) fn refresh_runtime_info(daemon: &mut Daemon) -> std::io::Result<bool> {
    let now = SystemTime::now();
    if daemon.state.runtime.restart_requested {
        daemon.persist();
        return Ok(true);
    }
    let worker_exe = resolve_worker_exe(None)?;
    let worker_exe_mtime = std::fs::metadata(&worker_exe)
        .and_then(|m| m.modified())
        .ok();
    let previous_exe = daemon.state.runtime.worker_exe.clone();
    let previous_mtime = daemon.state.runtime.worker_exe_mtime;

    daemon.state.runtime.worker_exe = Some(worker_exe.clone());
    daemon.state.runtime.worker_exe_mtime = worker_exe_mtime;
    if daemon_spare_enabled() {
        daemon.state.runtime.spare_ready = worker_exe.is_file();
        daemon.state.runtime.spare_checked_at = Some(now);
    }

    let changed = previous_exe
        .as_ref()
        .is_some_and(|previous| previous != &worker_exe)
        || (previous_mtime.is_some() && previous_mtime != worker_exe_mtime);
    if changed && daemon_restart_on_upgrade_enabled() {
        daemon.state.runtime.restart_requested = true;
        daemon.state.runtime.restart_reason = Some(format!(
            "worker binary changed from {} to {}",
            previous_exe
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "(unknown)".to_owned()),
            worker_exe.display()
        ));
        daemon.persist();
        return Ok(true);
    }
    daemon.persist();
    Ok(false)
}

pub(crate) fn maybe_retire_low_memory_worker(
    paths: &DaemonPaths,
    daemon: &mut Daemon,
) -> std::io::Result<bool> {
    let Some(threshold_mb) = std::env::var("JFC_DAEMON_LOW_MEM_RETIRE_MB")
        .ok()
        .and_then(|v| v.parse::<u64>().ok())
        .filter(|v| *v > 0)
    else {
        return Ok(false);
    };
    let Some(available) = available_memory_mb() else {
        return Ok(false);
    };
    if available >= threshold_mb {
        return Ok(false);
    }

    let retired = with_state_lock(paths, || -> std::io::Result<Option<(String, PathBuf)>> {
        let mut state = load_state_for_update(paths)?;
        let Some(agent) = state
            .background_agents
            .values_mut()
            .filter(|agent| {
                agent.status == BackgroundAgentStatus::Running && !agent.cancel_requested
            })
            .min_by_key(|agent| agent.started_at)
        else {
            return Ok(None);
        };
        agent.cancel_requested = true;
        agent.updated_at = SystemTime::now();
        agent.error = Some(format!(
            "low-memory retirement requested: MemAvailable={available}MB below threshold={threshold_mb}MB"
        ));
        let id = agent.id.clone();
        let log_path = agent.log_path.clone();
        state.runtime.low_memory_retire_count =
            state.runtime.low_memory_retire_count.saturating_add(1);
        save_state(paths, &state)?;
        daemon.state = state;
        Ok(Some((id, log_path)))
    })?;
    if let Some((id, log_path)) = retired {
        append_log_line(
            &log_path,
            &format!(
                "[retire-requested] low memory: MemAvailable={available}MB threshold={threshold_mb}MB"
            ),
        );
        tracing::warn!(
            target: "jfc::daemon",
            agent_id = %id,
            available_mb = available,
            threshold_mb,
            "requested low-memory worker retirement"
        );
        return Ok(true);
    }
    Ok(false)
}

fn daemon_spare_enabled() -> bool {
    std::env::var("JFC_DAEMON_SPARE_ENABLE")
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

fn daemon_restart_on_upgrade_enabled() -> bool {
    std::env::var("JFC_DAEMON_RESTART_ON_UPGRADE")
        .or_else(|_| std::env::var("JFC_DAEMON_SELF_RESTART_ON_UPGRADE"))
        .map(|v| matches!(v.as_str(), "1" | "true" | "yes" | "on"))
        .unwrap_or(false)
}

/// `jfc daemon list` — render cron jobs + pending wakeups.
pub fn list_string(paths: &DaemonPaths) -> String {
    let state = reconcile_background_agents(paths).unwrap_or_default();
    let mut s = String::new();
    s.push_str("cron jobs:\n");
    if state.cron_jobs.is_empty() {
        s.push_str("  (none)\n");
    }
    for j in &state.cron_jobs {
        s.push_str(&format!(
            "  {} [{}] {} :: {}\n",
            j.id,
            if j.enabled { "on" } else { "off" },
            describe_schedule(&j.schedule),
            j.command
        ));
    }
    s.push_str("scheduled wakeups:\n");
    if state.wakeups.is_empty() {
        s.push_str("  (none)\n");
    }
    for w in &state.wakeups {
        let in_secs = w
            .fire_at
            .duration_since(SystemTime::now())
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);
        s.push_str(&format!(
            "  {} fires in {}s — {} :: {}\n",
            w.id, in_secs, w.reason, w.prompt
        ));
    }
    s.push_str("background agents:\n");
    if state.background_agents.is_empty() {
        s.push_str("  (none)\n");
    }
    let mut agents: Vec<_> = state.background_agents.values().collect();
    agents.sort_by_key(|a| a.started_at);
    agents.reverse();
    for a in agents.iter().take(20) {
        s.push_str(&format!(
            "  {} [{:?}] tools={} tokens={} :: {}\n",
            a.id,
            a.status,
            a.tool_use_count,
            a.latest_input_tokens
                .saturating_add(a.cumulative_output_tokens),
            a.description
        ));
    }
    s
}

/// `jfc daemon fire <id>` — manually fire a cron job once.
pub async fn fire_cron_cli(paths: &DaemonPaths, id: &str) -> std::io::Result<String> {
    let mut daemon = Daemon::new(&paths.base_dir)?;
    let now = SystemTime::now();
    let job = daemon.fire_cron(id, now).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::NotFound, format!("no cron job `{id}`"))
    })?;
    run_cron_command(&job).await?;
    Ok(format!("fired {} ({})", job.id, job.command))
}
