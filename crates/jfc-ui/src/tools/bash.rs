use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::safe_tools::{
    configure_tool_command, non_interactive_shell_command, terminal_safe_text,
};
use super::{ExecutionResult, ToolProvenance, ToolSource};

type ProgressSink = Option<(String, tokio::sync::mpsc::Sender<crate::runtime::AppEvent>)>;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_FOREGROUND_BUDGET_MS: u64 = 15_000;
const INLINE_OUTPUT_BYTES: usize = 30_720;
const TAIL_BUFFER_BYTES: usize = INLINE_OUTPUT_BYTES * 2;
const DEFAULT_OUTPUT_LIMIT_LINES: u64 = 2_000;

#[derive(Clone, Debug)]
struct BashTaskInfo {
    id: String,
    command: String,
    cwd: PathBuf,
    output_path: PathBuf,
    status: BashTaskStatus,
    started_at_ms: u128,
    completed_at_ms: Option<u128>,
    total_bytes: u64,
    total_lines: u64,
}

#[derive(Clone, Debug)]
enum BashTaskStatus {
    Running,
    Completed { exit_code: i32 },
    TimedOut { timeout_ms: u64 },
    Failed { message: String },
}

impl BashTaskStatus {
    fn label(&self) -> String {
        match self {
            Self::Running => "running".to_owned(),
            Self::Completed { exit_code } => format!("completed exit={exit_code}"),
            Self::TimedOut { timeout_ms } => format!("timed_out after {timeout_ms}ms"),
            Self::Failed { message } => format!("failed: {message}"),
        }
    }
}

#[derive(Debug)]
struct BashOutputState {
    tail: String,
    total_bytes: u64,
    total_lines: u64,
}

impl BashOutputState {
    fn new() -> Self {
        Self {
            tail: String::new(),
            total_bytes: 0,
            total_lines: 0,
        }
    }

    fn push(&mut self, text: &str) {
        self.total_bytes = self.total_bytes.saturating_add(text.len() as u64);
        self.total_lines = self.total_lines.saturating_add(
            text.as_bytes()
                .iter()
                .filter(|byte| **byte == b'\n')
                .count() as u64,
        );
        self.tail.push_str(text);
        if self.tail.len() > TAIL_BUFFER_BYTES {
            let excess = self.tail.len() - TAIL_BUFFER_BYTES;
            let split = self.tail.ceil_char_boundary(excess);
            self.tail.drain(..split);
        }
    }

    fn snapshot(&self) -> BashOutputSnapshot {
        let truncated = self.total_bytes as usize > INLINE_OUTPUT_BYTES;
        let content = if truncated && self.tail.len() > INLINE_OUTPUT_BYTES {
            let start = self.tail.len() - INLINE_OUTPUT_BYTES;
            self.tail[self.tail.ceil_char_boundary(start)..].to_owned()
        } else {
            self.tail.clone()
        };
        BashOutputSnapshot {
            content,
            total_bytes: self.total_bytes,
            total_lines: self.total_lines,
            truncated,
        }
    }
}

#[derive(Debug)]
struct BashOutputSnapshot {
    content: String,
    total_bytes: u64,
    total_lines: u64,
    truncated: bool,
}

#[derive(Debug)]
struct BashTaskCompletion {
    status: BashTaskStatus,
    snapshot: BashOutputSnapshot,
}

struct RunningBashTask {
    id: String,
    command: String,
    cwd: PathBuf,
    output_path: PathBuf,
    completion: Option<oneshot::Receiver<BashTaskCompletion>>,
}

fn bash_tasks() -> &'static Mutex<HashMap<String, BashTaskInfo>> {
    static TASKS: OnceLock<Mutex<HashMap<String, BashTaskInfo>>> = OnceLock::new();
    TASKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default()
}

fn bash_output_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("JFC_BASH_OUTPUT_DIR") {
        return PathBuf::from(dir);
    }
    dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("jfc")
        .join("bash")
}

