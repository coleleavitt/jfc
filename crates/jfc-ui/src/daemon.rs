//! Fleet daemon — persistent headless agent management.
//!
//! Implements a background daemon process that manages multiple jfc sessions:
//! - Daemonize (fork to background, write PID file)
//! - Session registry (track active/idle/completed sessions)
//! - Cron scheduling (periodic task execution)
//! - Health monitoring (heartbeat, stall detection)
//! - Unix socket API for control (start/stop/status/list)
//! - Log rotation
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │                  Daemon Process                   │
//! ├─────────────┬───────────────┬───────────────────┤
//! │  Session 1  │  Session 2    │  Session N ...    │
//! │  (idle)     │  (running)    │  (scheduled)      │
//! ├─────────────┴───────────────┴───────────────────┤
//! │              Cron Scheduler                       │
//! │              Health Monitor                       │
//! │              Unix Socket Server                   │
//! └─────────────────────────────────────────────────┘
//! ```
//!
//! # Usage
//!
//! ```bash
//! jfc daemon start          # Start the daemon
//! jfc daemon stop           # Stop the daemon
//! jfc daemon status         # Show daemon + session status
//! jfc daemon run <task>     # Schedule a headless task
//! jfc daemon list           # List active sessions
//! jfc daemon logs [id]      # Tail session logs
//! jfc daemon cron add ...   # Add a cron job
//! jfc daemon cron list      # List cron jobs
//! jfc daemon cron rm <id>   # Remove a cron job
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Unique session identifier.
pub type SessionId = String;

/// Daemon state — persisted to disk for crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    pub pid: u32,
    pub started_at: SystemTime,
    pub sessions: HashMap<SessionId, SessionInfo>,
    pub cron_jobs: Vec<CronJob>,
}

/// Information about a managed session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub id: SessionId,
    pub status: SessionStatus,
    pub task_description: String,
    pub started_at: SystemTime,
    pub last_activity: SystemTime,
    pub tokens_used: usize,
    pub model: Option<String>,
    pub worktree: Option<String>,
    pub log_path: PathBuf,
}

/// Session lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionStatus {
    /// Queued, waiting to start.
    Pending,
    /// Actively running (model is generating or tools are executing).
    Running,
    /// Waiting for input (teammate idle, permission request, etc).
    Idle,
    /// Completed successfully.
    Completed,
    /// Failed with an error.
    Failed,
    /// Cancelled by user/system.
    Cancelled,
}

/// A cron-scheduled recurring task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub schedule: CronSchedule,
    pub task_description: String,
    pub model: Option<String>,
    pub working_dir: PathBuf,
    pub enabled: bool,
    pub last_run: Option<SystemTime>,
    pub next_run: Option<SystemTime>,
    pub created_at: SystemTime,
}

/// Simplified cron schedule (not full crontab syntax — interval-based).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum CronSchedule {
    /// Run every N minutes.
    Every { minutes: u32 },
    /// Run every N hours.
    EveryHours { hours: u32 },
    /// Run daily at a specific hour (0-23).
    DailyAt { hour: u8 },
    /// Run on specific weekdays at a specific hour.
    WeeklyAt { days: Vec<Weekday>, hour: u8 },
}

/// Day of week.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Weekday {
    Mon, Tue, Wed, Thu, Fri, Sat, Sun,
}

/// Commands sent to the daemon via Unix socket.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonCommand {
    /// Get daemon status.
    Status,
    /// List active sessions.
    ListSessions,
    /// Run a new headless task.
    RunTask {
        description: String,
        model: Option<String>,
        working_dir: PathBuf,
    },
    /// Stop a specific session.
    StopSession { id: SessionId },
    /// Get session logs.
    GetLogs { id: SessionId, tail: usize },
    /// Add a cron job.
    AddCron {
        schedule: CronSchedule,
        description: String,
        model: Option<String>,
        working_dir: PathBuf,
    },
    /// List cron jobs.
    ListCrons,
    /// Remove a cron job.
    RemoveCron { id: String },
    /// Shutdown the daemon.
    Shutdown,
}

/// Response from the daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonResponse {
    /// Daemon status info.
    Status {
        pid: u32,
        uptime_secs: u64,
        active_sessions: usize,
        total_sessions: usize,
        cron_jobs: usize,
    },
    /// List of sessions.
    Sessions(Vec<SessionInfo>),
    /// Task accepted.
    TaskStarted { id: SessionId },
    /// Session stopped.
    SessionStopped { id: SessionId },
    /// Log output.
    Logs { id: SessionId, lines: Vec<String> },
    /// Cron jobs list.
    CronJobs(Vec<CronJob>),
    /// Cron job added.
    CronAdded { id: String },
    /// Cron job removed.
    CronRemoved { id: String },
    /// Generic success.
    Ok,
    /// Error.
    Error { message: String },
}

