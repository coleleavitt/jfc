use std::path::Path;

use super::ExecutionResult;

pub fn execute_cron_create(
    schedule_expr: &str,
    command: &str,
    description: &str,
) -> ExecutionResult {
    use crate::daemon::{Daemon, DaemonPaths, parse_schedule};
    // CS-JFC-004: a cron command is later executed by the daemon as `bash -c`
    // without the interactive Bash approval path. Run the same catastrophic
    // classifier the live Bash tool uses *before* persistence so the model
    // cannot schedule an effectively-unrecoverable command for later, unattended
    // execution. (`JFC_ALLOW_CATASTROPHIC_BASH` still overrides for power users.)
    if let Some(reason) = crate::app::shell_safety::catastrophic_bash_reason(command) {
        return ExecutionResult::failure(format!(
            "CronCreate refused: scheduled command is {reason}. \
             Catastrophic commands cannot be persisted for unattended execution."
        ));
    }
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

pub fn execute_cron_list() -> ExecutionResult {
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

pub fn execute_cron_delete(id: &str) -> ExecutionResult {
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

pub fn execute_schedule_wakeup(delay_seconds: u32, prompt: &str, reason: &str) -> ExecutionResult {
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
    // Clamp the model-chosen delay into the supported dynamic-wakeup window for
    // autonomous-loop sentinels only. A bare `<<autonomous-loop-dynamic>>`
    // keepalive or an over-eager 5s reschedule would otherwise hammer the loop;
    // a multi-day delay would silently age it out. Non-loop wakeups (explicit
    // user reminders) keep the exact requested delay. Mirrors Claude 2.1.177's
    // `Zz5` clamp of `delaySeconds` to `[ZT$, yJ8]`.
    let (effective_seconds, was_clamped) = if crate::autonomous_loop::is_loop_sentinel(prompt) {
        crate::autonomous_loop::clamp_wakeup_delay_seconds(delay_seconds)
    } else {
        (delay_seconds, false)
    };
    if was_clamped {
        tracing::info!(
            target: "jfc::autonomous_loop",
            requested_seconds = delay_seconds,
            clamped_seconds = effective_seconds,
            "tengu_loop_dynamic_wakeup_scheduled: clamped dynamic-wakeup delay into supported window"
        );
    }
    let delay = std::time::Duration::from_secs(u64::from(effective_seconds));
    let id = daemon.schedule_wakeup(delay, &resolved_prompt, reason);
    let note = if crate::autonomous_loop::is_loop_sentinel(prompt) {
        if was_clamped {
            " (autonomous loop sentinel expanded; delay clamped)"
        } else {
            " (autonomous loop sentinel expanded)"
        }
    } else {
        ""
    };
    ExecutionResult::success(format!(
        "Scheduled wakeup `{id}` in {effective_seconds}s: {reason}{note}"
    ))
}

/// Spawn `command` and stream stdout line-by-line until `until` matches
/// or 60s elapse. Reuses the same `tokio::process` + `BufReader::lines`
/// pattern that `execute_bash_inner` uses so behaviour stays consistent
/// (line-buffered output, no terminal-color env, sane defaults).
pub async fn execute_monitor(command: &str, until: &str, cwd: &Path) -> ExecutionResult {
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

#[cfg(test)]
mod cron_safety_tests {
    use super::*;

    // CS-JFC-004: catastrophic commands must be rejected at CronCreate time,
    // before they are ever persisted for unattended `bash -c` execution. The
    // rejection path returns before touching daemon state, so this is filesystem-safe.
    #[test]
    fn cron_create_rejects_catastrophic_command_regression() {
        let result = execute_cron_create("0 0 * * *", "rm -rf /", "nightly cleanup");
        assert!(result.is_error(), "catastrophic cron must be rejected");
        assert!(
            result.output.contains("CronCreate refused"),
            "unexpected message: {}",
            result.output
        );
    }

    #[test]
    fn cron_create_rejects_disk_wipe_robust() {
        let result = execute_cron_create("0 0 * * *", "mkfs.ext4 /dev/sda", "format");
        assert!(result.is_error());
    }
}