fn foreground_budget_ms() -> u64 {
    std::env::var("JFC_BASH_FOREGROUND_BUDGET_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_FOREGROUND_BUDGET_MS)
}

fn new_task_id() -> String {
    let raw = Uuid::new_v4().simple().to_string();
    format!("bash_{}", &raw[..12])
}

fn task_output_path(task_id: &str) -> PathBuf {
    bash_output_dir().join(format!("{task_id}.log"))
}

fn is_safe_task_id(task_id: &str) -> bool {
    !task_id.is_empty()
        && task_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn build_bash_command(command: &str, cwd: &Path) -> (Command, String) {
    let command = non_interactive_shell_command(command);

    let (executable, args) = match crate::sandbox::active_bash_sandbox_config() {
        Some(ref cfg) if cfg.enabled => match crate::sandbox::build_bwrap_argv(cfg, cwd) {
            Some(mut bwrap_argv) => {
                bwrap_argv.push("bash".into());
                bwrap_argv.push("-c".into());
                bwrap_argv.push(command.clone());
                let exe = bwrap_argv.remove(0);
                (exe, bwrap_argv)
            }
            None => ("bash".to_string(), vec!["-c".into(), command.clone()]),
        },
        _ => ("bash".to_string(), vec!["-c".into(), command.clone()]),
    };

    let mut cmd = Command::new(&executable);
    cmd.args(&args)
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
    if crate::effort::active_fast_mode() {
        cmd.env("CLAUDE_FAST_MODE", "1");
    }
    if let Ok(model) = std::env::var("JFC_MODEL").or_else(|_| std::env::var("ANTHROPIC_MODEL")) {
        cmd.env("CLAUDE_MODEL", model);
    }
    cmd.env_remove("CLICOLOR_FORCE")
        .env_remove("COLORTERM")
        .env_remove("EDITOR")
        .env_remove("FORCE_COLOR")
        .env_remove("GREP_COLORS")
        .env_remove("LS_COLORS")
        .env_remove("LD_PRELOAD")
        .env_remove("LD_AUDIT")
        .env_remove("LD_LIBRARY_PATH")
        .env_remove("LD_BIND_NOW")
        .env_remove("DYLD_INSERT_LIBRARIES")
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
        .env("MANPAGER", "cat")
        .env("PAGER", "cat");
    configure_tool_command(&mut cmd);
    (cmd, command)
}

async fn copy_pipe_to_output<R>(
    pipe: Option<R>,
    file: Arc<Mutex<tokio::fs::File>>,
    state: Arc<Mutex<BashOutputState>>,
    progress: ProgressSink,
    first_chunk_prefix: Option<&'static str>,
) where
    R: AsyncRead + Unpin + Send + 'static,
{
    let Some(mut pipe) = pipe else {
        return;
    };
    let mut buf = [0_u8; 8192];
    let mut first_chunk = true;
    loop {
        let n = match pipe.read(&mut buf).await {
            Ok(0) => break,
            Ok(n) => n,
            Err(_) => break,
        };
        let text = String::from_utf8_lossy(&buf[..n]);
        let mut safe = terminal_safe_text(text.replace('\r', "").as_ref());
        if first_chunk {
            first_chunk = false;
            if let Some(prefix) = first_chunk_prefix {
                safe = format!("{prefix}{safe}");
            }
        }
        {
            let mut state = state.lock().await;
            state.push(&safe);
        }
        {
            let mut file = file.lock().await;
            let _ = file.write_all(safe.as_bytes()).await;
        }
        if let Some((tool_id, tx)) = &progress {
            for line in safe.lines() {
                let _ = tx
                    .send(crate::runtime::AppEvent::Tool(
                        crate::runtime::ToolEvent::OutputChunk {
                            tool_id: crate::ids::ToolId::from(tool_id.clone()),
                            chunk: line.to_owned(),
                        },
                    ))
                    .await;
            }
        }
    }
}

async fn append_output_file(path: &Path, text: &str) {
    if let Ok(mut file) = tokio::fs::OpenOptions::new().append(true).open(path).await {
        let _ = file.write_all(text.as_bytes()).await;
    }
}

async fn start_bash_task(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
    progress: ProgressSink,
    track_pid: bool,
) -> Result<RunningBashTask, String> {
    use std::process::Stdio;

    let timeout = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let (mut cmd, command) = build_bash_command(command, cwd);
    let task_id = new_task_id();
    let output_path = task_output_path(&task_id);
    if let Some(parent) = output_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|err| format!("Failed to create Bash output dir: {err}"))?;
    }
    let file = tokio::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&output_path)
        .await
        .map_err(|err| format!("Failed to create Bash output file: {err}"))?;

    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = cmd
        .spawn()
        .map_err(|err| format!("Failed to spawn bash: {err}"))?;
    let pid_guard = track_pid
        .then(|| child.id().map(crate::bash_processes::PidGuard::register))
        .flatten();

    let cmd_preview: String = command.chars().take(100).collect();
    info!(
        target: "jfc::tools",
        task_id = %task_id,
        cmd = %cmd_preview,
        timeout_ms = timeout,
        cwd = %cwd.display(),
        output_path = %output_path.display(),
        "bash: executing"
    );

    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let file = Arc::new(Mutex::new(file));
    let state = Arc::new(Mutex::new(BashOutputState::new()));
    let stdout_task = tokio::spawn(copy_pipe_to_output(
        stdout,
        Arc::clone(&file),
        Arc::clone(&state),
        progress.clone(),
        None,
    ));
    let stderr_task = tokio::spawn(copy_pipe_to_output(
        stderr,
        Arc::clone(&file),
        Arc::clone(&state),
        progress,
        Some("\n---stderr---\n"),
    ));

    let started_at_ms = now_ms();
    let info = BashTaskInfo {
        id: task_id.clone(),
        command: command.clone(),
        cwd: cwd.to_path_buf(),
        output_path: output_path.clone(),
        status: BashTaskStatus::Running,
        started_at_ms,
        completed_at_ms: None,
        total_bytes: 0,
        total_lines: 0,
    };
    bash_tasks().lock().await.insert(task_id.clone(), info);

    let (completion_tx, completion_rx) = oneshot::channel();
    let task_id_for_wait = task_id.clone();
    let output_path_for_wait = output_path.clone();
    tokio::spawn(async move {
        let _pid_guard = pid_guard;
        let wait = tokio::time::timeout(Duration::from_millis(timeout), child.wait()).await;
        let status = match wait {
            Ok(Ok(status)) => BashTaskStatus::Completed {
                exit_code: status.code().unwrap_or(-1),
            },
            Ok(Err(err)) => BashTaskStatus::Failed {
                message: format!("failed to wait for child: {err}"),
            },
            Err(_) => {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let marker = format!("\n[Command timed out after {timeout}ms]\n");
                append_output_file(&output_path_for_wait, &marker).await;
                {
                    let mut state = state.lock().await;
                    state.push(&marker);
                }
                BashTaskStatus::TimedOut {
                    timeout_ms: timeout,
                }
            }
        };
        let _ = stdout_task.await;
        let _ = stderr_task.await;
        {
            let mut file = file.lock().await;
            let _ = file.flush().await;
        }
        let snapshot = state.lock().await.snapshot();
        {
            let mut tasks = bash_tasks().lock().await;
            if let Some(info) = tasks.get_mut(&task_id_for_wait) {
                info.status = status.clone();
                info.completed_at_ms = Some(now_ms());
                info.total_bytes = snapshot.total_bytes;
                info.total_lines = snapshot.total_lines;
            }
        }
        debug!(
            target: "jfc::tools",
            task_id = %task_id_for_wait,
            status = %status.label(),
            output_len = snapshot.total_bytes,
            "bash: task settled"
        );
        let _ = completion_tx.send(BashTaskCompletion { status, snapshot });
    });

    Ok(RunningBashTask {
        id: task_id,
        command,
        cwd: cwd.to_path_buf(),
        output_path,
        completion: Some(completion_rx),
    })
}

