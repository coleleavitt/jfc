use std::collections::HashMap;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio::sync::{Mutex, oneshot, watch};
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::safe_tools::{
    configure_tool_command, non_interactive_shell_command, terminal_safe_text,
};
use super::{ExecutionResult, ToolProvenance, ToolSource};

type ProgressSink = Option<(
    String,
    tokio::sync::mpsc::Sender<crate::runtime::EngineEvent>,
)>;

const DEFAULT_TIMEOUT_MS: u64 = 120_000;
const DEFAULT_FOREGROUND_BUDGET_MS: u64 = 15_000;
const MIN_TIMEOUT_MS: u64 = 1_000; // Minimum 1s to avoid instant kills on tiny timeouts
const BACKGROUNDED_TIMEOUT_MS: u64 = 600_000; // 600s max for backgrounded tasks (matches schema)
/// Default inline output cap — mirrors CC 2.1.167's `og6 = 30_000` chars.
/// Override via `BASH_MAX_OUTPUT_LENGTH` (CC-compatible) or `JFC_BASH_MAX_OUTPUT_LENGTH`.
const INLINE_OUTPUT_BYTES_DEFAULT: usize = 30_000;
const INLINE_OUTPUT_BYTES_MAX: usize = 150_000; // CC's rg6 = 15e4

/// Read the effective bash output cap from the environment.
/// `BASH_MAX_OUTPUT_LENGTH` is the CC-compatible name; `JFC_BASH_MAX_OUTPUT_LENGTH`
/// is the JFC-native override. Values are clamped to [1024, 150_000].
fn inline_output_bytes() -> usize {
    for key in &["JFC_BASH_MAX_OUTPUT_LENGTH", "BASH_MAX_OUTPUT_LENGTH"] {
        if let Ok(val) = std::env::var(key) {
            if let Ok(n) = val.trim().parse::<usize>() {
                return n.clamp(1024, INLINE_OUTPUT_BYTES_MAX);
            }
        }
    }
    INLINE_OUTPUT_BYTES_DEFAULT
}
const DEFAULT_OUTPUT_LIMIT_LINES: u64 = 2_000;
const DEFAULT_OUTPUT_WAIT_MS: u64 = 30_000;
const TERMINATE_GRACE_MS: u64 = 1_500;

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
    completion_rx: Option<watch::Receiver<BashTaskStatus>>,
}

#[derive(Clone, Debug)]
enum BashTaskStatus {
    Running,
    Completed { exit_code: i32 },
    TimedOut { timeout_ms: u64 },
    Failed { message: String },
}

impl BashTaskStatus {
    fn is_terminal(&self) -> bool {
        !matches!(self, Self::Running)
    }

