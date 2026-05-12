//! Detached background-agent worker — spawn path and worker entry point.
//!
//! # Layout
//!
//! - Worker binary resolution: `resolve_worker_exe` tries persisted exe →
//!   `JFC_WORKER_BIN` → `current_exe` → workspace `target/{release,debug}`
//!   → `PATH` → `cargo build` rebuild. Shell aliases are intentionally
//!   ignored: `Command::spawn` cannot see them.
//! - Spawn (`spawn_background_agent_worker_with_paths`): writes the launch
//!   spec, records a roster entry, forks a detached `jfc daemon worker
//!   --launch <path>` process (setsid on Unix), captures its PID.
//! - Entry (`run_background_agent_worker`): the worker process re-enters
//!   here, rebuilds providers, prepares a worktree if requested, drives
//!   `tools::execute_task`, and writes terminal state to the daemon roster.
//! - Lifecycle helpers (`mark_background_agent_spawn_failed`,
//!   `record_background_agent_worker_pid`, `reap_worker_process`) are
//!   `pub(super)` so `reconcile` can use them on respawn.

use std::path::{Path, PathBuf};
use std::time::{Instant, SystemTime};

use super::logs::{append_log_line, background_agent_launch_path, background_agent_log_path};
use super::registry::{
    record_background_agent_finished, record_background_agent_log,
    record_background_agent_progress, record_background_agent_started_at,
};
use super::state::{
    BackgroundAgentLaunch, BackgroundAgentStatus, DaemonPaths, load_state, save_state,
    with_state_lock,
};

fn path_is_executable_file(path: &Path) -> bool {
    path.is_file()
}

fn find_worker_exe_on_path() -> Option<PathBuf> {
    let path_var = std::env::var_os("PATH")?;
    std::env::split_paths(&path_var)
        .map(|dir| dir.join("jfc"))
        .find(|candidate| path_is_executable_file(candidate))
}

fn workspace_root_from_manifest_dir() -> Option<PathBuf> {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir.parent()?.parent().map(Path::to_path_buf)
}

pub(super) fn worker_exe_workspace_candidates() -> Vec<PathBuf> {
    let Some(root) = workspace_root_from_manifest_dir() else {
        return Vec::new();
    };
    vec![
        root.join("target").join("release").join("jfc"),
        root.join("target").join("debug").join("jfc"),
    ]
}

fn build_worker_exe_from_workspace() -> std::io::Result<Option<PathBuf>> {
    let Some(root) = workspace_root_from_manifest_dir() else {
        return Ok(None);
    };
    if !root.join("Cargo.toml").is_file() {
        return Ok(None);
    }

    let status = std::process::Command::new("cargo")
        .arg("build")
        .arg("-p")
        .arg("jfc-ui")
        .arg("--bin")
        .arg("jfc")
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "failed to rebuild background worker with `cargo build -p jfc-ui --bin jfc` from {} (exit {:?})",
            root.display(),
            status.code()
        )));
    }

    let candidate = root.join("target").join("debug").join("jfc");
    if path_is_executable_file(&candidate) {
        Ok(Some(candidate))
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "cargo build completed but background worker binary was not produced at {}",
                candidate.display()
            ),
        ))
    }
}

fn resolve_worker_exe(preferred: Option<&Path>) -> std::io::Result<PathBuf> {
    if let Some(path) = preferred
        && path_is_executable_file(path)
    {
        return Ok(path.to_path_buf());
    }

    if let Ok(path) = std::env::var("JFC_WORKER_BIN") {
        let path = PathBuf::from(path);
        if path_is_executable_file(&path) {
            return Ok(path);
        }
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "background worker executable from JFC_WORKER_BIN does not exist: {}",
                path.display()
            ),
        ));
    }

    let current_exe = std::env::current_exe()?;
    if path_is_executable_file(&current_exe) {
        return Ok(current_exe);
    }

    for candidate in worker_exe_workspace_candidates() {
        if path_is_executable_file(&candidate) {
            return Ok(candidate);
        }
    }

    if let Some(path_exe) = find_worker_exe_on_path() {
        return Ok(path_exe);
    }

    if let Some(built_exe) = build_worker_exe_from_workspace()? {
        return Ok(built_exe);
    }

    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        format!(
            "background worker executable not found; current_exe={} no longer exists, PATH did not contain jfc, and workspace rebuild was unavailable. Rebuild/install jfc or set JFC_WORKER_BIN=/absolute/path/to/jfc",
            current_exe.display()
        ),
    ))
}

