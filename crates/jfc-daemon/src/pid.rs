//! PID tracking helpers.
//!
//! - `is_daemon_running` reads the daemon PID file and probes the process.
//! - `process_is_running` probes by signal 0 (kill -0) on Unix; trivially
//!   true on non-Unix (no portable probe).
//!
//! The historical `/proc/<pid>/cmdline` worker-discovery pass that lived
//! here is gone — the UI no longer races the worker on the PID field, so
//! there's nothing to repair after the fact. See
//! `registry::record_background_agent_started_at` for the write contract.

use super::state::DaemonPaths;

/// Check if daemon is running by reading PID file and probing the process.
/// Returns the live PID, or `None` if the file is absent / process is dead.
pub fn is_daemon_running(paths: &DaemonPaths) -> Option<u32> {
    let pid_str = std::fs::read_to_string(&paths.pid_file).ok()?;
    let pid: u32 = pid_str.trim().parse().ok()?;

    process_is_running(pid).then_some(pid)
}

pub(super) fn process_is_running(pid: u32) -> bool {
    if pid == 0 {
        return false;
    }

    #[cfg(unix)]
    {
        use std::process::Command;
        let result = Command::new("kill").args(["-0", &pid.to_string()]).output();
        result.map(|r| r.status.success()).unwrap_or(false)
    }

    #[cfg(not(unix))]
    {
        true
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
