//! Daemon schema types + on-disk state I/O.
//!
//! - `DaemonState` is the root persistent record under
//!   `~/.config/jfc/daemon-state.json`.
//! - `DaemonPaths` is the filesystem layout (PID file, state file, log dir).
//! - `load_state` / `save_state` are the only blessed entry points for
//!   touching the state file; everything else in `daemon::*` goes through
//!   them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::cron::CronJob;

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
    /// Persistent background-agent roster. This backs `jfc daemon agents`,
    /// `logs`, `wait`, and cross-process cancellation requests.
    #[serde(default)]
    pub background_agents: HashMap<String, BackgroundAgentInfo>,
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
            background_agents: HashMap::new(),
        }
    }
}

pub(super) fn epoch() -> SystemTime {
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

/// Durable lifecycle for a background Task/subagent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BackgroundAgentStatus {
    Running,
    Completed,
    Failed,
    Cancelled,
}

impl BackgroundAgentStatus {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Persistent background-agent metadata. Background Tasks run in detached worker
/// processes; this record survives UI restarts and gives CLI tools a stable
/// roster/log/cancel/respawn substrate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundAgentInfo {
    pub id: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub status: BackgroundAgentStatus,
    pub started_at: SystemTime,
    pub updated_at: SystemTime,
    pub completed_at: Option<SystemTime>,
    pub pid: Option<u32>,
    pub model: Option<String>,
    pub worktree_path: Option<PathBuf>,
    pub log_path: PathBuf,
    #[serde(default)]
    pub launch_path: Option<PathBuf>,
    #[serde(default)]
    pub cancel_requested: bool,
    #[serde(default)]
    pub respawn_count: u32,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub tool_use_count: u32,
    #[serde(default)]
    pub latest_input_tokens: u64,
    #[serde(default)]
    pub latest_cache_read_tokens: u64,
    #[serde(default)]
    pub latest_cache_write_tokens: u64,
    #[serde(default)]
    pub cumulative_output_tokens: u64,
    /// Last tool the worker invoked. Persisted so the UI fan can show
    /// "task · last_tool" for detached agents, not just in-process ones.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_tool: Option<String>,
}

/// Durable worker launch metadata for a background Task. This is the piece that
/// lets a background agent run outside the TUI process and be respawned once if
/// its worker exits before reporting a terminal state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackgroundAgentLaunch {
    pub task_id: String,
    pub task_input: crate::types::TaskInput,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    pub model: crate::provider::ModelId,
    pub provider_name: Option<String>,
    pub agent_def: Option<crate::agents::AgentDef>,
    pub cwd: PathBuf,
    /// Absolute executable used to run `jfc daemon worker`.
    ///
    /// Persisting this avoids a respawn guessing from a later daemon process,
    /// and lets us report a precise error if the original binary was removed.
    #[serde(default)]
    pub worker_exe: Option<PathBuf>,
    pub active_team_name: Option<String>,
    pub created_at: SystemTime,
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
// State I/O
// ─────────────────────────────────────────────────────────────────────────────
//
// Concurrent writers (UI + N detached workers) all do
// `load_state → mutate → save_state` on the same JSON file. Without
// locking, two writers can interleave and one's changes can vanish.
// We take an exclusive advisory flock on a sidecar `.lock` file for the
// duration of any read-modify-write that callers run via
// [`with_state_lock`]. Plain `load_state`/`save_state` still work
// individually (atomic write via tempfile + rename) for callers that
// don't need read-modify-write atomicity.

