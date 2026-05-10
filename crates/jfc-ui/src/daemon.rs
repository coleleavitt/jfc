#![allow(dead_code, unused_imports, unused_variables)]
//! Fleet daemon — persistent headless agent management.
//!
//! Implements a background daemon process that manages multiple jfc sessions:
//! - Daemonize (write PID file, detach)
//! - Session registry (track active/idle/completed sessions)
//! - Cron scheduling (periodic task execution)
//! - Health monitoring (heartbeat, stall detection)
//! - Scheduled wakeups (one-shot reminders that re-fire after restarts)
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
//! │              Wakeup Scheduler                     │
//! │              Health Monitor                       │
//! └─────────────────────────────────────────────────┘
//! ```
//!
//! # Storage layout
//!
//! - PID file:   `~/.config/jfc/daemon.pid`
//! - State file: `~/.config/jfc/daemon-state.json`
//! - Log dir:    `~/.config/jfc/logs/daemon/`
//!
//! # CLI
//!
//! ```bash
//! jfc daemon start           # Fork to background, write PID, run cron loop
//! jfc daemon stop            # Send SIGTERM to PID file
//! jfc daemon status          # Show daemon + session status
//! jfc daemon list            # List cron jobs + scheduled wakeups
//! jfc daemon fire <id>       # Manually fire a cron job by id
//! ```

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ─────────────────────────────────────────────────────────────────────────────
// Types
// ─────────────────────────────────────────────────────────────────────────────

/// Unique session identifier.
pub type SessionId = String;

/// Daemon state — persisted to disk for crash recovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonState {
    /// PID of the daemon process (0 when state hasn't been claimed yet).
    #[serde(default)]
    pub pid: u32,
    /// When the daemon started. Defaults to UNIX epoch when missing on disk.
    #[serde(default = "epoch")]
    pub started_at: SystemTime,
    /// Active / completed sessions tracked by the daemon.
    #[serde(default)]
    pub sessions: HashMap<SessionId, SessionInfo>,
    /// Registered cron jobs.
    #[serde(default)]
    pub cron_jobs: Vec<CronJob>,
    /// Pending one-shot scheduled wakeups (not yet fired).
    #[serde(default)]
    pub wakeups: Vec<ScheduledWakeup>,
    /// Wakeups that have already fired — kept for replay/audit. Bounded.
    #[serde(default)]
    pub fired_wakeups: Vec<ScheduledWakeup>,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self {
            pid: 0,
            started_at: UNIX_EPOCH,
            sessions: HashMap::new(),
            cron_jobs: Vec::new(),
            wakeups: Vec::new(),
            fired_wakeups: Vec::new(),
        }
    }
}

fn epoch() -> SystemTime {
    UNIX_EPOCH
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
    Pending,
    Running,
    Idle,
    Completed,
    Failed,
    Cancelled,
}

/// A cron-scheduled recurring task.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CronJob {
    pub id: String,
    pub schedule: CronSchedule,
    /// Free-form human description ("nightly housekeeping").
    pub description: String,
    /// Shell command to execute when the job fires.
    pub command: String,
    pub enabled: bool,
    pub last_run: Option<SystemTime>,
    pub created_at: SystemTime,
}

/// Schedule expressions supported by the cron parser.
///
/// Mirrors the v132 `tengu_cron_*` syntax surface:
/// - `* * * * *` — five-field POSIX crontab (minute hour day month dow)
/// - `@hourly`   — alias for `0 * * * *`
/// - `@daily`    — alias for `0 0 * * *`
/// - `@weekly`   — alias for `0 0 * * 0`
/// - `@every 5m` / `@every 1h30m` — interval relative to last run
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CronSchedule {
    /// Five-field crontab. Field values are stored as-is; matching uses
    /// minute-resolution (the daemon polls every minute).
    Crontab {
        minute: CronField,
        hour: CronField,
        day: CronField,
        month: CronField,
        weekday: CronField,
    },
    /// Re-run when at least `period` has elapsed since `last_run`. Fires
    /// immediately when `last_run` is None.
    Every {
        #[serde(with = "duration_secs")]
        period: Duration,
    },
}

