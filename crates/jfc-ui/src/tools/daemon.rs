use std::path::Path;

use super::ExecutionResult;

pub(super) fn execute_cron_create(
    schedule_expr: &str,
    command: &str,
    description: &str,
) -> ExecutionResult {
    use crate::daemon::{Daemon, DaemonPaths, parse_schedule};
    let schedule = match parse_schedule(schedule_expr) {
        Ok(s) => s,
        Err(e) => {
            return ExecutionResult::failure(format!("Invalid schedule `{schedule_expr}`: {e}"));
        }
    };
    let paths = DaemonPaths::default_user();
    let mut daemon = match Daemon::new(&paths.base_dir) {
        Ok(d) => d,
        Err(e) => return ExecutionResult::failure(format!("daemon state init failed: {e}")),
    };
    let id = daemon.add_cron_job(schedule, description, command);
    ExecutionResult::success(format!(
        "Created cron job `{id}` ({schedule_expr}): {description}\n  command: {command}"
    ))
}

pub(super) fn execute_cron_list() -> ExecutionResult {
    use crate::daemon::{Daemon, DaemonPaths};
    let paths = DaemonPaths::default_user();
    let daemon = match Daemon::new(&paths.base_dir) {
        Ok(d) => d,
        Err(e) => return ExecutionResult::failure(format!("daemon state init failed: {e}")),
    };
    if daemon.state.cron_jobs.is_empty() {
        return ExecutionResult::success("(no cron jobs registered)".to_string());
    }
    let mut out = String::new();
    for j in &daemon.state.cron_jobs {
        out.push_str(&format!(
            "{} [{}] {}\n  command: {}\n",
            j.id,
            if j.enabled { "on" } else { "off" },
            j.description,
            j.command,
        ));
    }
    ExecutionResult::success(out)
}

pub(super) fn execute_cron_delete(id: &str) -> ExecutionResult {
    use crate::daemon::{Daemon, DaemonPaths};
    let paths = DaemonPaths::default_user();
    let mut daemon = match Daemon::new(&paths.base_dir) {
        Ok(d) => d,
        Err(e) => return ExecutionResult::failure(format!("daemon state init failed: {e}")),
    };
    if daemon.remove_cron_job(id) {
        ExecutionResult::success(format!("Deleted cron job `{id}`"))
    } else {
        ExecutionResult::failure(format!("No cron job with id `{id}`"))
    }
}

pub(crate) fn execute_schedule_wakeup(
    delay_seconds: u32,
    prompt: &str,
    reason: &str,
) -> ExecutionResult {
    use crate::daemon::{Daemon, DaemonPaths};
    if prompt.is_empty() {
        return ExecutionResult::failure("ScheduleWakeup: prompt must not be empty");
    }
    // Autonomous loop sentinel handling: when the agent passes the special
    // sentinel string, we expand it to the full loop preamble + loop.md content
    // so the wakeup fires with proper instructions instead of the bare token.
    let resolved_prompt = if crate::autonomous_loop::is_loop_sentinel(prompt) {
        let pacing = if prompt == crate::autonomous_loop::LOOP_SENTINEL_DYNAMIC {
            crate::autonomous_loop::LoopPacing::Dynamic
        } else {
            crate::autonomous_loop::LoopPacing::Cron
        };
        let mut expanded = crate::autonomous_loop::AUTONOMOUS_LOOP_PREAMBLE.to_string();
        expanded.push_str("\n\n");
        expanded.push_str(crate::autonomous_loop::loop_tick_preamble(pacing));
        // Append loop.md if present.
        let git_root = crate::context::discover_git_root();
        let project = git_root
            .as_deref()
            .unwrap_or_else(|| std::path::Path::new("."));
        if let Some(loop_md) = crate::autonomous_loop::read_loop_file(project) {
            expanded.push_str("\n\n## loop.md\n\n");
            expanded.push_str(&loop_md);
        }
        expanded
    } else {
        prompt.to_string()
    };
    let paths = DaemonPaths::default_user();
    let mut daemon = match Daemon::new(&paths.base_dir) {
        Ok(d) => d,
        Err(e) => return ExecutionResult::failure(format!("daemon state init failed: {e}")),
    };
    let delay = std::time::Duration::from_secs(u64::from(delay_seconds));
    let id = daemon.schedule_wakeup(delay, &resolved_prompt, reason);
    let note = if crate::autonomous_loop::is_loop_sentinel(prompt) {
        " (autonomous loop sentinel expanded)"
    } else {
        ""
    };
    ExecutionResult::success(format!(
        "Scheduled wakeup `{id}` in {delay_seconds}s: {reason}{note}"
    ))
}

/// Spawn `command` and stream stdout line-by-line until `until` matches
/// or 60s elapse. Reuses the same `tokio::process` + `BufReader::lines`
/// pattern that `execute_bash_inner` uses so behaviour stays consistent
/// (line-buffered output, no terminal-color env, sane defaults).
pub(super) async fn execute_monitor(command: &str, until: &str, cwd: &Path) -> ExecutionResult {
    use std::process::Stdio;
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command as TokioCommand;

    const MONITOR_TIMEOUT_SECS: u64 = 60;

    let regex = match regex::Regex::new(until) {
        Ok(r) => r,
        Err(e) => return ExecutionResult::failure(format!("invalid `until` regex: {e}")),
    };

    let mut cmd = TokioCommand::new("bash");
    cmd.arg("-c")
        .arg(command)
        .current_dir(cwd)
        .env("CI", "true")
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .env("PAGER", "cat")
        .env("GIT_PAGER", "cat")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => return ExecutionResult::failure(format!("failed to spawn monitor: {e}")),
    };

    let stdout = match child.stdout.take() {
        Some(s) => s,
        None => return ExecutionResult::failure("monitor: no stdout handle"),
    };
    let mut reader = BufReader::new(stdout).lines();
    let mut tail: Vec<String> = Vec::new();
    const TAIL_KEEP: usize = 20;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(MONITOR_TIMEOUT_SECS);
    loop {
        let next = tokio::time::timeout_at(deadline, reader.next_line()).await;
        match next {
            Ok(Ok(Some(line))) => {
                if regex.is_match(&line) {
                    let _ = child.kill().await;
                    return ExecutionResult::success(format!("matched: {line}"));
                }
                tail.push(line);
                if tail.len() > TAIL_KEEP {
                    tail.remove(0);
                }
            }
            Ok(Ok(None)) => {
                // EOF without match — process exited.
                let exit = child.wait().await.ok().and_then(|s| s.code()).unwrap_or(-1);
                let body = if tail.is_empty() {
                    format!("Process exited [{exit}] without matching /{until}/")
                } else {
                    format!(
                        "Process exited [{exit}] without matching /{until}/. Last {} line(s):\n{}",
                        tail.len(),
                        tail.join("\n")
                    )
                };
                return ExecutionResult::failure(body);
            }
            Ok(Err(e)) => {
                let _ = child.kill().await;
                return ExecutionResult::failure(format!("monitor read error: {e}"));
            }
            Err(_) => {
                let _ = child.kill().await;
                let body = if tail.is_empty() {
                    format!("Timeout after {MONITOR_TIMEOUT_SECS}s without matching /{until}/")
                } else {
                    format!(
                        "Timeout after {MONITOR_TIMEOUT_SECS}s without matching /{until}/. Last {} line(s):\n{}",
                        tail.len(),
                        tail.join("\n")
                    )
                };
                return ExecutionResult::failure(body);
            }
        }
    }
}