#[cfg(unix)]
fn lock_state_file(lock_path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::io::AsRawFd;
    if let Some(parent) = lock_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(lock_path)?;
    // SAFETY: flock with LOCK_EX on a valid fd is safe; the kernel
    // releases the lock when the fd is closed (i.e., when `file` drops).
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

#[cfg(not(unix))]
fn lock_state_file(_lock_path: &Path) -> std::io::Result<std::fs::File> {
    // Non-unix: degrade to no-op lock. The atomic tempfile+rename in
    // `save_state` still gives us last-writer-wins consistency; we just
    // can't prevent inter-process clobbers of the same agent record.
    Ok(std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .read(true)
        .open(std::env::temp_dir().join("jfc-daemon-noop.lock"))?)
}

/// Hold an exclusive flock on the daemon state file for the duration of
/// the closure, then release it when the lock guard drops. Use this
/// around any read-modify-write of `DaemonState` from a context where
/// other processes (UI, workers) might be mutating the file in parallel.
pub fn with_state_lock<F, R>(paths: &DaemonPaths, f: F) -> R
where
    F: FnOnce() -> R,
{
    let lock_path = paths.state_file.with_extension("json.lock");
    // If we can't acquire the lock (rare — flock on a local file very
    // seldom fails), fall back to running unlocked rather than blocking
    // the entire UI. The atomic rename still bounds the worst case.
    let _guard = lock_state_file(&lock_path).ok();
    f()
}

/// Load daemon state from disk. Returns `None` when the file is missing
/// or unparseable; callers should fall back to `DaemonState::default()`.
pub fn load_state(paths: &DaemonPaths) -> Option<DaemonState> {
    let data = std::fs::read_to_string(&paths.state_file).ok()?;
    serde_json::from_str(&data).ok()
}

/// Default retention for terminal (completed/failed/cancelled) background
/// agents. Anything older than this is dropped on `compact_background_agents`.
pub const TERMINAL_AGENT_RETENTION: std::time::Duration =
    std::time::Duration::from_secs(7 * 24 * 60 * 60);

/// Cap on the number of terminal background agents retained regardless of
/// age. Most recent N are kept; the rest are dropped. Stops the state file
/// from growing unbounded when the user runs hundreds of background tasks
/// inside the retention window.
pub const TERMINAL_AGENT_CAP: usize = 100;

/// Drop terminal (Completed/Failed/Cancelled) background-agent records that
/// are either older than `retention` or beyond the most-recent `cap`.
/// Running agents are always retained. Returns the number of records dropped
/// so callers can decide whether to persist the compacted state.
pub fn compact_background_agents(
    state: &mut DaemonState,
    now: SystemTime,
    retention: std::time::Duration,
    cap: usize,
) -> usize {
    let initial_count = state.background_agents.len();
    // First pass: drop records past the retention window.
    let cutoff = now.checked_sub(retention).unwrap_or(UNIX_EPOCH);
    state.background_agents.retain(|_, agent| {
        if !agent.status.is_terminal() {
            return true;
        }
        let ts = agent.completed_at.unwrap_or(agent.updated_at);
        ts >= cutoff
    });

    // Second pass: cap the terminal-record count at `cap`.
    let mut terminal: Vec<(String, SystemTime)> = state
        .background_agents
        .iter()
        .filter(|(_, a)| a.status.is_terminal())
        .map(|(id, a)| (id.clone(), a.completed_at.unwrap_or(a.updated_at)))
        .collect();
    if terminal.len() > cap {
        terminal.sort_by_key(|(_, ts)| *ts);
        let drop_count = terminal.len() - cap;
        for (id, _) in terminal.into_iter().take(drop_count) {
            state.background_agents.remove(&id);
        }
    }
    initial_count.saturating_sub(state.background_agents.len())
}

/// Return the mtime of `daemon-state.json`, or `None` if the file is missing
/// or its metadata can't be read. Callers throttle reads of the (potentially
/// large) state file by comparing this against a cached value — when the
/// mtime is unchanged the parse can be skipped entirely.
pub fn state_file_mtime(paths: &DaemonPaths) -> Option<SystemTime> {
    std::fs::metadata(&paths.state_file).ok()?.modified().ok()
}

/// Load daemon state only when the file's mtime is newer than `cached`.
/// Returns `(state, new_mtime)` when a reload happened, `None` otherwise.
/// The UI calls this once per tick so a stable file (no new background
/// workers reporting progress) doesn't trigger a 1.4 MB JSON parse on the
/// render thread every second.
pub fn load_state_if_changed(
    paths: &DaemonPaths,
    cached: Option<SystemTime>,
) -> Option<(DaemonState, SystemTime)> {
    let mtime = state_file_mtime(paths)?;
    if Some(mtime) == cached {
        return None;
    }
    let state = load_state(paths)?;
    Some((state, mtime))
}

/// Save daemon state to disk (atomic write via tempfile + rename).
pub fn save_state(paths: &DaemonPaths, state: &DaemonState) -> std::io::Result<()> {
    paths.ensure_dirs()?;
    let json = serde_json::to_string_pretty(state).map_err(std::io::Error::other)?;
    let tmp = paths.state_file.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    std::fs::rename(&tmp, &paths.state_file)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn agent(
        id: &str,
        status: BackgroundAgentStatus,
        completed_offset: std::time::Duration,
        now: SystemTime,
    ) -> BackgroundAgentInfo {
        let ts = now - completed_offset;
        BackgroundAgentInfo {
            id: id.into(),
            description: "x".into(),
            parent_session_id: None,
            status,
            started_at: ts,
            updated_at: ts,
            completed_at: Some(ts),
            pid: None,
            model: None,
            worktree_path: None,
            log_path: PathBuf::from("/dev/null"),
            launch_path: None,
            cancel_requested: false,
            respawn_count: 0,
            summary: None,
            error: None,
            tool_use_count: 0,
            latest_input_tokens: 0,
            latest_cache_read_tokens: 0,
            latest_cache_write_tokens: 0,
            cumulative_output_tokens: 0,
            last_tool: None,
        }
    }

    // Normal: compact drops terminal agents older than the retention
    // window. Running agents are always retained.
    #[test]
    fn compact_drops_old_terminal_agents_normal() {
        let now = SystemTime::now();
        let mut state = DaemonState::default();
        state.background_agents.insert(
            "old".into(),
            agent(
                "old",
                BackgroundAgentStatus::Completed,
                std::time::Duration::from_secs(8 * 86400),
                now,
            ),
        );
        state.background_agents.insert(
            "fresh".into(),
            agent(
                "fresh",
                BackgroundAgentStatus::Completed,
                std::time::Duration::from_secs(60),
                now,
            ),
        );
        state.background_agents.insert(
            "running".into(),
            agent(
                "running",
                BackgroundAgentStatus::Running,
                std::time::Duration::from_secs(30 * 86400),
                now,
            ),
        );
        let dropped = compact_background_agents(
            &mut state,
            now,
            std::time::Duration::from_secs(7 * 86400),
            100,
        );
        assert_eq!(dropped, 1);
        assert!(!state.background_agents.contains_key("old"));
        assert!(state.background_agents.contains_key("fresh"));
        assert!(state.background_agents.contains_key("running"));
    }

    // Normal: when the terminal record count exceeds the cap, the oldest
    // are dropped first.
    #[test]
    fn compact_enforces_cap_keeps_most_recent_normal() {
        let now = SystemTime::now();
        let mut state = DaemonState::default();
        for i in 0..10 {
            state.background_agents.insert(
                format!("a{i}"),
                agent(
                    &format!("a{i}"),
                    BackgroundAgentStatus::Completed,
                    // a0 is oldest, a9 newest
                    std::time::Duration::from_secs((10 - i) as u64),
                    now,
                ),
            );
        }
        let dropped = compact_background_agents(
            &mut state,
            now,
            std::time::Duration::from_secs(86400),
            3,
        );
        assert_eq!(dropped, 7);
        assert_eq!(state.background_agents.len(), 3);
        // The three newest (highest i) must survive.
        for i in 7..10 {
            assert!(state.background_agents.contains_key(&format!("a{i}")));
        }
    }

    // Robust: compact on an empty state is a no-op.
    #[test]
    fn compact_noop_on_empty_state_robust() {
        let mut state = DaemonState::default();
        let dropped = compact_background_agents(
            &mut state,
            SystemTime::now(),
            std::time::Duration::from_secs(86400),
            100,
        );
        assert_eq!(dropped, 0);
    }

    // Normal: load_state_if_changed returns None when mtime is unchanged.
    #[test]
    fn load_state_if_changed_skips_unchanged_normal() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DaemonPaths::new(tmp.path());
        save_state(&paths, &DaemonState::default()).unwrap();
        let first = load_state_if_changed(&paths, None).expect("initial load");
        let cached = first.1;
        // Second call with the same mtime returns None.
        let second = load_state_if_changed(&paths, Some(cached));
        assert!(
            second.is_none(),
            "unchanged file must not re-parse on every poll"
        );
    }

    // Robust: load_state_if_changed re-parses when the mtime advances.
    #[test]
    fn load_state_if_changed_reloads_when_mtime_changes_robust() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = DaemonPaths::new(tmp.path());
        save_state(&paths, &DaemonState::default()).unwrap();
        let first = load_state_if_changed(&paths, None).expect("first");
        let cached = first.1;
        // Sleep past the filesystem mtime granularity, then rewrite.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut state = DaemonState::default();
        state.pid = 42;
        save_state(&paths, &state).unwrap();
        let second = load_state_if_changed(&paths, Some(cached));
        assert!(second.is_some(), "modified file must re-parse");
        let (loaded, new_mtime) = second.unwrap();
        assert_eq!(loaded.pid, 42);
        assert!(new_mtime > cached);
    }
}