/// One field of a five-field crontab expression.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CronField {
    /// `*` — match anything.
    Any,
    /// Literal value (`5`).
    Exact(u32),
    /// `*/N` step — match values where `value % step == 0`.
    Step(u32),
}

impl CronField {
    fn matches(&self, value: u32) -> bool {
        match self {
            Self::Any => true,
            Self::Exact(v) => *v == value,
            Self::Step(step) => *step > 0 && value % step == 0,
        }
    }

    fn parse(s: &str) -> Result<Self, String> {
        if s == "*" {
            return Ok(Self::Any);
        }
        if let Some(rest) = s.strip_prefix("*/") {
            let n: u32 = rest.parse().map_err(|_| format!("bad step `{s}`"))?;
            if n == 0 {
                return Err(format!("step must be > 0 (`{s}`)"));
            }
            return Ok(Self::Step(n));
        }
        let n: u32 = s.parse().map_err(|_| format!("bad cron field `{s}`"))?;
        Ok(Self::Exact(n))
    }
}

mod duration_secs {
    use serde::{Deserialize, Deserializer, Serializer};
    use std::time::Duration;

    pub fn serialize<S: Serializer>(d: &Duration, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u64(d.as_secs())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
        let s = u64::deserialize(d)?;
        Ok(Duration::from_secs(s))
    }
}

/// Parse a schedule expression into a `CronSchedule`.
///
/// Accepted forms:
/// - `"* * * * *"` (and any 5-field variant where each field is `*`,
///   a literal integer, or `*/N`)
/// - `"@hourly"`, `"@daily"`, `"@weekly"`
/// - `"@every <duration>"` where duration uses `Ns/Nm/Nh/Nd` chunks
///   (e.g. `5m`, `1h30m`, `2d`).
pub fn parse_schedule(expr: &str) -> Result<CronSchedule, String> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return Err("empty schedule".into());
    }

    // Aliases.
    let aliased = match trimmed {
        "@hourly" => Some("0 * * * *"),
        "@daily" | "@midnight" => Some("0 0 * * *"),
        "@weekly" => Some("0 0 * * 0"),
        "@monthly" => Some("0 0 1 * *"),
        _ => None,
    };
    if let Some(replacement) = aliased {
        return parse_schedule(replacement);
    }

    // @every N{s,m,h,d}
    if let Some(rest) = trimmed.strip_prefix("@every ") {
        let period = parse_duration_spec(rest.trim())?;
        if period.is_zero() {
            return Err("@every period must be > 0".into());
        }
        return Ok(CronSchedule::Every { period });
    }

    // Five-field crontab.
    let fields: Vec<&str> = trimmed.split_whitespace().collect();
    if fields.len() != 5 {
        return Err(format!(
            "expected 5 cron fields or `@<alias>`, got `{expr}`"
        ));
    }
    Ok(CronSchedule::Crontab {
        minute: CronField::parse(fields[0])?,
        hour: CronField::parse(fields[1])?,
        day: CronField::parse(fields[2])?,
        month: CronField::parse(fields[3])?,
        weekday: CronField::parse(fields[4])?,
    })
}

/// Parse `"5m"`, `"1h30m"`, `"2d"`, `"45s"` etc. into a `Duration`.
fn parse_duration_spec(s: &str) -> Result<Duration, String> {
    let mut total = Duration::ZERO;
    let mut num = String::new();
    for ch in s.chars() {
        if ch.is_ascii_digit() {
            num.push(ch);
            continue;
        }
        if num.is_empty() {
            return Err(format!("bad duration `{s}`: unit `{ch}` without number"));
        }
        let n: u64 = num.parse().map_err(|_| format!("bad duration `{s}`"))?;
        let chunk = match ch {
            's' => Duration::from_secs(n),
            'm' => Duration::from_secs(n * 60),
            'h' => Duration::from_secs(n * 3600),
            'd' => Duration::from_secs(n * 86_400),
            _ => return Err(format!("unknown duration unit `{ch}`")),
        };
        total += chunk;
        num.clear();
    }
    if !num.is_empty() {
        // Bare number — assume seconds for compatibility with `@every 30`.
        let n: u64 = num.parse().map_err(|_| format!("bad duration `{s}`"))?;
        total += Duration::from_secs(n);
    }
    Ok(total)
}