// ─────────────────────────────────────────────────────────────────────────────
// Daemon Process Management
// ─────────────────────────────────────────────────────────────────────────────

/// Paths used by the daemon.
pub struct DaemonPaths {
    /// Base directory (~/.jfc/daemon/)
    pub base_dir: PathBuf,
    /// PID file (~/.jfc/daemon/daemon.pid)
    pub pid_file: PathBuf,
    /// State file (~/.jfc/daemon/state.json)
    pub state_file: PathBuf,
    /// Unix socket (~/.jfc/daemon/daemon.sock)
    pub socket_path: PathBuf,
    /// Log directory (~/.jfc/daemon/logs/)
    pub log_dir: PathBuf,
}

impl DaemonPaths {
    /// Create daemon paths from the jfc config directory.
    pub fn new(config_dir: &Path) -> Self {
        let base_dir = config_dir.join("daemon");
        Self {
            pid_file: base_dir.join("daemon.pid"),
            state_file: base_dir.join("state.json"),
            socket_path: base_dir.join("daemon.sock"),
            log_dir: base_dir.join("logs"),
            base_dir,
        }
    }

    /// Ensure all directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.base_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }
}

/// Check if daemon is running by reading PID file and checking process.
pub fn is_daemon_running(paths: &DaemonPaths) -> Option<u32> {
    let pid_str = std::fs::read_to_string(&paths.pid_file).ok()?;
    let pid: u32 = pid_str.trim().parse().ok()?;

    // Check if process is alive
    #[cfg(unix)]
    {
        use std::process::Command;
        let result = Command::new("kill")
            .args(["-0", &pid.to_string()])
            .output()
            .ok()?;
        if result.status.success() {
            return Some(pid);
        }
    }

    #[cfg(not(unix))]
    {
        // On non-Unix, assume alive if PID file exists
        return Some(pid);
    }

    None
}

/// Write PID file for the current process.
pub fn write_pid_file(paths: &DaemonPaths) -> std::io::Result<()> {
    std::fs::write(&paths.pid_file, std::process::id().to_string())
}

/// Remove PID file.
pub fn remove_pid_file(paths: &DaemonPaths) {
    let _ = std::fs::remove_file(&paths.pid_file);
}

/// Load daemon state from disk.
pub fn load_state(paths: &DaemonPaths) -> Option<DaemonState> {
    let data = std::fs::read_to_string(&paths.state_file).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save daemon state to disk.
pub fn save_state(paths: &DaemonPaths, state: &DaemonState) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    std::fs::write(&paths.state_file, json)
}

// ─────────────────────────────────────────────────────────────────────────────
// Daemon Core
// ─────────────────────────────────────────────────────────────────────────────

/// The daemon runtime — manages sessions, cron, and the control socket.
pub struct Daemon {
    pub paths: DaemonPaths,
    pub state: DaemonState,
    shutdown_tx: Option<mpsc::Sender<()>>,
}

impl Daemon {
    /// Create a new daemon instance.
    pub fn new(config_dir: &Path) -> std::io::Result<Self> {
        let paths = DaemonPaths::new(config_dir);
        paths.ensure_dirs()?;

        let state = load_state(&paths).unwrap_or(DaemonState {
            pid: std::process::id(),
            started_at: SystemTime::now(),
            sessions: HashMap::new(),
            cron_jobs: Vec::new(),
        });

        Ok(Self {
            paths,
            state,
            shutdown_tx: None,
        })
    }