pub(super) fn validate_worker_spawn_inputs(exe: &Path, cwd: &Path) -> std::io::Result<()> {
    if !path_is_executable_file(exe) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!(
                "background worker executable does not exist: {}",
                exe.display()
            ),
        ));
    }
    if !cwd.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("background worker cwd does not exist: {}", cwd.display()),
        ));
    }
    Ok(())
}

pub(super) fn spawn_worker_process(
    launch_path: &Path,
    launch: &BackgroundAgentLaunch,
) -> std::io::Result<std::process::Child> {
    let exe = resolve_worker_exe(launch.worker_exe.as_deref())?;
    validate_worker_spawn_inputs(&exe, &launch.cwd)?;
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("daemon")
        .arg("worker")
        .arg("--launch")
        .arg(launch_path)
        .current_dir(&launch.cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // Detach from the TUI's controlling process group so closing the UI
        // does not SIGHUP the background worker.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    Err(std::io::Error::last_os_error())
                } else {
                    Ok(())
                }
            });
        }
    }

    cmd.spawn()
}

pub(super) fn reap_worker_process(mut child: std::process::Child) {
    let _ = std::thread::Builder::new()
        .name("jfc-worker-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        });
}

pub(super) fn record_background_agent_worker_pid(
    paths: &DaemonPaths,
    id: &str,
    pid: u32,
    launch_path: &Path,
) -> std::io::Result<()> {
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state(paths).unwrap_or_default();
        let Some(agent) = state.background_agents.get_mut(id) else {
            return Ok(());
        };
        agent.pid = Some(pid);
        agent.launch_path = Some(launch_path.to_path_buf());
        agent.status = BackgroundAgentStatus::Running;
        agent.updated_at = SystemTime::now();
        save_state(paths, &state)
    })
}

pub(super) fn mark_background_agent_spawn_failed(
    paths: &DaemonPaths,
    id: &str,
    error: &str,
) -> std::io::Result<()> {
    let log_path = with_state_lock(paths, || -> std::io::Result<Option<PathBuf>> {
        let mut state = load_state(paths).unwrap_or_default();
        let now = SystemTime::now();
        let Some(agent) = state.background_agents.get_mut(id) else {
            return Ok(None);
        };
        agent.status = BackgroundAgentStatus::Failed;
        agent.updated_at = now;
        agent.completed_at = Some(now);
        agent.error = Some(error.to_owned());
        let log_path = agent.log_path.clone();
        save_state(paths, &state)?;
        Ok(Some(log_path))
    })?;
    if let Some(log_path) = log_path {
        append_log_line(&log_path, &format!("[Failed] {error}"));
    }
    Ok(())
}

pub(super) fn spawn_background_agent_worker_with_paths(
    paths: &DaemonPaths,
    mut launch: BackgroundAgentLaunch,
) -> std::io::Result<u32> {
    paths.ensure_dirs()?;
    let launch_path = background_agent_launch_path(paths, &launch.task_id);

    record_background_agent_started_at(
        paths,
        &launch.task_id,
        &launch.task_input.description,
        launch.parent_session_id.clone(),
        Some(launch.model.as_str().to_owned()),
        None,
        None,
    );
    record_background_agent_launch_path(paths, &launch.task_id, &launch_path)?;

    let worker_exe = match resolve_worker_exe(launch.worker_exe.as_deref()) {
        Ok(worker_exe) => worker_exe,
        Err(e) => {
            let _ = mark_background_agent_spawn_failed(paths, &launch.task_id, &e.to_string());
            return Err(e);
        }
    };
    if let Err(e) = validate_worker_spawn_inputs(&worker_exe, &launch.cwd) {
        let _ = mark_background_agent_spawn_failed(paths, &launch.task_id, &e.to_string());
        return Err(e);
    }
    launch.worker_exe = Some(worker_exe);
    let json = match serde_json::to_string_pretty(&launch).map_err(std::io::Error::other) {
        Ok(json) => json,
        Err(e) => {
            let _ = mark_background_agent_spawn_failed(paths, &launch.task_id, &e.to_string());
            return Err(e);
        }
    };
    if let Err(e) = std::fs::write(&launch_path, json) {
        let _ = mark_background_agent_spawn_failed(paths, &launch.task_id, &e.to_string());
        return Err(e);
    }

    match spawn_worker_process(&launch_path, &launch) {
        Ok(child) => {
            let pid = child.id();
            record_background_agent_worker_pid(paths, &launch.task_id, pid, &launch_path)?;
            reap_worker_process(child);
            append_log_line(
                &background_agent_log_path(paths, &launch.task_id),
                &format!("[worker-started] pid={pid}"),
            );
            Ok(pid)
        }
        Err(e) => {
            let _ = mark_background_agent_spawn_failed(paths, &launch.task_id, &e.to_string());
            Err(e)
        }
    }
}