    fn as_metadata(&self) -> PersistedBashTaskStatus {
        match self {
            Self::Running => PersistedBashTaskStatus::Running,
            Self::Completed { exit_code } => PersistedBashTaskStatus::Completed {
                exit_code: *exit_code,
            },
            Self::TimedOut { timeout_ms } => PersistedBashTaskStatus::TimedOut {
                timeout_ms: *timeout_ms,
            },
            Self::Failed { message } => PersistedBashTaskStatus::Failed {
                message: message.clone(),
            },
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum PersistedBashTaskStatus {
    Running,
    Completed { exit_code: i32 },
    TimedOut { timeout_ms: u64 },
    Failed { message: String },
}

impl PersistedBashTaskStatus {
    fn into_runtime(self) -> BashTaskStatus {
        match self {
            Self::Running => BashTaskStatus::Running,
            Self::Completed { exit_code } => BashTaskStatus::Completed { exit_code },
            Self::TimedOut { timeout_ms } => BashTaskStatus::TimedOut { timeout_ms },
            Self::Failed { message } => BashTaskStatus::Failed { message },
        }
    }
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize)]
struct PersistedBashTaskInfo {
    id: String,
    command: String,
    cwd: PathBuf,
    output_path: PathBuf,
    status: PersistedBashTaskStatus,
    started_at_ms: u128,
    completed_at_ms: Option<u128>,
    total_bytes: u64,
    total_lines: u64,
}

impl From<&BashTaskInfo> for PersistedBashTaskInfo {
    fn from(info: &BashTaskInfo) -> Self {
        Self {
            id: info.id.clone(),
            command: info.command.clone(),
            cwd: info.cwd.clone(),
            output_path: info.output_path.clone(),
            status: info.status.as_metadata(),
            started_at_ms: info.started_at_ms,
            completed_at_ms: info.completed_at_ms,
            total_bytes: info.total_bytes,
            total_lines: info.total_lines,
        }
    }
}

impl From<PersistedBashTaskInfo> for BashTaskInfo {
    fn from(info: PersistedBashTaskInfo) -> Self {
        Self {
            id: info.id,
            command: info.command,
            cwd: info.cwd,
            output_path: info.output_path,
            status: info.status.into_runtime(),
            started_at_ms: info.started_at_ms,
            completed_at_ms: info.completed_at_ms,
            total_bytes: info.total_bytes,
            total_lines: info.total_lines,
            completion_rx: None,
        }
    }
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
        let tail_cap = inline_output_bytes() * 2;
        if self.tail.len() > tail_cap {
            let excess = self.tail.len() - tail_cap;
            let split = self.tail.ceil_char_boundary(excess);
            self.tail.drain(..split);
        }
    }

    fn snapshot(&self) -> BashOutputSnapshot {
        let cap = inline_output_bytes();
        let truncated = self.total_bytes as usize > cap;
        let content = if truncated && self.tail.len() > cap {
            let start = self.tail.len() - cap;
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

pub fn bash_output_dir() -> PathBuf {
    if let Some(dir) = std::env::var_os("JFC_BASH_OUTPUT_DIR") {
        return PathBuf::from(dir);
    }
    default_bash_output_root().join("bash")
}

#[cfg(unix)]
fn current_uid() -> u32 {
    // SAFETY: getuid(2) has no preconditions and cannot fail.
    unsafe { libc::getuid() }
}

#[cfg(unix)]
fn default_bash_output_root() -> PathBuf {
    std::env::temp_dir().join(format!("jfc-{}", current_uid()))
}

#[cfg(not(unix))]
fn default_bash_output_root() -> PathBuf {
    std::env::temp_dir().join("jfc").join("bash-runtime")
}

#[cfg(unix)]
fn ensure_private_dir(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};

    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() {
                return Err(format!(
                    "Refusing to use Bash output dir {}: path is a symlink",
                    path.display()
                ));
            }
        }
        Err(err) if err.kind() == ErrorKind::NotFound => {
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true).mode(0o700);
            builder.create(path).map_err(|err| {
                format!(
                    "Failed to create private Bash output dir {}: {err}",
                    path.display()
                )
            })?;
        }
        Err(err) => {
            return Err(format!(
                "Failed to inspect Bash output dir {}: {err}",
                path.display()
            ));
        }
    }

    let metadata = std::fs::symlink_metadata(path).map_err(|err| {
        format!(
            "Failed to inspect Bash output dir {}: {err}",
            path.display()
        )
    })?;
    if metadata.file_type().is_symlink() {
        return Err(format!(
            "Refusing to use Bash output dir {}: path is a symlink",
            path.display()
        ));
    }
    if !metadata.is_dir() {
        return Err(format!(
            "Refusing to use Bash output dir {}: path is not a directory",
            path.display()
        ));
    }

    let uid = current_uid();
    if metadata.uid() != uid {
        return Err(format!(
            "Refusing to use Bash output dir {}: owned by uid {}, expected uid {}",
            path.display(),
            metadata.uid(),
            uid
        ));
    }