    /// Start a new headless session.
    pub fn start_session(&mut self, description: &str, model: Option<String>, working_dir: &Path) -> SessionId {
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
    pub fn update_session_status(&mut self, id: &str, status: SessionStatus) {
        if let Some(session) = self.state.sessions.get_mut(id) {
            session.status = status;
            session.last_activity = SystemTime::now();
            self.persist();
        }
    }

    /// Add a cron job.
    pub fn add_cron_job(&mut self, schedule: CronSchedule, description: &str, model: Option<String>, working_dir: &Path) -> String {
        let id = format!("cron-{}", uuid_short());
        let job = CronJob {
            id: id.clone(),
            schedule,
            task_description: description.to_string(),
            model,
            working_dir: working_dir.to_path_buf(),
            enabled: true,
            last_run: None,
            next_run: None,
            created_at: SystemTime::now(),
        };
        self.state.cron_jobs.push(job);
        self.persist();
        id
    }

    /// Remove a cron job by ID.
    pub fn remove_cron_job(&mut self, id: &str) -> bool {
        let before = self.state.cron_jobs.len();
        self.state.cron_jobs.retain(|j| j.id != id);
        let removed = self.state.cron_jobs.len() < before;
        if removed {
            self.persist();
        }
        removed
    }

    /// Handle a command from the control socket.
    pub fn handle_command(&mut self, cmd: DaemonCommand) -> DaemonResponse {
        match cmd {
            DaemonCommand::Status => {
                let uptime = SystemTime::now()
                    .duration_since(self.state.started_at)
                    .unwrap_or_default()
                    .as_secs();
                let active = self.state.sessions.values()
                    .filter(|s| matches!(s.status, SessionStatus::Running | SessionStatus::Idle))
                    .count();
                DaemonResponse::Status {
                    pid: self.state.pid,
                    uptime_secs: uptime,
                    active_sessions: active,
                    total_sessions: self.state.sessions.len(),
                    cron_jobs: self.state.cron_jobs.len(),
                }
            }
            DaemonCommand::ListSessions => {
                let sessions: Vec<SessionInfo> = self.state.sessions.values().cloned().collect();
                DaemonResponse::Sessions(sessions)
            }
            DaemonCommand::RunTask { description, model, working_dir } => {
                let id = self.start_session(&description, model, &working_dir);
                DaemonResponse::TaskStarted { id }
            }
            DaemonCommand::StopSession { id } => {
                self.update_session_status(&id, SessionStatus::Cancelled);
                DaemonResponse::SessionStopped { id }
            }
            DaemonCommand::GetLogs { id, tail } => {
                let lines = if let Some(session) = self.state.sessions.get(&id) {
                    read_last_lines(&session.log_path, tail)
                } else {
                    vec!["Session not found".to_string()]
                };
                DaemonResponse::Logs { id, lines }
            }
            DaemonCommand::AddCron { schedule, description, model, working_dir } => {
                let id = self.add_cron_job(schedule, &description, model, &working_dir);
                DaemonResponse::CronAdded { id }
            }
            DaemonCommand::ListCrons => {
                DaemonResponse::CronJobs(self.state.cron_jobs.clone())
            }
            DaemonCommand::RemoveCron { id } => {
                if self.remove_cron_job(&id) {
                    DaemonResponse::CronRemoved { id }
                } else {
                    DaemonResponse::Error { message: format!("Cron job '{id}' not found") }
                }
            }
            DaemonCommand::Shutdown => {
                if let Some(tx) = &self.shutdown_tx {
                    let _ = tx.try_send(());
                }
                DaemonResponse::Ok
            }
        }
    }

    /// Persist state to disk.
    fn persist(&self) {
        let _ = save_state(&self.paths, &self.state);
    }

    /// Check cron jobs and fire any that are due.
    pub fn tick_cron(&mut self) -> Vec<SessionId> {
        let now = SystemTime::now();
        let mut fired = Vec::new();

        for job in &mut self.state.cron_jobs {
            if !job.enabled {
                continue;
            }
            if should_fire_cron(job, now) {
                job.last_run = Some(now);
                // Start a session for this cron job
                let id = format!("session-{}", uuid_short());
                let log_path = self.paths.log_dir.join(format!("{id}.log"));
                let info = SessionInfo {
                    id: id.clone(),
                    status: SessionStatus::Pending,
                    task_description: format!("[cron:{}] {}", job.id, job.task_description),
                    started_at: now,
                    last_activity: now,
                    tokens_used: 0,
                    model: job.model.clone(),
                    worktree: None,
                    log_path,
                };
                self.state.sessions.insert(id.clone(), info);
                fired.push(id);
            }
        }

        if !fired.is_empty() {
            self.persist();
        }
        fired
    }

    /// Clean up completed sessions older than max_age.
    pub fn cleanup_old_sessions(&mut self, max_age: Duration) {
        let cutoff = SystemTime::now() - max_age;
        self.state.sessions.retain(|_, s| {
            if matches!(s.status, SessionStatus::Completed | SessionStatus::Failed | SessionStatus::Cancelled) {
                s.last_activity > cutoff
            } else {
                true
            }
        });
        self.persist();
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

fn uuid_short() -> String {
    use std::time::UNIX_EPOCH;
    let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
    format!("{:x}{:04x}", t.as_secs() & 0xFFFF_FFFF, t.subsec_millis())
}

fn should_fire_cron(job: &CronJob, now: SystemTime) -> bool {
    let Some(last) = job.last_run else {
        // Never run before — fire immediately.
        return true;
    };
    let elapsed = now.duration_since(last).unwrap_or_default();
    match &job.schedule {
        CronSchedule::Every { minutes } => elapsed >= Duration::from_secs(u64::from(*minutes) * 60),
        CronSchedule::EveryHours { hours } => elapsed >= Duration::from_secs(u64::from(*hours) * 3600),
        CronSchedule::DailyAt { .. } => elapsed >= Duration::from_secs(23 * 3600), // rough
        CronSchedule::WeeklyAt { .. } => elapsed >= Duration::from_secs(6 * 24 * 3600), // rough
    }
}

fn read_last_lines(path: &Path, n: usize) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec!["(log file not found)".to_string()];
    };
    content.lines().rev().take(n).map(String::from).collect::<Vec<_>>().into_iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_daemon() -> (Daemon, TempDir) {
        let tmp = TempDir::new().unwrap();
        let daemon = Daemon::new(tmp.path()).unwrap();
        (daemon, tmp)
    }

    #[test]
    fn test_start_session() {
        let (mut daemon, _tmp) = test_daemon();
        let id = daemon.start_session("test task", None, Path::new("/tmp"));
        assert!(id.starts_with("session-"));
        assert_eq!(daemon.state.sessions.len(), 1);
        assert_eq!(daemon.state.sessions[&id].status, SessionStatus::Pending);
    }

    #[test]
    fn test_update_session_status() {
        let (mut daemon, _tmp) = test_daemon();
        let id = daemon.start_session("test", None, Path::new("/tmp"));
        daemon.update_session_status(&id, SessionStatus::Running);
        assert_eq!(daemon.state.sessions[&id].status, SessionStatus::Running);
    }

    #[test]
    fn test_add_remove_cron() {
        let (mut daemon, _tmp) = test_daemon();
        let id = daemon.add_cron_job(
            CronSchedule::Every { minutes: 30 },
            "periodic check",
            None,
            Path::new("/tmp"),
        );
        assert_eq!(daemon.state.cron_jobs.len(), 1);
        assert!(daemon.remove_cron_job(&id));
        assert_eq!(daemon.state.cron_jobs.len(), 0);
    }

    #[test]
    fn test_handle_status_command() {
        let (mut daemon, _tmp) = test_daemon();
        daemon.start_session("t1", None, Path::new("/tmp"));
        let resp = daemon.handle_command(DaemonCommand::Status);
        match resp {
            DaemonResponse::Status { total_sessions, .. } => assert_eq!(total_sessions, 1),
            _ => panic!("expected Status response"),
        }
    }

    #[test]
    fn test_cron_fires_on_first_tick() {
        let (mut daemon, _tmp) = test_daemon();
        daemon.add_cron_job(
            CronSchedule::Every { minutes: 5 },
            "first tick",
            None,
            Path::new("/tmp"),
        );
        let fired = daemon.tick_cron();
        assert_eq!(fired.len(), 1);
        // Second tick shouldn't fire (not enough time elapsed)
        let fired2 = daemon.tick_cron();
        assert_eq!(fired2.len(), 0);
    }

    #[test]
    fn test_cleanup_old_sessions() {
        let (mut daemon, _tmp) = test_daemon();
        let id = daemon.start_session("old", None, Path::new("/tmp"));
        daemon.update_session_status(&id, SessionStatus::Completed);
        // With a 0-duration max_age, it should clean up
        daemon.cleanup_old_sessions(Duration::from_secs(0));
        assert!(daemon.state.sessions.is_empty());
    }

    #[test]
    fn test_persistence() {
        let tmp = TempDir::new().unwrap();
        {
            let mut daemon = Daemon::new(tmp.path()).unwrap();
            daemon.start_session("persisted", None, Path::new("/tmp"));
        }
        // Reload
        let daemon2 = Daemon::new(tmp.path()).unwrap();
        assert_eq!(daemon2.state.sessions.len(), 1);
    }

    #[test]
    fn test_daemon_paths() {
        let paths = DaemonPaths::new(Path::new("/home/user/.jfc"));
        assert_eq!(paths.base_dir, PathBuf::from("/home/user/.jfc/daemon"));
        assert_eq!(paths.pid_file, PathBuf::from("/home/user/.jfc/daemon/daemon.pid"));
        assert_eq!(paths.socket_path, PathBuf::from("/home/user/.jfc/daemon/daemon.sock"));
    }
}
