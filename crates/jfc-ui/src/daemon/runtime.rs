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

use std::path::Path;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use super::cron::{CronJob, CronSchedule, describe_schedule, run_cron_command, should_fire_cron};
use super::pid::{is_daemon_running, remove_pid_file, write_pid_file};
use super::reconcile::reconcile_background_agents;
use super::state::{
    DaemonPaths, DaemonState, ScheduledWakeup, SessionId, SessionInfo, SessionStatus, load_state,
    save_state,
};

/// In-memory daemon state + I/O paths.
pub struct Daemon {
    pub paths: DaemonPaths,
    pub state: DaemonState,
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

        Ok(Self { paths, state })
    }

    /// Persist current state to disk (best-effort).
    pub fn persist(&self) {
        let mut state = self.state.clone();
        if let Some(current) = load_state(&self.paths) {
            // Background workers update their roster/log metadata out-of-process.
            // Preserve that live subtree when the cron daemon persists its own
            // in-memory cron/wakeup/session state.
            state.background_agents = current.background_agents;
        }
        let _ = save_state(&self.paths, &state);
    }

    /// Register a new headless session.
    #[allow(dead_code)] // public API surface — used by daemon CLI consumers
    pub fn start_session(
        &mut self,
        description: &str,
        model: Option<String>,
        _working_dir: &Path,
    ) -> SessionId {
        let id = format!("session-{}", uuid_short());
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

    loop {
        tokio::select! {
            _ = interval.tick() => {
                let now = SystemTime::now();
                let _ = reconcile_background_agents(&paths);
                let fired = daemon.tick_cron(now);
                for id in fired {
                    if let Some(job) = daemon.cron_by_id(&id).cloned() {
                        tracing::info!(
                            target: "jfc::daemon",
                            cron_id = %job.id,
                            cmd = %job.command,
                            "cron firing"
                        );
                        let _ = run_cron_command(&job).await;
                    }
                }

                let wakes = daemon.drain_due_wakeups(now);
                for w in wakes {
                    tracing::info!(
                        target: "jfc::daemon",
                        wakeup_id = %w.id,
                        reason = %w.reason,
                        "wakeup firing"
                    );
                    println!("[wakeup {}] {} :: {}", w.id, w.reason, w.prompt);
                }
            }
            _ = &mut shutdown => {
                tracing::info!(target: "jfc::daemon", "shutdown signal, exiting");
                break;
            }
        }
    }

    remove_pid_file(&paths);
    daemon.persist();
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
    s.push_str(&format!(
        "background agents: {} ({} active)\n",
        state.background_agents.len(),
        state
            .background_agents
            .values()
            .filter(|a| !a.status.is_terminal())
            .count()
    ));
    s
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