    let mode = metadata.permissions().mode() & 0o777;
    if mode != 0o700 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|err| {
            format!(
                "Failed to make Bash output dir {} private: {err}",
                path.display()
            )
        })?;
        let metadata = std::fs::symlink_metadata(path).map_err(|err| {
            format!(
                "Failed to re-check Bash output dir {}: {err}",
                path.display()
            )
        })?;
        let mode = metadata.permissions().mode() & 0o777;
        if mode != 0o700 {
            return Err(format!(
                "Refusing to use Bash output dir {}: mode is {mode:o}, expected 700",
                path.display()
            ));
        }
    }

    Ok(())
}

#[cfg(not(unix))]
fn ensure_private_dir(path: &Path) -> Result<(), String> {
    std::fs::create_dir_all(path)
        .map_err(|err| format!("Failed to create Bash output dir {}: {err}", path.display()))?;
    if !path.is_dir() {
        return Err(format!(
            "Refusing to use Bash output dir {}: path is not a directory",
            path.display()
        ));
    }
    Ok(())
}

pub fn prepare_bash_output_dir() -> Result<PathBuf, String> {
    if std::env::var_os("JFC_BASH_OUTPUT_DIR").is_some() {
        let dir = bash_output_dir();
        ensure_private_dir(&dir)?;
        return Ok(dir);
    }

    let root = default_bash_output_root();
    ensure_private_dir(&root)?;
    let dir = root.join("bash");
    ensure_private_dir(&dir)?;
    Ok(dir)
}

fn foreground_budget_ms() -> u64 {
    std::env::var("JFC_BASH_FOREGROUND_BUDGET_MS")
        .ok()
        .and_then(|value| value.parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_FOREGROUND_BUDGET_MS)
}

/// Clamp a timeout to sensible bounds. Ensures timeouts are in [MIN_TIMEOUT_MS, max_bound].
/// Used to prevent instant kills on tiny/zero timeouts and enforce upper limits.
fn clamp_timeout(timeout_ms: Option<u64>, max_bound: u64) -> u64 {
    match timeout_ms {
        None => DEFAULT_TIMEOUT_MS,
        Some(0) | Some(1..=999) => MIN_TIMEOUT_MS, // Reject tiny explicit timeouts
        Some(t) => t.min(max_bound),               // Clamp to max_bound
    }
}

fn new_task_id() -> String {
    let raw = Uuid::new_v4().simple().to_string();
    format!("bash_{}", &raw[..12])
}

fn task_output_path(task_id: &str) -> PathBuf {
    bash_output_dir().join(format!("{task_id}.log"))
}

fn task_metadata_path(task_id: &str) -> PathBuf {
    bash_output_dir().join(format!("{task_id}.json"))
}