/// Top-level entry called from `event_loop`/`stream` to launch a detached
/// background worker for a Task.
pub fn spawn_background_agent_worker(launch: BackgroundAgentLaunch) -> std::io::Result<u32> {
    let paths = DaemonPaths::default_user();
    spawn_background_agent_worker_with_paths(&paths, launch)
}

pub(super) fn record_background_agent_launch_path(
    paths: &DaemonPaths,
    id: &str,
    launch_path: &Path,
) -> std::io::Result<()> {
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state(paths).unwrap_or_default();
        if let Some(agent) = state.background_agents.get_mut(id) {
            agent.launch_path = Some(launch_path.to_path_buf());
            agent.updated_at = SystemTime::now();
        }
        save_state(paths, &state)
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Worker-side entry: re-enter from `jfc daemon worker --launch <path>`,
/// rebuild providers, drive `execute_task`, and write terminal state.
pub async fn run_background_agent_worker(launch_path: PathBuf) -> std::io::Result<()> {
    let launch_json = std::fs::read_to_string(&launch_path)?;
    let launch: BackgroundAgentLaunch =
        serde_json::from_str(&launch_json).map_err(std::io::Error::other)?;
    let paths = DaemonPaths::default_user();
    paths.ensure_dirs()?;

    if let Err(e) = std::env::set_current_dir(&launch.cwd) {
        let msg = format!("worker failed to enter cwd {}: {e}", launch.cwd.display());
        mark_background_agent_spawn_failed(&paths, &launch.task_id, &msg)?;
        return Err(e);
    }

    let provider_init = crate::build_providers();
    let provider = launch
        .provider_name
        .as_deref()
        .and_then(|name| {
            provider_init
                .providers
                .iter()
                .find(|provider| provider.name() == name)
                .cloned()
        })
        .or_else(|| crate::provider_for_model(&provider_init.providers, launch.model.as_str()))
        .unwrap_or_else(|| provider_init.providers[provider_init.active_idx].clone());

    record_background_agent_started_at(
        &paths,
        &launch.task_id,
        &launch.task_input.description,
        launch.parent_session_id.clone(),
        Some(launch.model.as_str().to_owned()),
        None,
        Some(std::process::id()),
    );
    record_background_agent_launch_path(&paths, &launch.task_id, &launch_path)?;
    append_log_line(
        &background_agent_log_path(&paths, &launch.task_id),
        &format!(
            "[worker-running] pid={} provider={} cwd={}",
            std::process::id(),
            provider.name(),
            launch.cwd.display()
        ),
    );

    let (worktree_info, cwd_override) = prepare_background_worktree(&launch).await;
    if let Some(path) = &cwd_override {
        record_background_agent_started_at(
            &paths,
            &launch.task_id,
            &launch.task_input.description,
            launch.parent_session_id.clone(),
            Some(launch.model.as_str().to_owned()),
            Some(path.clone()),
            Some(std::process::id()),
        );
    }

    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::app::AppEvent>(512);
    let event_task_id = launch.task_id.clone();
    let event_collector = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                crate::app::AppEvent::AgentChunk { task_id, text }
                    if task_id.as_str() == event_task_id =>
                {
                    record_background_agent_log(&event_task_id, &text);
                }
                crate::app::AppEvent::TaskProgress {
                    task_id,
                    last_tool,
                    tool_use_count,
                    input_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    output_tokens,
                    ..
                } if task_id.as_str() == event_task_id => {
                    record_background_agent_progress(
                        &event_task_id,
                        last_tool.as_deref(),
                        tool_use_count,
                        input_tokens,
                        cache_read_tokens,
                        cache_write_tokens,
                        output_tokens,
                    );
                }
                _ => {}
            }
        }
    });

    let task_store = launch
        .active_team_name
        .as_deref()
        .map(crate::tasks::TaskStore::open_team);
    let started = Instant::now();
    let result = crate::tools::execute_task(
        &launch.task_input,
        provider.as_ref(),
        launch.model.clone(),
        Some(&tx),
        Some(&launch.task_id),
        launch.agent_def.as_ref(),
        cwd_override.clone(),
        task_store,
        launch.active_team_name.as_deref(),
    )
    .await;
    drop(tx);
    let _ = event_collector.await;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    finish_background_worktree(&launch.task_id, worktree_info).await;
    if result.is_error() {
        let was_cancelled = result
            .output
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("cancelled:");
        record_background_agent_finished(
            &launch.task_id,
            if was_cancelled {
                BackgroundAgentStatus::Cancelled
            } else {
                BackgroundAgentStatus::Failed
            },
            &result.output,
        );
    } else {
        record_background_agent_finished(
            &launch.task_id,
            BackgroundAgentStatus::Completed,
            &result.output,
        );
    }
    append_log_line(
        &background_agent_log_path(&paths, &launch.task_id),
        &format!("[worker-exited] elapsed_ms={elapsed_ms}"),
    );
    Ok(())
}