fn background_started_message(task: &RunningBashTask, reason: &str) -> String {
    format!(
        "{reason}\n\
         task_id: {task_id}\n\
         output_file: {output_file}\n\
         status: running\n\
         command: {command}\n\
         cwd: {cwd}\n\n\
         Use BashOutput with {{\"task_id\":\"{task_id}\"}} to read progress, or Read the output_file directly.",
        task_id = task.id,
        output_file = task.output_path.display(),
        command = task.command,
        cwd = task.cwd.display(),
    )
}

fn format_completed_task(
    task: &RunningBashTask,
    completion: BashTaskCompletion,
) -> ExecutionResult {
    let exit_code = match completion.status {
        BashTaskStatus::Completed { exit_code } => Some(exit_code),
        BashTaskStatus::TimedOut { timeout_ms } => {
            return ExecutionResult::failure(format!(
                "Command timed out after {timeout_ms}ms\n\
                 task_id: {task_id}\n\
                 output_file: {output_file}\n\n{output}",
                task_id = task.id,
                output_file = task.output_path.display(),
                output = completion.snapshot.content.trim_end(),
            ))
            .with_provenance(ToolProvenance {
                cwd: task.cwd.clone(),
                source: ToolSource::LocalExecutor,
            });
        }
        BashTaskStatus::Failed { message } => {
            return ExecutionResult::failure(format!(
                "{message}\n\
                 task_id: {task_id}\n\
                 output_file: {output_file}\n\n{output}",
                task_id = task.id,
                output_file = task.output_path.display(),
                output = completion.snapshot.content.trim_end(),
            ));
        }
        BashTaskStatus::Running => None,
    };

    let mut output = completion.snapshot.content.trim_end().to_owned();
    if output.is_empty() {
        output = "(no output)".to_owned();
    }
    if let Some(exit) = exit_code
        && exit != 0
    {
        output = format!("[exit {exit}]\n{output}");
    }
    if completion.snapshot.truncated {
        output.push_str(&format!(
            "\n\n[Output truncated: showing last ~{} bytes of {} bytes / {} lines]\n\
             output_file: {}\n\
             task_id: {}\n\
             Use BashOutput to read ranges.",
            INLINE_OUTPUT_BYTES,
            completion.snapshot.total_bytes,
            completion.snapshot.total_lines,
            task.output_path.display(),
            task.id,
        ));
    }
    ExecutionResult::success(output).with_provenance(ToolProvenance {
        cwd: task.cwd.clone(),
        source: ToolSource::LocalExecutor,
    })
}