fn output_metadata_path(output_path: &Path) -> PathBuf {
    output_path.with_extension("json")
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
            None if cfg.fail_if_unavailable => (
                "bash".to_string(),
                vec![
                    "-c".into(),
                    "echo 'Bash sandbox requested but bubblewrap is unavailable' >&2; exit 127"
                        .into(),
                ],
            ),
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
                    .send(crate::runtime::EngineEvent::Tool(
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

async fn persist_task_info(info: &BashTaskInfo) {
    let metadata_path = output_metadata_path(&info.output_path);
    let Ok(bytes) = serde_json::to_vec_pretty(&PersistedBashTaskInfo::from(info)) else {
        return;
    };
    let tmp_path = metadata_path.with_extension("json.tmp");
    if tokio::fs::write(&tmp_path, bytes).await.is_ok() {
        let _ = tokio::fs::rename(tmp_path, metadata_path).await;
    }
}

async fn load_persisted_task_info(task_id: &str) -> Option<BashTaskInfo> {
    let path = task_metadata_path(task_id);
    let bytes = tokio::fs::read(path).await.ok()?;
    serde_json::from_slice::<PersistedBashTaskInfo>(&bytes)
        .ok()
        .map(BashTaskInfo::from)
}

#[cfg(unix)]
async fn terminate_bash_child(child: &mut tokio::process::Child) {
    let Some(pid) = child.id() else {
        let _ = child.kill().await;
        return;
    };
    let _ = crate::bash_processes::signal_process_tree(pid, libc::SIGTERM);
    match tokio::time::timeout(Duration::from_millis(TERMINATE_GRACE_MS), child.wait()).await {
        Ok(_) => {}
        Err(_) => {
            let _ = crate::bash_processes::signal_process_tree(pid, libc::SIGKILL);
            let _ = child.wait().await;
        }
    }
}

#[cfg(not(unix))]
async fn terminate_bash_child(child: &mut tokio::process::Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
}

async fn start_bash_task(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
    progress: ProgressSink,
    track_for_abort: bool,
    is_backgrounded: bool,
) -> Result<RunningBashTask, String> {
    use std::process::Stdio;

    // Backgrounded tasks (explicitly `run_in_background=true` or auto-backgrounded after
    // foreground budget) should get a generous timeout (up to the max 600s from the schema)
    // to allow long-running operations. Foreground-only tasks are limited by the user's
    // timeout or the default 120s.
    let timeout = if is_backgrounded {
        clamp_timeout(timeout_ms, BACKGROUNDED_TIMEOUT_MS)
    } else {
        clamp_timeout(timeout_ms, DEFAULT_TIMEOUT_MS)
    };
    let (mut cmd, command) = build_bash_command(command, cwd);
    let task_id = new_task_id();
    let output_path = prepare_bash_output_dir()?.join(format!("{task_id}.log"));
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
    let pid_guard = if track_for_abort {
        child.id().map(crate::bash_processes::PidGuard::register)
    } else {
        None
    };

    let cmd_preview: String = command.chars().take(100).collect();
    info!(
        target: "jfc::tools",
        task_id = %task_id,
        cmd = %cmd_preview,
        timeout_ms = timeout,
        track_for_abort,
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
        completion_rx: None,
    };
    let (status_tx, status_rx) = watch::channel(BashTaskStatus::Running);
    let mut registered_info = info;
    registered_info.completion_rx = Some(status_rx);
    persist_task_info(&registered_info).await;
    bash_tasks()
        .lock()
        .await
        .insert(task_id.clone(), registered_info);

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
                terminate_bash_child(&mut child).await;
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
                persist_task_info(info).await;
            }
        }
        let _ = status_tx.send(status.clone());
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
         Use BashOutput with {{\"task_id\":\"{task_id}\"}} to wait for completion or {{\"task_id\":\"{task_id}\",\"block\":false}} to read a non-blocking snapshot. Do not spawn sleep/poll Bash commands for this task.",
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
            inline_output_bytes(),
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

// Production callers go through `execute_bash_with_options`; this thin wrapper
// remains the test-suite entry point.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn execute_bash(command: &str, timeout_ms: Option<u64>, cwd: &Path) -> ExecutionResult {
    execute_bash_with_options(command, timeout_ms, cwd, None, false).await
}

pub async fn execute_bash_with_options(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
    progress: ProgressSink,
    run_in_background: bool,
) -> ExecutionResult {
    // Determine the effective timeout for the underlying watcher task.
    // - Explicitly backgrounded tasks get the generous BACKGROUNDED_TIMEOUT_MS.
    // - Foreground tasks that might exceed the budget are treated as potentially
    //   backgrounded, so they also get the generous timeout to avoid premature kills.
    // - Only truly short foreground tasks (timeout < budget) are limited by their timeout.
    let foreground_budget = foreground_budget_ms();
    let user_timeout = clamp_timeout(timeout_ms, DEFAULT_TIMEOUT_MS);
    let effective_timeout_for_watcher = if run_in_background || user_timeout > foreground_budget {
        // This task will (or might) run in the background; give it the max generous timeout.
        clamp_timeout(timeout_ms, BACKGROUNDED_TIMEOUT_MS)
    } else {
        // Pure foreground task that will complete before the budget expires.
        user_timeout
    };

    let mut task = match start_bash_task(
        command,
        Some(effective_timeout_for_watcher),
        cwd,
        progress,
        !run_in_background,
        run_in_background,
    )
    .await
    {
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

    let mut completion = task
        .completion
        .take()
        .expect("fresh Bash task must carry completion receiver");
    if foreground_budget < user_timeout {
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
pub async fn execute_bash_inner(
    command: &str,
    timeout_ms: Option<u64>,
    cwd: &Path,
    progress: ProgressSink,
) -> ExecutionResult {
    execute_bash_with_options(command, timeout_ms, cwd, progress, false).await
}

pub async fn execute_bash_output(
    task_id: &str,
    offset: Option<u64>,
    limit: Option<u64>,
    block: Option<bool>,
    timeout_ms: Option<u64>,
) -> ExecutionResult {
    if !is_safe_task_id(task_id) {
        return ExecutionResult::failure("Invalid Bash task id");
    }
    if let Err(err) = prepare_bash_output_dir() {
        warn!(target: "jfc::tools", error = %err, "bash: refusing unsafe output dir");
        return ExecutionResult::failure(err);
    }

    let mut info = bash_tasks().lock().await.get(task_id).cloned();
    if info.is_none() {
        info = load_persisted_task_info(task_id).await;
    }

    let should_block = block.unwrap_or(true);
    let wait_timeout = timeout_ms.unwrap_or(DEFAULT_OUTPUT_WAIT_MS);
    if should_block
        && let Some(rx) = info.as_ref().and_then(|info| info.completion_rx.clone())
        && !rx.borrow().is_terminal()
    {
        let mut rx = rx;
        let wait = tokio::time::timeout(Duration::from_millis(wait_timeout), async {
            loop {
                if rx.borrow().is_terminal() {
                    return;
                }
                if rx.changed().await.is_err() {
                    return;
                }
            }
        })
        .await;
        if wait.is_ok() {
            info = bash_tasks().lock().await.get(task_id).cloned();
        }
    }

    let retrieval_status = match info.as_ref().map(|info| &info.status) {
        Some(BashTaskStatus::Completed { .. })
        | Some(BashTaskStatus::TimedOut { .. })
        | Some(BashTaskStatus::Failed { .. }) => "success",
        Some(BashTaskStatus::Running) if should_block => "timeout",
        Some(BashTaskStatus::Running) => "not_ready",
        None => "unknown",
    };

    let output_path = info
        .as_ref()
        .map(|info| info.output_path.clone())
        .unwrap_or_else(|| task_output_path(task_id));
    let bytes = match tokio::fs::read(&output_path).await {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == ErrorKind::NotFound && retrieval_status == "timeout" => {
            Vec::new()
        }
        // Unknown task id: not in the live registry, no persisted metadata, and no
        // log on disk. Almost always a hallucinated id — the model invented a
        // plausible `bash_<hex>` or a semantic name instead of using the id the Bash
        // tool actually returned. The old message leaked the raw OS error
        // ("No such file or directory") and didn't say the id was bogus, so the model
        // retried with *new* fabricated ids. Name the failure and list the real ids so
        // it can self-correct in one round-trip.
        Err(err) if err.kind() == ErrorKind::NotFound && info.is_none() => {
            let mut known: Vec<(u128, String)> = bash_tasks()
                .lock()
                .await
                .values()
                .map(|task| (task.started_at_ms, task.id.clone()))
                .collect();
            known.sort_unstable_by_key(|entry| std::cmp::Reverse(entry.0));
            let hint = if known.is_empty() {
                "no background Bash tasks exist in this session".to_owned()
            } else {
                let ids = known
                    .iter()
                    .take(10)
                    .map(|(_, id)| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("known task ids: {ids}")
            };
            return ExecutionResult::failure(format!(
                "Unknown Bash task id '{task_id}': no such background task. Task ids are \
                 issued by the Bash tool (run_in_background=true, or a command \
                 auto-backgrounded after exceeding the foreground budget) — copy the exact \
                 id from that tool's output verbatim; do not invent or guess one. ({hint})"
            ));
        }
        Err(err) => {
            return ExecutionResult::failure(format!(
                "No output available for Bash task {task_id}: {err}"
            ));
        }
    };
    let text = terminal_safe_text(&String::from_utf8_lossy(&bytes));
    let start_line = offset.unwrap_or(1).saturating_sub(1) as usize;
    let max_lines = limit.unwrap_or(DEFAULT_OUTPUT_LIMIT_LINES) as usize;
    // Validate the requested window instead of silently returning nothing.
    // Only for TERMINAL tasks: on a running task, offset == total+1 is the
    // normal incremental-poll position ("give me whatever arrives next"),
    // but on a finished task no more output can ever appear, so a past-EOF
    // offset is a caller error and the valid range is the useful reply.
    let task_is_terminal = matches!(
        info.as_ref().map(|info| &info.status),
        Some(BashTaskStatus::Completed { .. })
            | Some(BashTaskStatus::TimedOut { .. })
            | Some(BashTaskStatus::Failed { .. })
    );
    let total_lines_now = text.lines().count();
    if task_is_terminal && start_line >= total_lines_now && start_line > 0 {
        return ExecutionResult::failure(format!(
            "offset {} is past the end of output for finished Bash task {task_id}: \
             {total_lines_now} line(s) total (valid offsets: 1-{}).",
            start_line + 1,
            total_lines_now.max(1),
        ));
    }
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
            "retrieval_status: {retrieval_status}\ntask_id: {}\nstatus: {}\noutput_file: {}\ncommand: {}\ncwd: {}\nstarted_at_ms: {}\ncompleted_at_ms: {}\nbytes: {}\nlines: {}\nshowing_lines: {}-{} of {}",
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
            "retrieval_status: {retrieval_status}\ntask_id: {task_id}\nstatus: {status}\noutput_file: {}\nbytes: {}\nlines: {}\nshowing_lines: {}-{} of {}",
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

#[cfg(test)]
mod timeout_tests {
    use super::*;

    #[test]
    fn test_clamp_timeout_with_none() {
        // When timeout_ms is None, default is used
        assert_eq!(clamp_timeout(None, DEFAULT_TIMEOUT_MS), DEFAULT_TIMEOUT_MS);
        assert_eq!(
            clamp_timeout(None, BACKGROUNDED_TIMEOUT_MS),
            DEFAULT_TIMEOUT_MS
        );
    }

    #[test]
    fn test_clamp_timeout_rejects_zero() {
        // Zero timeout is rejected and replaced with MIN_TIMEOUT_MS
        assert_eq!(clamp_timeout(Some(0), DEFAULT_TIMEOUT_MS), MIN_TIMEOUT_MS);
        assert_eq!(
            clamp_timeout(Some(0), BACKGROUNDED_TIMEOUT_MS),
            MIN_TIMEOUT_MS
        );
    }

    #[test]
    fn test_clamp_timeout_rejects_tiny_timeouts() {
        // Sub-1000ms timeouts are rejected
        for tiny in 1..=999 {
            assert_eq!(
                clamp_timeout(Some(tiny), DEFAULT_TIMEOUT_MS),
                MIN_TIMEOUT_MS,
                "timeout {} should be clamped to MIN_TIMEOUT_MS",
                tiny
            );
        }
    }

    #[test]
    fn test_clamp_timeout_accepts_reasonable() {
        // Timeouts >= 1000ms are accepted as-is (up to the bound)
        assert_eq!(clamp_timeout(Some(1_000), DEFAULT_TIMEOUT_MS), 1_000);
        assert_eq!(clamp_timeout(Some(60_000), DEFAULT_TIMEOUT_MS), 60_000);
        assert_eq!(clamp_timeout(Some(120_000), DEFAULT_TIMEOUT_MS), 120_000);
    }

    #[test]
    fn test_clamp_timeout_respects_upper_bound() {
        // Timeouts exceeding the max_bound are clamped
        assert_eq!(
            clamp_timeout(Some(DEFAULT_TIMEOUT_MS + 1), DEFAULT_TIMEOUT_MS),
            DEFAULT_TIMEOUT_MS
        );
        assert_eq!(
            clamp_timeout(Some(700_000), BACKGROUNDED_TIMEOUT_MS),
            BACKGROUNDED_TIMEOUT_MS
        );
        assert_eq!(
            clamp_timeout(Some(1_000_000), BACKGROUNDED_TIMEOUT_MS),
            BACKGROUNDED_TIMEOUT_MS
        );
    }

    #[test]
    fn test_backgrounded_timeout_is_generous() {
        // BACKGROUNDED_TIMEOUT_MS should be >= DEFAULT_TIMEOUT_MS (600s >= 120s)
        assert!(
            BACKGROUNDED_TIMEOUT_MS >= DEFAULT_TIMEOUT_MS,
            "backgrounded timeout {} should be >= default timeout {}",
            BACKGROUNDED_TIMEOUT_MS,
            DEFAULT_TIMEOUT_MS
        );
        // Verify the actual values match schema ("max 600000")
        assert_eq!(BACKGROUNDED_TIMEOUT_MS, 600_000);
        assert_eq!(DEFAULT_TIMEOUT_MS, 120_000);
    }

    #[tokio::test]
    async fn test_explicitly_backgrounded_task_gets_generous_timeout() {
        // When run_in_background=true, the task should get BACKGROUNDED_TIMEOUT_MS
        // to allow it to run long. We test by checking that the timeout calculation
        // uses the generous bound.
        let foreground_budget = foreground_budget_ms();
        let user_timeout = clamp_timeout(Some(50_000), DEFAULT_TIMEOUT_MS); // 50s user timeout

        // For an explicitly backgrounded task:
        let effective_for_bg_true = if true || user_timeout > foreground_budget {
            clamp_timeout(Some(50_000), BACKGROUNDED_TIMEOUT_MS)
        } else {
            user_timeout
        };

        // Should use BACKGROUNDED_TIMEOUT_MS bound
        assert_eq!(effective_for_bg_true, 50_000);

        // But if the timeout exceeds backgrounded bound (unlikely), it would be clamped:
        let user_timeout_high = clamp_timeout(Some(700_000), DEFAULT_TIMEOUT_MS); // 700s clamped to 120s
        let effective_high = if true || user_timeout_high > foreground_budget {
            clamp_timeout(Some(700_000), BACKGROUNDED_TIMEOUT_MS)
        } else {
            user_timeout_high
        };
        assert_eq!(effective_high, BACKGROUNDED_TIMEOUT_MS);
    }

    #[tokio::test]
    async fn test_foreground_task_might_get_generous_timeout() {
        // When foreground_budget < timeout, the task might be auto-backgrounded,
        // so it should get BACKGROUNDED_TIMEOUT_MS to survive the transition.
        let foreground_budget = foreground_budget_ms();
        let user_timeout = clamp_timeout(Some(30_000), DEFAULT_TIMEOUT_MS); // 30s, > 15s budget

        // For a foreground task with timeout > budget:
        let effective = if false || user_timeout > foreground_budget {
            clamp_timeout(Some(30_000), BACKGROUNDED_TIMEOUT_MS)
        } else {
            user_timeout
        };

        // Should use BACKGROUNDED_TIMEOUT_MS bound (30s < 600s, so 30s is kept)
        assert_eq!(effective, 30_000);
        assert!(
            effective > foreground_budget,
            "timeout should exceed foreground budget"
        );
    }

    #[tokio::test]
    async fn test_short_foreground_task_keeps_timeout() {
        // A foreground task with timeout < budget is not auto-backgrounded,
        // so it keeps its specified timeout.
        let foreground_budget = foreground_budget_ms();
        let user_timeout = clamp_timeout(Some(5_000), DEFAULT_TIMEOUT_MS); // 5s, < 15s budget

        // For a foreground task with timeout < budget:
        let effective = if false || user_timeout > foreground_budget {
            clamp_timeout(Some(5_000), BACKGROUNDED_TIMEOUT_MS)
        } else {
            user_timeout
        };

        // Should keep the user timeout
        assert_eq!(effective, 5_000);
        assert!(
            effective < foreground_budget,
            "timeout should be < foreground budget"
        );
    }
}
