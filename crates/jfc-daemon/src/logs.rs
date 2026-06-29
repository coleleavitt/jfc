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
    let _linkscope_read = linkscope::phase("daemon.logs.read_last_lines");
    linkscope::event_fields(
        "daemon.logs.read_last_lines.start",
        [
            linkscope::TraceField::text("path", path.display().to_string()),
            linkscope::TraceField::count("requested_lines", usize_to_u64_saturating(n)),
        ],
    );
    let Ok(content) = std::fs::read_to_string(path) else {
        linkscope::event_fields(
            "daemon.logs.read_last_lines.result",
            [
                linkscope::TraceField::text("status", "missing_or_unreadable"),
                linkscope::TraceField::text("path", path.display().to_string()),
            ],
        );
        return vec!["(log file not found)".to_string()];
    };
    linkscope::record_bytes(
        "daemon.logs.read_last_lines.input",
        usize_to_u64_saturating(content.len()),
    );
    let lines = content
        .lines()
        .rev()
        .take(n)
        .map(String::from)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>();
    linkscope::event_fields(
        "daemon.logs.read_last_lines.result",
        [
            linkscope::TraceField::text("status", "ok"),
            linkscope::TraceField::count("returned_lines", usize_to_u64_saturating(lines.len())),
        ],
    );
    lines
}

pub fn append_log_line(path: &Path, line: &str) {
    let _linkscope_append = linkscope::phase("daemon.logs.append_line");
    linkscope::record_bytes(
        "daemon.logs.append_line.input",
        usize_to_u64_saturating(line.len()),
    );
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let needs_leading_newline = last_byte_is_not_newline(path);
    linkscope::detail_event_fields(
        "daemon.logs.append_line.open",
        [
            linkscope::TraceField::text("path", path.display().to_string()),
            linkscope::TraceField::count("leading_newline", u64::from(needs_leading_newline)),
        ],
    );
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        if needs_leading_newline {
            let _ = writeln!(file);
        }
        let _ = writeln!(file, "{line}");
        linkscope::event_fields(
            "daemon.logs.append_line.result",
            [linkscope::TraceField::text("status", "ok")],
        );
    } else {
        linkscope::event_fields(
            "daemon.logs.append_line.result",
            [
                linkscope::TraceField::text("status", "open_failed"),
                linkscope::TraceField::text("path", path.display().to_string()),
            ],
        );
    }
}

/// Append a raw chunk of streamed text to the log without inserting any
/// extra newlines. SSE deltas arrive mid-word ("SPIR-V lif" + "ter with"),
/// so a per-chunk `writeln!` turns prose into a column of fragments. The
/// model's own `\n` bytes (paragraph breaks, code fences) survive untouched.
pub(super) fn append_chunk_raw(path: &Path, text: &str) {
    let _linkscope_append = linkscope::phase("daemon.logs.append_chunk_raw");
    let bytes = usize_to_u64_saturating(text.len());
    linkscope::record_bytes("daemon.logs.append_chunk_raw.input", bytes);
    if text.is_empty() {
        linkscope::detail_event_fields(
            "daemon.logs.append_chunk_raw.skip",
            [linkscope::TraceField::text("reason", "empty")],
        );
        return;
    }
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        use std::io::Write;
        let _ = file.write_all(text.as_bytes());
        linkscope::detail_event_fields(
            "daemon.logs.append_chunk_raw.result",
            [
                linkscope::TraceField::text("status", "ok"),
                linkscope::TraceField::text("path", path.display().to_string()),
                linkscope::TraceField::bytes("bytes", bytes),
            ],
        );
    } else {
        linkscope::event_fields(
            "daemon.logs.append_chunk_raw.result",
            [
                linkscope::TraceField::text("status", "open_failed"),
                linkscope::TraceField::text("path", path.display().to_string()),
                linkscope::TraceField::bytes("bytes", bytes),
            ],
        );
    }
}

fn last_byte_is_not_newline(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if meta.len() == 0 {
        return false;
    }
    let Ok(mut file) = std::fs::OpenOptions::new().read(true).open(path) else {
        return false;
    };
    if file.seek(SeekFrom::End(-1)).is_err() {
        return false;
    }
    let mut buf = [0u8; 1];
    matches!(file.read(&mut buf), Ok(1) if buf[0] != b'\n')
}

/// Convert an arbitrary agent/task id into a filesystem-safe filename
/// stem. Path separators, `..`, NUL, and other non-alphanumeric bytes
/// would otherwise allow a crafted id to escape the agents log directory
/// (path traversal / arbitrary file write under daemon privileges). We
/// keep ASCII alphanumerics plus `-` and `_` verbatim and replace every
/// other byte with `_`, so the result always stays a single path
/// component. An empty or all-stripped id falls back to a stable token.
fn safe_id_stem(id: &str) -> String {
    let mut out = String::with_capacity(id.len());
    for ch in id.chars() {
        if ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("unnamed");
    }
    out
}

pub fn background_agent_log_path(paths: &DaemonPaths, id: &str) -> PathBuf {
    let path = paths
        .log_dir
        .join("agents")
        .join(format!("{}.log", safe_id_stem(id)));
    linkscope::detail_event_fields(
        "daemon.logs.agent_log_path",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::text("path", path.display().to_string()),
        ],
    );
    path
}

pub fn background_agent_launch_path(paths: &DaemonPaths, id: &str) -> PathBuf {
    let path = paths
        .log_dir
        .join("agents")
        .join(format!("{}.launch.json", safe_id_stem(id)));
    linkscope::detail_event_fields(
        "daemon.logs.agent_launch_path",
        [
            linkscope::TraceField::text("id", id.to_owned()),
            linkscope::TraceField::text("path", path.display().to_string()),
        ],
    );
    path
}

fn usize_to_u64_saturating(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