/// One-shot scheduled wakeup persisted to daemon state for replay across
/// restarts. Mirrors v132's `tengu_loop_dynamic_wakeup_*` payloads.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledWakeup {
    pub id: String,
    /// The prompt to post to the conversation when the wakeup fires.
    pub prompt: String,
    /// Why this was scheduled — surfaces in the daemon list output.
    pub reason: String,
    /// Absolute time at which the wakeup is due.
    pub fire_at: SystemTime,
    /// Time the wakeup was registered (for ordering / debugging).
    pub created_at: SystemTime,
}

// ─────────────────────────────────────────────────────────────────────────────
// Paths
// ─────────────────────────────────────────────────────────────────────────────

/// File-system layout used by the daemon.
#[derive(Debug, Clone)]
pub struct DaemonPaths {
    pub base_dir: PathBuf,
    pub pid_file: PathBuf,
    pub state_file: PathBuf,
    pub log_dir: PathBuf,
}

impl DaemonPaths {
    /// Build paths rooted at the given config directory (typically
    /// `~/.config/jfc`).
    pub fn new(config_dir: &Path) -> Self {
        let base_dir = config_dir.to_path_buf();
        Self {
            pid_file: base_dir.join("daemon.pid"),
            state_file: base_dir.join("daemon-state.json"),
            log_dir: base_dir.join("logs").join("daemon"),
            base_dir,
        }
    }

    /// Ensure all directories exist.
    pub fn ensure_dirs(&self) -> std::io::Result<()> {
        std::fs::create_dir_all(&self.base_dir)?;
        std::fs::create_dir_all(&self.log_dir)?;
        Ok(())
    }

    /// Default daemon paths under `~/.config/jfc`.
    pub fn default_user() -> Self {
        let cfg = dirs::config_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("jfc");
        Self::new(&cfg)
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// PID + state I/O
// ─────────────────────────────────────────────────────────────────────────────

/// Check if daemon is running by reading PID file and probing the process.
/// Returns the live PID, or `None` if the file is absent / process is dead.
pub fn is_daemon_running(paths: &DaemonPaths) -> Option<u32> {
    let pid_str = std::fs::read_to_string(&paths.pid_file).ok()?;
    let pid: u32 = pid_str.trim().parse().ok()?;

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
        return None;
    }

    #[cfg(not(unix))]
    {
        Some(pid)
    }
}

/// Write PID file for the current process.
pub fn write_pid_file(paths: &DaemonPaths) -> std::io::Result<()> {
    paths.ensure_dirs()?;
    std::fs::write(&paths.pid_file, std::process::id().to_string())
}

/// Remove PID file.
pub fn remove_pid_file(paths: &DaemonPaths) {
    let _ = std::fs::remove_file(&paths.pid_file);
}

/// Load daemon state from disk. Returns `None` when the file is missing
/// or unparseable; callers should fall back to `DaemonState::default()`.
pub fn load_state(paths: &DaemonPaths) -> Option<DaemonState> {
    let data = std::fs::read_to_string(&paths.state_file).ok()?;
    serde_json::from_str(&data).ok()
}

/// Save daemon state to disk (atomic write via tempfile + rename).
pub fn save_state(paths: &DaemonPaths, state: &DaemonState) -> std::io::Result<()> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(state)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let tmp = paths.state_file.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &paths.state_file)
}

// ─────────────────────────────────────────────────────────────────────────────
// Daemon Core
// ─────────────────────────────────────────────────────────────────────────────

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
        let _ = save_state(&self.paths, &self.state);
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

// ─────────────────────────────────────────────────────────────────────────────
// Cron firing logic
// ─────────────────────────────────────────────────────────────────────────────

