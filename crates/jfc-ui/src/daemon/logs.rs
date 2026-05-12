//! Log and launch-spec path helpers for background agents.
//!
//! - `read_last_lines` is the tail-of-file primitive shared by the
//!   `daemon logs`, `daemon attach`, and UI restore code paths.
//! - `append_log_line` writes a single line to a per-agent log file under
//!   `~/.config/jfc/logs/daemon/agents/<id>.log`. Used by every state-
//!   transition recorder (registry, worker, reconcile).
//! - `background_agent_log_path` / `background_agent_launch_path` are the
//!   canonical paths for per-agent log + launch-spec files.

use std::path::{Path, PathBuf};

use super::state::DaemonPaths;

/// Read up to the last `n` lines of a file. Used by `daemon list/status`
/// to surface recent log output. Returns a placeholder when the file is
/// missing rather than erroring — the daemon log dir may legitimately
/// not contain a file for a session that never wrote one.
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

pub(super) fn append_log_line(path: &Path, line: &str) {
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = writeln!(file, "{line}");
    }
}

pub(super) fn background_agent_log_path(paths: &DaemonPaths, id: &str) -> PathBuf {
    paths.log_dir.join("agents").join(format!("{id}.log"))
}

pub(super) fn background_agent_launch_path(paths: &DaemonPaths, id: &str) -> PathBuf {
    paths
        .log_dir
        .join("agents")
        .join(format!("{id}.launch.json"))
}