pub(super) async fn execute_bash(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
) -> ExecutionResult {
    execute_bash_with_options(command, timeout_ms, cwd, None, false).await
}

pub(super) async fn execute_bash_with_options(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
    progress: ProgressSink,
    run_in_background: bool,
) -> ExecutionResult {
    let mut task =
        match start_bash_task(command, timeout_ms, cwd, progress, !run_in_background).await {
            Ok(task) => task,
            Err(err) => {
                warn!(target: "jfc::tools", error = %err, "bash: failed to start task");
                return ExecutionResult::failure(err);
            }
        };

    if run_in_background {
        return ExecutionResult::success(background_started_message(
            &task,
            "Command running in background.",
        ));
    }

    let timeout = timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
    let foreground_budget = foreground_budget_ms();
    let mut completion = task
        .completion
        .take()
        .expect("fresh Bash task must carry completion receiver");
    if foreground_budget < timeout {
        tokio::select! {
            completion = &mut completion => match completion {
                Ok(completion) => format_completed_task(&task, completion),
                Err(_) => ExecutionResult::failure("Bash task ended without a completion result"),
            },
            _ = tokio::time::sleep(Duration::from_millis(foreground_budget)) => {
                ExecutionResult::success(background_started_message(
                    &task,
                    &format!("Command exceeded the foreground budget ({foreground_budget}ms) and was moved to the background."),
                ))
            }
        }
    } else {
        match completion.await {
            Ok(completion) => format_completed_task(&task, completion),
            Err(_) => ExecutionResult::failure("Bash task ended without a completion result"),
        }
    }
}

/// Execute bash with optional streaming progress. When `progress_tx` is
/// provided, stdout/stderr chunks are streamed to the UI in real time via
/// `ToolOutputChunk` events.
#[allow(dead_code)]
pub(super) async fn execute_bash_inner(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
    progress: ProgressSink,
) -> ExecutionResult {
    execute_bash_with_options(command, timeout_ms, cwd, progress, false).await
}

pub(super) async fn execute_bash_output(
    task_id: &str,
    offset: Option<u64>,
    limit: Option<u64>,
) -> ExecutionResult {
    if !is_safe_task_id(task_id) {
        return ExecutionResult::failure("Invalid Bash task id");
    }

    let info = bash_tasks().lock().await.get(task_id).cloned();
    let output_path = info
        .as_ref()
        .map(|info| info.output_path.clone())
        .unwrap_or_else(|| task_output_path(task_id));
    let bytes = match tokio::fs::read(&output_path).await {
        Ok(bytes) => bytes,
        Err(err) => {
            return ExecutionResult::failure(format!(
                "No output available for Bash task {task_id}: {err}"
            ));
        }
    };
    let text = terminal_safe_text(&String::from_utf8_lossy(&bytes));
    let start_line = offset.unwrap_or(1).saturating_sub(1) as usize;
    let max_lines = limit.unwrap_or(DEFAULT_OUTPUT_LIMIT_LINES) as usize;
    let selected = text
        .lines()
        .skip(start_line)
        .take(max_lines)
        .collect::<Vec<_>>()
        .join("\n");
    let total_lines = text.lines().count();
    let shown_start = start_line.saturating_add(1);
    let shown_end = (start_line + selected.lines().count()).min(total_lines);
    let status = info
        .as_ref()
        .map(|info| info.status.label())
        .unwrap_or_else(|| "unknown (task not in current process registry)".to_owned());
    let metadata = match info {
        Some(info) => format!(
            "task_id: {}\nstatus: {}\noutput_file: {}\ncommand: {}\ncwd: {}\nstarted_at_ms: {}\ncompleted_at_ms: {}\nbytes: {}\nlines: {}\nshowing_lines: {}-{} of {}",
            info.id,
            status,
            info.output_path.display(),
            info.command,
            info.cwd.display(),
            info.started_at_ms,
            info.completed_at_ms
                .map(|value| value.to_string())
                .unwrap_or_else(|| "running".to_owned()),
            info.total_bytes.max(bytes.len() as u64),
            info.total_lines.max(total_lines as u64),
            shown_start,
            shown_end,
            total_lines,
        ),
        None => format!(
            "task_id: {task_id}\nstatus: {status}\noutput_file: {}\nbytes: {}\nlines: {}\nshowing_lines: {}-{} of {}",
            output_path.display(),
            bytes.len(),
            total_lines,
            shown_start,
            shown_end,
            total_lines,
        ),
    };

    let body = if selected.is_empty() {
        "(no output in requested range)".to_owned()
    } else {
        selected
    };
    ExecutionResult::success(format!("{metadata}\n\n{body}"))
}