/// Decide whether a cron job should fire at `now`.
///
/// `Every { period }` fires immediately if `last_run` is `None`, then
/// re-fires once at least `period` has elapsed.
///
/// `Crontab { … }` fires at most once per minute, when the minute /
/// hour / day / month / weekday fields all match the local-time
/// components of `now`. The "at most once" guard uses `last_run` so a
/// 30-second poll loop can't fire the same minute twice.
pub fn should_fire_cron(job: &CronJob, now: SystemTime) -> bool {
    match &job.schedule {
        CronSchedule::Every { period } => match job.last_run {
            None => true,
            Some(last) => {
                // `duration_since` errors if `now < last` (system clock went
                // backward). Saturate to ZERO and warn — a clock skew should
                // not silently re-fire jobs nor crash the daemon.
                match now.duration_since(last) {
                    Ok(elapsed) => elapsed >= *period,
                    Err(_) => {
                        tracing::warn!(
                            target: "jfc::daemon",
                            "clock skew detected: now < last_run for cron job {}",
                            job.id
                        );
                        false
                    }
                }
            }
        },
        CronSchedule::Crontab {
            minute,
            hour,
            day,
            month,
            weekday,
        } => {
            let parts = match local_parts(now) {
                Some(p) => p,
                None => return false,
            };
            if !minute.matches(parts.minute as u32) {
                return false;
            }
            if !hour.matches(parts.hour as u32) {
                return false;
            }
            if !day.matches(parts.day as u32) {
                return false;
            }
            if !month.matches(parts.month as u32) {
                return false;
            }
            if !weekday.matches(parts.weekday as u32) {
                return false;
            }
            // Don't refire within the same minute.
            if let Some(last) = job.last_run {
                let last_parts = match local_parts(last) {
                    Some(p) => p,
                    None => return true,
                };
                if last_parts.same_minute(&parts) {
                    return false;
                }
            }
            true
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct LocalParts {
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    /// 0 = Sunday … 6 = Saturday.
    weekday: u32,
}

impl LocalParts {
    fn same_minute(&self, other: &Self) -> bool {
        self.year == other.year
            && self.month == other.month
            && self.day == other.day
            && self.hour == other.hour
            && self.minute == other.minute
    }
}

/// Decompose `t` into local-time year/month/day/hour/minute/weekday using
/// chrono. Returns `None` if `t` predates the UNIX epoch.
fn local_parts(t: SystemTime) -> Option<LocalParts> {
    use chrono::{Datelike, Local, TimeZone, Timelike};
    let secs = t.duration_since(UNIX_EPOCH).ok()?.as_secs() as i64;
    let dt = Local.timestamp_opt(secs, 0).single()?;
    Some(LocalParts {
        year: dt.year(),
        month: dt.month(),
        day: dt.day(),
        hour: dt.hour(),
        minute: dt.minute(),
        weekday: dt.weekday().num_days_from_sunday(),
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────────────────

/// Read up to the last `n` lines of a file. Used by `daemon list/status`
/// to surface recent log output. Returns a placeholder when the file is
/// missing rather than erroring — the daemon log dir may legitimately
/// not contain a file for a session that never wrote one.
#[allow(dead_code)] // exposed for future `daemon logs <id>` plumbing
pub fn read_last_lines(path: &Path, n: usize) -> Vec<String> {
    let Ok(content) = std::fs::read_to_string(path) else {
        return vec!["(log file not found)".to_string()];
    };
    content
        .lines()
        .rev()
        .take(n)
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

fn uuid_short() -> String {
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

async fn run_cron_command(job: &CronJob) -> std::io::Result<()> {
    use tokio::process::Command;
    let status = Command::new("bash")
        .arg("-c")
        .arg(&job.command)
        .status()
        .await?;
    tracing::info!(
        target: "jfc::daemon",
        cron_id = %job.id,
        exit = ?status.code(),
        "cron command exited"
    );
    Ok(())
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
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("kill failed: {}", String::from_utf8_lossy(&result.stderr)),
            ));
        }
    }

    Ok(())
}

/// `jfc daemon status` — render a one-paragraph status string.
pub fn status_string(paths: &DaemonPaths) -> String {
    let running = is_daemon_running(paths);
    let state = load_state(paths).unwrap_or_default();
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
    s
}

/// `jfc daemon list` — render cron jobs + pending wakeups.
pub fn list_string(paths: &DaemonPaths) -> String {
    let state = load_state(paths).unwrap_or_default();
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
    s
}

fn describe_schedule(s: &CronSchedule) -> String {
    match s {
        CronSchedule::Every { period } => format!("@every {}s", period.as_secs()),
        CronSchedule::Crontab {
            minute,
            hour,
            day,
            month,
            weekday,
        } => format!(
            "{} {} {} {} {}",
            field_str(minute),
            field_str(hour),
            field_str(day),
            field_str(month),
            field_str(weekday),
        ),
    }
}

fn field_str(f: &CronField) -> String {
    match f {
        CronField::Any => "*".to_string(),
        CronField::Exact(n) => n.to_string(),
        CronField::Step(n) => format!("*/{n}"),
    }
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

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_daemon() -> (Daemon, TempDir) {
        let tmp = TempDir::new().unwrap();
        let daemon = Daemon::new(tmp.path()).unwrap();
        (daemon, tmp)
    }

    // ─── schedule parsing (DO-178B _normal) ─────────────────────────────

    #[test]
    fn parse_schedule_crontab_normal() {
        let s = parse_schedule("* * * * *").unwrap();
        match s {
            CronSchedule::Crontab {
                minute,
                hour,
                day,
                month,
                weekday,
            } => {
                assert!(matches!(minute, CronField::Any));
                assert!(matches!(hour, CronField::Any));
                assert!(matches!(day, CronField::Any));
                assert!(matches!(month, CronField::Any));
                assert!(matches!(weekday, CronField::Any));
            }
            _ => panic!("expected Crontab"),
        }
    }

    #[test]
    fn parse_schedule_daily_normal() {
        let s = parse_schedule("@daily").unwrap();
        match s {
            CronSchedule::Crontab { minute, hour, .. } => {
                assert_eq!(minute, CronField::Exact(0));
                assert_eq!(hour, CronField::Exact(0));
            }
            _ => panic!("expected Crontab"),
        }
    }

    #[test]
    fn parse_schedule_hourly_normal() {
        let s = parse_schedule("@hourly").unwrap();
        match s {
            CronSchedule::Crontab { minute, hour, .. } => {
                assert_eq!(minute, CronField::Exact(0));
                assert_eq!(hour, CronField::Any);
            }
            _ => panic!("expected Crontab"),
        }
    }

    #[test]
    fn parse_schedule_every_5m_normal() {
        let s = parse_schedule("@every 5m").unwrap();
        assert_eq!(
            s,
            CronSchedule::Every {
                period: Duration::from_secs(300)
            }
        );
    }

    #[test]
    fn parse_schedule_every_complex_normal() {
        let s = parse_schedule("@every 1h30m").unwrap();
        assert_eq!(
            s,
            CronSchedule::Every {
                period: Duration::from_secs(5400)
            }
        );
    }

    #[test]
    fn parse_schedule_step_normal() {
        let s = parse_schedule("*/15 * * * *").unwrap();
        match s {
            CronSchedule::Crontab { minute, .. } => {
                assert_eq!(minute, CronField::Step(15));
            }
            _ => panic!("expected Crontab"),
        }
    }

    // ─── schedule parsing (DO-178B _robust) ─────────────────────────────

    #[test]
    fn parse_schedule_empty_robust() {
        assert!(parse_schedule("").is_err());
        assert!(parse_schedule("   ").is_err());
    }

    #[test]
    fn parse_schedule_short_robust() {
        assert!(parse_schedule("* * *").is_err());
    }

    #[test]
    fn parse_schedule_garbage_field_robust() {
        assert!(parse_schedule("foo * * * *").is_err());
    }

    #[test]
    fn parse_schedule_zero_step_robust() {
        assert!(parse_schedule("*/0 * * * *").is_err());
    }

    #[test]
    fn parse_schedule_zero_every_robust() {
        assert!(parse_schedule("@every 0s").is_err());
    }

    #[test]
    fn parse_schedule_unknown_alias_robust() {
        // @yearly isn't supported; should fail rather than silently misparse.
        assert!(parse_schedule("@yearly").is_err());
    }

    // ─── should_fire_cron boundary conditions (DO-178B _normal/_robust) ─

    fn cron_job(schedule: CronSchedule, last_run: Option<SystemTime>) -> CronJob {
        CronJob {
            id: "cron-test".into(),
            schedule,
            description: "test".into(),
            command: "true".into(),
            enabled: true,
            last_run,
            created_at: SystemTime::now(),
        }
    }

    #[test]
    fn should_fire_every_first_run_normal() {
        let job = cron_job(
            CronSchedule::Every {
                period: Duration::from_secs(60),
            },
            None,
        );
        assert!(should_fire_cron(&job, SystemTime::now()));
    }

    #[test]
    fn should_fire_every_just_after_normal() {
        let now = SystemTime::now();
        let job = cron_job(
            CronSchedule::Every {
                period: Duration::from_secs(60),
            },
            Some(now - Duration::from_secs(61)),
        );
        assert!(should_fire_cron(&job, now));
    }

    #[test]
    fn should_fire_every_just_before_robust() {
        let now = SystemTime::now();
        let job = cron_job(
            CronSchedule::Every {
                period: Duration::from_secs(60),
            },
            Some(now - Duration::from_secs(59)),
        );
        assert!(!should_fire_cron(&job, now));
    }

    #[test]
    fn should_fire_every_exactly_at_boundary_normal() {
        let now = SystemTime::now();
        let job = cron_job(
            CronSchedule::Every {
                period: Duration::from_secs(60),
            },
            Some(now - Duration::from_secs(60)),
        );
        // At exactly the boundary the contract is `>=`, so it fires.
        assert!(should_fire_cron(&job, now));
    }

    #[test]
    fn should_fire_crontab_minute_match_normal() {
        // Build a `*/1 * * * *` (every minute) schedule and a "now"
        // value; the job should fire on the first poll.
        let s = parse_schedule("*/1 * * * *").unwrap();
        let job = cron_job(s, None);
        assert!(should_fire_cron(&job, SystemTime::now()));
    }

    #[test]
    fn should_fire_crontab_no_double_fire_within_minute_robust() {
        let s = parse_schedule("* * * * *").unwrap();
        let now = SystemTime::now();
        let job = cron_job(s, Some(now));
        // Same `now` ⇒ same minute ⇒ must not fire twice.
        assert!(!should_fire_cron(&job, now));
    }

    // ─── state save/load round-trip (DO-178B _normal) ───────────────────

    #[test]
    fn state_roundtrip_normal() {
        let tmp = TempDir::new().unwrap();
        {
            let mut d = Daemon::new(tmp.path()).unwrap();
            d.add_cron_job(
                parse_schedule("@daily").unwrap(),
                "nightly housekeeping",
                "echo hi",
            );
            d.schedule_wakeup(Duration::from_secs(60), "ping me", "test");
        }
        let d2 = Daemon::new(tmp.path()).unwrap();
        assert_eq!(d2.state.cron_jobs.len(), 1);
        assert_eq!(d2.state.cron_jobs[0].command, "echo hi");
        assert_eq!(d2.state.wakeups.len(), 1);
        assert_eq!(d2.state.wakeups[0].reason, "test");
    }

    #[test]
    fn state_roundtrip_empty_state_robust() {
        let tmp = TempDir::new().unwrap();
        // Reading from a fresh dir should yield default state.
        let d = Daemon::new(tmp.path()).unwrap();
        assert!(d.state.cron_jobs.is_empty());
        assert!(d.state.wakeups.is_empty());
    }

    #[test]
    fn state_roundtrip_corrupt_file_robust() {
        let tmp = TempDir::new().unwrap();
        let paths = DaemonPaths::new(tmp.path());
        paths.ensure_dirs().unwrap();
        std::fs::write(&paths.state_file, "not-json {{ ").unwrap();
        // Should not panic — `Daemon::new` falls back to default state.
        let d = Daemon::new(tmp.path()).unwrap();
        assert!(d.state.cron_jobs.is_empty());
    }

    // ─── ScheduleWakeup persistence (DO-178B _normal/_robust) ───────────

    #[test]
    fn schedule_wakeup_persistence_normal() {
        let tmp = TempDir::new().unwrap();
        let id;
        {
            let mut d = Daemon::new(tmp.path()).unwrap();
            id = d.schedule_wakeup(
                Duration::from_secs(120),
                "check the deploy",
                "user said `/loop check`",
            );
        }
        let d2 = Daemon::new(tmp.path()).unwrap();
        assert_eq!(d2.state.wakeups.len(), 1);
        assert_eq!(d2.state.wakeups[0].id, id);
        assert_eq!(d2.state.wakeups[0].prompt, "check the deploy");
    }

    #[test]
    fn schedule_wakeup_drain_due_normal() {
        let (mut d, _tmp) = test_daemon();
        d.schedule_wakeup(Duration::from_secs(0), "fire me", "now");
        d.schedule_wakeup(Duration::from_secs(3600), "later", "much later");
        let due = d.drain_due_wakeups(SystemTime::now() + Duration::from_secs(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].prompt, "fire me");
        assert_eq!(d.state.wakeups.len(), 1, "future wakeup must remain");
        assert_eq!(d.state.fired_wakeups.len(), 1);
    }

    #[test]
    fn schedule_wakeup_drain_due_replays_after_restart_robust() {
        let tmp = TempDir::new().unwrap();
        {
            let mut d = Daemon::new(tmp.path()).unwrap();
            d.schedule_wakeup(Duration::from_secs(0), "p1", "r1");
        }
        let mut d2 = Daemon::new(tmp.path()).unwrap();
        let due = d2.drain_due_wakeups(SystemTime::now() + Duration::from_secs(1));
        assert_eq!(due.len(), 1);
        assert_eq!(due[0].reason, "r1");
    }

    #[test]
    fn schedule_wakeup_no_due_returns_empty_robust() {
        let (mut d, _tmp) = test_daemon();
        d.schedule_wakeup(Duration::from_secs(3600), "later", "much later");
        let due = d.drain_due_wakeups(SystemTime::now());
        assert!(due.is_empty());
        assert_eq!(d.state.wakeups.len(), 1);
    }

    // ─── existing surface-area tests ────────────────────────────────────

    #[test]
    fn add_remove_cron_normal() {
        let (mut daemon, _tmp) = test_daemon();
        let id = daemon.add_cron_job(
            CronSchedule::Every {
                period: Duration::from_secs(1800),
            },
            "periodic check",
            "true",
        );
        assert_eq!(daemon.state.cron_jobs.len(), 1);
        assert!(daemon.remove_cron_job(&id));
        assert_eq!(daemon.state.cron_jobs.len(), 0);
    }

    #[test]
    fn remove_unknown_cron_robust() {
        let (mut daemon, _tmp) = test_daemon();
        assert!(!daemon.remove_cron_job("no-such-id"));
    }

    #[test]
    fn fire_cron_advances_last_run_normal() {
        let (mut daemon, _tmp) = test_daemon();
        let id = daemon.add_cron_job(
            CronSchedule::Every {
                period: Duration::from_secs(60),
            },
            "x",
            "true",
        );
        let now = SystemTime::now();
        let snapshot = daemon.fire_cron(&id, now).unwrap();
        assert_eq!(snapshot.last_run, Some(now));
        // Re-run should not fire — period not elapsed yet.
        let fired = daemon.tick_cron(now);
        assert!(fired.is_empty());
    }

    #[test]
    fn cleanup_old_sessions_normal() {
        let (mut daemon, _tmp) = test_daemon();
        let id = daemon.start_session("old", None, Path::new("/tmp"));
        daemon.update_session_status(&id, SessionStatus::Completed);
        daemon.cleanup_old_sessions(Duration::from_secs(0));
        assert!(daemon.state.sessions.is_empty());
    }

    #[test]
    fn paths_default_user_uses_jfc_subdir_normal() {
        let p = DaemonPaths::default_user();
        assert!(p.state_file.ends_with("daemon-state.json"));
        assert!(p.pid_file.ends_with("daemon.pid"));
    }
}