async fn prepare_background_worktree(
    launch: &BackgroundAgentLaunch,
) -> (
    Option<(crate::worktrees::WorktreeInfo, PathBuf)>,
    Option<PathBuf>,
) {
    if launch.task_input.isolation.as_deref() != Some("worktree") {
        return (None, None);
    }

    let name = format!(
        "agent-{}",
        launch
            .task_id
            .replace("toolu_", "")
            .chars()
            .take(8)
            .collect::<String>()
    );
    let repo_root = match crate::worktrees::find_repo_root_async(&launch.cwd).await {
        Ok(root) => root,
        Err(e) => {
            record_background_agent_log(
                &launch.task_id,
                &format!(
                    "[worktree] failed to resolve git root from {}: {e}; using cwd",
                    launch.cwd.display()
                ),
            );
            launch.cwd.clone()
        }
    };
    match crate::worktrees::create_worktree_async(&repo_root, &name).await {
        Ok(info) => {
            let path = PathBuf::from(&info.path);
            record_background_agent_log(
                &launch.task_id,
                &format!("[worktree] created {}", path.display()),
            );
            (Some((info, repo_root)), Some(path))
        }
        Err(e) => {
            record_background_agent_log(
                &launch.task_id,
                &format!("[worktree] failed to create worktree: {e}; using cwd"),
            );
            (None, None)
        }
    }
}

async fn finish_background_worktree(
    task_id: &str,
    worktree_info: Option<(crate::worktrees::WorktreeInfo, PathBuf)>,
) {
    let Some((wt, repo_root)) = worktree_info else {
        return;
    };
    let dirty = match tokio::process::Command::new("git")
        .arg("-C")
        .arg(&wt.path)
        .arg("status")
        .arg("--porcelain")
        .output()
        .await
    {
        Ok(out) if out.status.success() => !out.stdout.is_empty(),
        Ok(out) => {
            record_background_agent_log(
                task_id,
                &format!(
                    "[worktree] git status failed; preserving {}: {}",
                    wt.path,
                    String::from_utf8_lossy(&out.stderr)
                ),
            );
            true
        }
        Err(e) => {
            record_background_agent_log(
                task_id,
                &format!(
                    "[worktree] git status spawn failed; preserving {}: {e}",
                    wt.path
                ),
            );
            true
        }
    };
    if dirty {
        record_background_agent_log(
            task_id,
            &format!(
                "[worktree-preserved] path={} branch={} inspect=\"cd {} && git diff\"",
                wt.path, wt.branch, wt.path
            ),
        );
        return;
    }
    let wt_name = Path::new(&wt.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    match crate::worktrees::remove_worktree_async(&repo_root, wt_name).await {
        Ok(_) => {
            record_background_agent_log(task_id, &format!("[worktree-removed] path={}", wt.path))
        }
        Err(e) => record_background_agent_log(
            task_id,
            &format!("[worktree] cleanup failed for {}: {e}", wt.path),
        ),
    }
}
