use std::path::Path;

use tracing::{debug, info, warn};

use super::{
    ExecutionResult, ToolProvenance, ToolSource, configure_tool_command,
    non_interactive_shell_command, terminal_safe_text,
};

pub(super) async fn execute_bash(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
) -> ExecutionResult {
    execute_bash_inner(command, timeout_ms, cwd, None).await
}

/// Execute bash with optional streaming progress. When `progress_tx` is
/// provided, stdout lines are streamed to the UI in real-time via
/// `ToolOutputChunk` events.
pub(super) async fn execute_bash_inner(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
    progress: Option<(String, tokio::sync::mpsc::Sender<crate::runtime::AppEvent>)>,
) -> ExecutionResult {
    use std::process::Stdio;
    use tokio::io::{AsyncBufReadExt, BufReader};
    use tokio::process::Command;

    let timeout = timeout_ms.unwrap_or(120_000);
    let command = non_interactive_shell_command(command);

    let cmd_preview: String = command.chars().take(100).collect();
    info!(target: "jfc::tools", cmd = %cmd_preview, timeout_ms = timeout, cwd = %cwd.display(), "bash: executing");

    let mut cmd = Command::new("bash");
    cmd.arg("-c")
        .arg(&command)
        .current_dir(cwd)
        .env("CI", "true")
        .env("TERM", "dumb")
        .env("NO_COLOR", "1")
        .env("CLICOLOR", "0")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .env("GCM_INTERACTIVE", "never")
        .env("GIT_EDITOR", ":")
        .env("GIT_MERGE_AUTOEDIT", "no")
        .env("GIT_PAGER", "cat")
        .env("GIT_SEQUENCE_EDITOR", ":")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("HOMEBREW_NO_AUTO_UPDATE", "1")
        .env("PAGER", "cat")
        .env("PIP_NO_INPUT", "1")
        .env("VISUAL", "");
    // Expose fast-mode flag so bash scripts can detect it.
    if crate::effort::active_fast_mode() {
        cmd.env("CLAUDE_FAST_MODE", "1");
    }
    // Expose current model to subprocess (read from env, same as main process).
    if let Ok(model) = std::env::var("JFC_MODEL").or_else(|_| std::env::var("ANTHROPIC_MODEL")) {
        cmd.env("CLAUDE_MODEL", model);
    }
    cmd.env_remove("CLICOLOR_FORCE")
        .env_remove("COLORTERM")
        .env_remove("EDITOR")
        .env_remove("FORCE_COLOR")
        .env_remove("GREP_COLORS")
        .env_remove("LS_COLORS");
    configure_tool_command(&mut cmd);

    // If streaming, pipe stdout and read line-by-line
    if let Some((ref tool_id, ref tx)) = progress {
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let spawn_result = cmd.spawn();
        match spawn_result {
            Ok(mut child) => {
                // v132: track the PID so user-initiated abort (ESC×2) can
                // SIGTERM the in-flight process. The PidGuard deregisters
                // on Drop so every exit path (success, timeout, error) is
                // covered automatically.
                let _pid_guard = child.id().map(crate::bash_processes::PidGuard::register);
                let stdout = child.stdout.take();
                let stderr = child.stderr.take();
                let mut stdout_buf = String::new();
                let mut stderr_buf = String::new();

                // Stream stdout line-by-line
                if let Some(stdout) = stdout {
                    let mut reader = BufReader::new(stdout).lines();
                    let deadline =
                        tokio::time::Instant::now() + std::time::Duration::from_millis(timeout);
                    loop {
                        let line = tokio::time::timeout_at(deadline, reader.next_line()).await;
                        match line {
                            Ok(Ok(Some(l))) => {
                                // Strip ANSI / control bytes BEFORE the
                                // chunk goes anywhere — the UI's
                                // sanitize_terminal_text scrubber runs
                                // at draw-time but the raw bytes still
                                // sit in app state and can corrupt the
                                // TUI buffer if they slip through. Bug:
                                // git's `--stat` output occasionally
                                // includes box-drawing or backspace
                                // sequences depending on locale, and
                                // those leaked through the streaming
                                // path (the non-streaming path already
                                // scrubs at line 167).
                                let safe = super::terminal_safe_text(&l);
                                stdout_buf.push_str(&safe);
                                stdout_buf.push('\n');
                                // Send chunk to UI (non-blocking)
                                let _ = tx.try_send(crate::runtime::AppEvent::Tool(
                                    crate::runtime::ToolEvent::OutputChunk {
                                        tool_id: crate::ids::ToolId::from(tool_id.clone()),
                                        chunk: safe,
                                    },
                                ));
                            }
                            Ok(Ok(None)) => break, // EOF
                            Ok(Err(_)) => break,   // read error
                            Err(_) => {
                                // Timeout — kill the process
                                let _ = child.kill().await;
                                return ExecutionResult::failure(
                                    "Command timed out (streaming)".to_string(),
                                );
                            }
                        }
                    }
                }

                // Collect stderr (not streamed — shown at end)
                if let Some(mut stderr) = stderr {
                    use tokio::io::AsyncReadExt;
                    let _ = stderr.read_to_string(&mut stderr_buf).await;
                }

                let status = child.wait().await;
                let exit = status.map(|s| s.code().unwrap_or(-1)).unwrap_or(-1);
                debug!(target: "jfc::tools", exit_code = exit, stdout_len = stdout_buf.len(), stderr_len = stderr_buf.len(), "bash: completed (streamed)");

                let header = if exit == 0 {
                    String::new()
                } else {
                    format!("[exit {exit}]\n")
                };
                // Scrub stderr too — it's the same untrusted input as
                // stdout. stdout was already scrubbed line-by-line
                // above; this catches stderr (read in one shot).
                let stderr_buf = terminal_safe_text(&stderr_buf);
                let body = if stderr_buf.is_empty() {
                    stdout_buf
                } else if stdout_buf.is_empty() {
                    stderr_buf
                } else {
                    format!("{stdout_buf}\n---stderr---\n{stderr_buf}")
                };
                ExecutionResult::success(format!("{header}{body}"))
            }
            Err(e) => ExecutionResult::failure(format!("Failed to spawn: {e}")),
        }
    } else {
        // Non-streaming path. Spawn explicitly (rather than `cmd.output()`)
        // so we can register the PID for ESC×2 SIGTERM handling — same
        // contract as the streaming path above.
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                warn!(target: "jfc::tools", error = %e, "bash: failed to spawn");
                return ExecutionResult::failure(format!("Failed to spawn bash: {e}"));
            }
        };
        let _pid_guard = child.id().map(crate::bash_processes::PidGuard::register);
        let result = tokio::time::timeout(
            std::time::Duration::from_millis(timeout),
            child.wait_with_output(),
        )
        .await;

        match result {
            Ok(Ok(out)) => {
                let stdout = String::from_utf8_lossy(&out.stdout);
                let stderr = String::from_utf8_lossy(&out.stderr);
                let exit = out.status.code().unwrap_or(-1);
                debug!(target: "jfc::tools", exit_code = exit, stdout_len = stdout.len(), stderr_len = stderr.len(), "bash: completed");
                // Bash semantics per Anthropic's reference tool: a non-zero exit
                // code is part of the *output*, not a tool failure. Many shell
                // utilities use exit 1 as a normal signal (`grep` with no matches,
                // `diff` finding differences, `test` for false). Marking those
                // Failed shows the tool row as red even though the command ran
                // perfectly. Always Complete; the model reads the exit code in
                // the output prefix and interprets.
                let exit = out.status.code().unwrap_or(-1);
                let header = if exit == 0 {
                    String::new()
                } else {
                    format!("[exit {exit}]\n")
                };
                let body = if stderr.is_empty() {
                    stdout.to_string()
                } else if stdout.is_empty() {
                    stderr.to_string()
                } else {
                    format!("{stdout}\n---stderr---\n{stderr}")
                };
                let body = terminal_safe_text(body.trim_end());
                ExecutionResult::success(format!("{header}{body}")).with_provenance(
                    ToolProvenance {
                        cwd: cwd.to_path_buf(),
                        source: ToolSource::LocalExecutor,
                    },
                )
            }
            Ok(Err(e)) => {
                warn!(target: "jfc::tools", error = %e, "bash: failed to spawn");
                ExecutionResult::failure(format!("Failed to spawn bash: {e}"))
            }
            Err(_) => {
                warn!(target: "jfc::tools", timeout_ms = timeout, "bash: command timed out");
                ExecutionResult::failure(format!("Command timed out after {timeout}ms"))
            }
        }
    }
}
