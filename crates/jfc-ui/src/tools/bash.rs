use std::path::Path;

use tracing::{debug, info, warn};

use super::{
    ExecutionResult, ToolProvenance, ToolSource, configure_tool_command,
    non_interactive_shell_command, terminal_safe_text,
};

type ProgressSink = Option<(String, tokio::sync::mpsc::Sender<crate::runtime::AppEvent>)>;

pub(super) async fn execute_bash(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
) -> ExecutionResult {
    execute_bash_inner(command, timeout_ms, cwd, None).await
}

async fn collect_streaming_pipe<R>(pipe: Option<R>, progress: ProgressSink) -> String
where
    R: tokio::io::AsyncRead + Unpin + Send + 'static,
{
    use tokio::io::{AsyncBufReadExt, BufReader};

    let Some(pipe) = pipe else {
        return String::new();
    };
    let mut reader = BufReader::new(pipe).lines();
    let mut buf = String::new();
    loop {
        match reader.next_line().await {
            Ok(Some(line)) => {
                let safe = terminal_safe_text(&line);
                buf.push_str(&safe);
                buf.push('\n');
                if let Some((tool_id, tx)) = &progress {
                    let _ = tx.try_send(crate::runtime::AppEvent::Tool(
                        crate::runtime::ToolEvent::OutputChunk {
                            tool_id: crate::ids::ToolId::from(tool_id.clone()),
                            chunk: safe,
                        },
                    ));
                }
            }
            Ok(None) | Err(_) => break,
        }
    }
    buf
}

/// Execute bash with optional streaming progress. When `progress_tx` is
/// provided, stdout/stderr lines are streamed to the UI in real-time via
/// `ToolOutputChunk` events.
pub(super) async fn execute_bash_inner(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
    progress: Option<(String, tokio::sync::mpsc::Sender<crate::runtime::AppEvent>)>,
) -> ExecutionResult {
    use std::process::Stdio;
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
    // Security-critical env-pin (recommendation #4 from the bash-CVE
    // research). These variables are the documented vectors for
    // "child process RCE from a parent that looked benign":
    //   * LD_PRELOAD / LD_AUDIT / LD_LIBRARY_PATH — inject shared
    //     libraries into every spawned binary. Even a `date` invocation
    //     becomes a code-exec primitive.
    //   * BASH_ENV / ENV — bash sources this file on non-interactive
    //     startup (Shellshock-era vector).
    //   * PROMPT_COMMAND / PS0..PS4 — command-substituted on each
    //     prompt; `@P}` expansion (Flatt #8) hooks here.
    //   * IFS — re-tokenizes the rest of the command line; setting
    //     it to a digit defeats parser anchors.
    //   * GIT_EXTERNAL_DIFF / GIT_SSH_COMMAND / GIT_DIR /
    //     GIT_ALTERNATE_OBJECT_DIRECTORIES — git RCE via config-hook
    //     equivalents (github.blog: git-security-vulnerabilities-
    //     announced-4).
    //   * MANPAGER / LESS / MANROFFSEQ — man-page viewers that exec
    //     filters; less in particular runs `!` commands.
    //   * BASH_FUNC_*() / shellshock — bash <4.3 parses these as
    //     function definitions in the env. `env_clear` would also
    //     work but we want to keep PATH and standard locale vars.
    cmd.env_remove("LD_PRELOAD")
        .env_remove("LD_AUDIT")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_BIND_NOW")
        .env_remove("DYLD_INSERT_LIBRARIES") // macOS equivalent of LD_PRELOAD
        .env_remove("DYLD_LIBRARY_PATH")
        .env_remove("BASH_ENV")
        .env_remove("ENV")
        .env_remove("PROMPT_COMMAND")
        .env_remove("PS0")
        .env_remove("PS1")
        .env_remove("PS2")
        .env_remove("PS3")
        .env_remove("PS4")
        .env_remove("IFS")
        .env_remove("CDPATH")
        .env_remove("GLOBIGNORE")
        .env_remove("HISTFILE")
        .env_remove("HISTCMD")
        .env_remove("GIT_EXTERNAL_DIFF")
        .env_remove("GIT_DIR")
        .env_remove("GIT_SSH_COMMAND")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_CONFIG")
        .env_remove("GIT_CONFIG_GLOBAL")
        .env_remove("GIT_CONFIG_SYSTEM")
        .env_remove("MANPAGER")
        .env_remove("LESS")
        .env_remove("LESSOPEN")
        .env_remove("LESSCLOSE")
        .env_remove("MANROFFSEQ")
        .env_remove("MANROFFOPT")
        // Pin PAGER, MANPAGER explicitly to cat — any tool that
        // honors them now goes through a deterministic, non-exec
        // viewer. (Already had PAGER above; MANPAGER added here.)
        .env("MANPAGER", "cat")
        .env("PAGER", "cat");
    configure_tool_command(&mut cmd);

    // If streaming, pipe stdout/stderr and drain both concurrently. Reading
    // stdout to EOF before stderr can deadlock commands that write enough
    // diagnostics to fill the stderr pipe (cargo/rustc are common examples).
    if progress.is_some() {
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
                let stdout_task = tokio::spawn(collect_streaming_pipe(stdout, progress.clone()));
                let stderr_task = tokio::spawn(collect_streaming_pipe(stderr, progress.clone()));
                let status =
                    tokio::time::timeout(std::time::Duration::from_millis(timeout), child.wait())
                        .await;
                let status = match status {
                    Ok(status) => status,
                    Err(_) => {
                        let _ = child.kill().await;
                        let _ = child.wait().await;
                        stdout_task.abort();
                        stderr_task.abort();
                        return ExecutionResult::failure(
                            "Command timed out (streaming)".to_string(),
                        );
                    }
                };
                let stdout_buf = stdout_task.await.unwrap_or_default();
                let stderr_buf = stderr_task.await.unwrap_or_default();
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
