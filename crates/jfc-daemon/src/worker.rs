//! Detached background-agent worker — spawn path and worker entry point.
//!
//! # Layout
//!
//! - Worker binary resolution: `resolve_worker_exe` tries persisted exe →
//!   `JFC_WORKER_BIN` → `current_exe` → workspace `target/{release,debug}`
//!   → `PATH` → `cargo build` rebuild. Shell aliases are intentionally
//!   ignored: `Command::spawn` cannot see them.
//! - Spawn (`spawn_background_agent_worker_with_paths`): persists the launch
//!   spec in the DB, records a roster entry, forks a detached `jfc daemon
//!   worker --launch <handle>` process (setsid on Unix), captures its PID.
//! - Entry (`run_background_agent_worker`): the worker process re-enters
//!   here, rebuilds providers, prepares a worktree if requested, drives
//!   `tools::execute_task`, and writes terminal state to the daemon roster.
//! - Lifecycle helpers (`mark_background_agent_spawn_failed`,
//!   `record_background_agent_worker_pid`, `reap_worker_process`) are
//!   `pub(super)` so `reconcile` can use them on respawn.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::logs::{append_log_line, background_agent_launch_path, background_agent_log_path};
use super::registry::record_background_agent_started_at;
use super::state::{
    BackgroundAgentInfo, BackgroundAgentLaunch, BackgroundAgentStatus, DaemonPaths,
    load_state_for_update, save_state, with_state_lock,
};

const BACKGROUND_AGENT_LAUNCH_SESSION_ID: &str = "__daemon__";
const BACKGROUND_AGENT_LAUNCH_KIND: &str = "background_agent_launch";

pub fn load_background_agent_launch(
    paths: &DaemonPaths,
    launch_path: &Path,
) -> std::io::Result<BackgroundAgentLaunch> {
    let key = launch_path.display().to_string();
    let store = if paths.base_dir == DaemonPaths::default_user().base_dir {
        jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open_default())
    } else {
        std::fs::create_dir_all(&paths.base_dir)?;
        jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open(
            &paths.base_dir.join("knowledge.db"),
        ))
    }
    .map_err(std::io::Error::other)?;
    let row = jfc_knowledge::block_on_knowledge(async {
        store
            .get_session_artifact(
                BACKGROUND_AGENT_LAUNCH_SESSION_ID,
                BACKGROUND_AGENT_LAUNCH_KIND,
                &key,
            )
            .await
    })
    .map_err(std::io::Error::other)?
    .ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::NotFound,
            format!("background agent launch spec not found: {key}"),
        )
    })?;
    serde_json::from_str(&row.value_json).map_err(std::io::Error::other)
}

fn persist_background_agent_launch(
    paths: &DaemonPaths,
    launch_path: &Path,
    launch: &BackgroundAgentLaunch,
) -> std::io::Result<()> {
    let key = launch_path.display().to_string();
    let json = serde_json::to_string(launch).map_err(std::io::Error::other)?;
    let store = if paths.base_dir == DaemonPaths::default_user().base_dir {
        jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open_default())
    } else {
        std::fs::create_dir_all(&paths.base_dir)?;
        jfc_knowledge::block_on_knowledge(jfc_knowledge::KnowledgeStore::open(
            &paths.base_dir.join("knowledge.db"),
        ))
    }
    .map_err(std::io::Error::other)?;
    jfc_knowledge::block_on_knowledge(async {
        store
            .upsert_session_artifact(
                BACKGROUND_AGENT_LAUNCH_SESSION_ID,
                BACKGROUND_AGENT_LAUNCH_KIND,
                &key,
                &json,
            )
            .await
    })
    .map_err(std::io::Error::other)
}

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
        .arg("jfc")
        .arg("--bin")
        .arg("jfc")
        .current_dir(&root)
        .status()?;
    if !status.success() {
        return Err(std::io::Error::other(format!(
            "failed to rebuild background worker with `cargo build -p jfc --bin jfc` from {} (exit {:?})",
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

pub(crate) fn resolve_worker_exe(preferred: Option<&Path>) -> std::io::Result<PathBuf> {
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

/// Live reaper-thread handles. Each detached worker gets one short thread
/// that blocks on `child.wait()` to avoid leaving a zombie. Tracking the
/// handles here (instead of `let _ = spawn(...)`) keeps the thread count
/// bounded: finished handles are pruned on every new spawn, and a daemon
/// shutdown can join the rest via [`join_worker_reapers`].
static WORKER_REAPERS: std::sync::OnceLock<std::sync::Mutex<Vec<std::thread::JoinHandle<()>>>> =
    std::sync::OnceLock::new();

fn worker_reapers() -> &'static std::sync::Mutex<Vec<std::thread::JoinHandle<()>>> {
    WORKER_REAPERS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

pub(super) fn reap_worker_process(mut child: std::process::Child) {
    let pid = child.id();
    match std::thread::Builder::new()
        .name("jfc-worker-reaper".to_owned())
        .spawn(move || {
            let _ = child.wait();
        }) {
        Ok(handle) => {
            if let Ok(mut reapers) = worker_reapers().lock() {
                // Drop handles whose worker already exited so a long-lived
                // daemon that respawns many workers doesn't accumulate
                // thread handles without bound.
                reapers.retain(|h| !h.is_finished());
                reapers.push(handle);
            }
        }
        Err(e) => {
            // The child still gets reaped by init once we exit, but a
            // failure to spawn the reaper is unexpected and worth a trace.
            tracing::warn!(
                target: "jfc::daemon::worker",
                error = %e,
                pid,
                "failed to spawn worker reaper thread; relying on init to reap"
            );
        }
    }
}

/// Join any finished reaper threads and drop their handles. Best-effort:
/// only threads whose worker has already exited are joined, so this never
/// blocks on a still-running worker. Call on daemon shutdown to clean up.
pub(super) fn join_worker_reapers() {
    let Ok(mut reapers) = worker_reapers().lock() else {
        return;
    };
    let mut still_running = Vec::new();
    for handle in reapers.drain(..) {
        if handle.is_finished() {
            let _ = handle.join();
        } else {
            still_running.push(handle);
        }
    }
    *reapers = still_running;
}

pub(super) fn record_background_agent_worker_pid(
    paths: &DaemonPaths,
    id: &str,
    pid: u32,
    launch_path: &Path,
    worker_epoch: u64,
) -> std::io::Result<()> {
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state_for_update(paths)?;
        let Some(agent) = state.background_agents.get_mut(id) else {
            return Ok(());
        };
        if worker_epoch != 0 && agent.worker_epoch != worker_epoch {
            return Ok(());
        }
        let now = SystemTime::now();
        agent.pid = Some(pid);
        agent.launch_path = Some(launch_path.to_path_buf());
        agent.status = BackgroundAgentStatus::Running;
        agent.updated_at = now;
        agent.last_heartbeat_at = Some(now);
        save_state(paths, &state)
    })
}

pub(super) fn arm_worker_launch_epoch(
    paths: &DaemonPaths,
    launch_path: &Path,
    mut launch: BackgroundAgentLaunch,
    takeover: bool,
) -> std::io::Result<BackgroundAgentLaunch> {
    let (worker_epoch, runtime_worker_exe) = with_state_lock(paths, || -> std::io::Result<_> {
        let mut state = load_state_for_update(paths)?;
        let Some(agent) = state.background_agents.get_mut(&launch.task_id) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("background agent not found: {}", launch.task_id),
            ));
        };
        let next_epoch = agent.worker_epoch.saturating_add(1).max(1);
        let now = SystemTime::now();
        agent.worker_epoch = next_epoch;
        agent.launch_path = Some(launch_path.to_path_buf());
        agent.updated_at = now;
        agent.last_heartbeat_at = None;
        if takeover {
            agent.takeover_count = agent.takeover_count.saturating_add(1);
            agent.pid = None;
        }
        let runtime_worker_exe = state.runtime.worker_exe.clone();
        save_state(paths, &state)?;
        Ok((next_epoch, runtime_worker_exe))
    })?;

    launch.worker_epoch = worker_epoch;
    if let Some(worker_exe) = runtime_worker_exe {
        launch.worker_exe = Some(worker_exe);
    }
    persist_background_agent_launch(paths, launch_path, &launch)?;
    Ok(launch)
}

pub(super) fn mark_background_agent_spawn_failed(
    paths: &DaemonPaths,
    id: &str,
    error: &str,
) -> std::io::Result<()> {
    let log_path = with_state_lock(paths, || -> std::io::Result<Option<PathBuf>> {
        let mut state = load_state_for_update(paths)?;
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
    let task_id = launch.task_id.clone();
    let launch = match arm_worker_launch_epoch(paths, &launch_path, launch, false) {
        Ok(launch) => launch,
        Err(e) => {
            let _ = mark_background_agent_spawn_failed(paths, &task_id, &e.to_string());
            return Err(e);
        }
    };

    match spawn_worker_process(&launch_path, &launch) {
        Ok(child) => {
            let pid = child.id();
            record_background_agent_worker_pid(
                paths,
                &launch.task_id,
                pid,
                &launch_path,
                launch.worker_epoch,
            )?;
            reap_worker_process(child);
            append_log_line(
                &background_agent_log_path(paths, &launch.task_id),
                &format!("[worker-started] pid={pid} epoch={}", launch.worker_epoch),
            );
            Ok(pid)
        }
        Err(e) => {
            let _ = mark_background_agent_spawn_failed(paths, &launch.task_id, &e.to_string());
            Err(e)
        }
    }
}

pub(super) fn takeover_background_agent_worker_with_paths(
    paths: &DaemonPaths,
    agent_id: &str,
    force: bool,
    reason: &str,
) -> std::io::Result<u32> {
    let (launch_path, launch, previous_pid) = with_state_lock(paths, || -> std::io::Result<_> {
        let state = load_state_for_update(paths)?;
        let agent = state.background_agents.get(agent_id).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("background agent not found: {agent_id}"),
            )
        })?;
        if agent.status != BackgroundAgentStatus::Running {
            return Err(std::io::Error::other(format!(
                "background agent {agent_id} is not running"
            )));
        }
        if !force
            && let Some(pid) = agent.pid
            && super::pid::process_is_running(pid)
        {
            return Err(std::io::Error::other(format!(
                "background agent {agent_id} owner pid {pid} is still running; pass --force to take over anyway"
            )));
        }
        let launch_path = agent.launch_path.clone().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("background agent {agent_id} has no launch spec"),
            )
        })?;
        let launch = load_background_agent_launch(paths, &launch_path)?;
        Ok((launch_path, launch, agent.pid))
    })?;

    let launch = arm_worker_launch_epoch(paths, &launch_path, launch, true)?;
    let child = spawn_worker_process(&launch_path, &launch)?;
    let pid = child.id();
    record_background_agent_worker_pid(paths, agent_id, pid, &launch_path, launch.worker_epoch)?;
    reap_worker_process(child);
    append_log_line(
        &background_agent_log_path(paths, agent_id),
        &format!(
            "[worker-takeover] pid={pid} previous_pid={previous_pid:?} epoch={} reason={reason}",
            launch.worker_epoch
        ),
    );
    Ok(pid)
}

/// Top-level entry called from `event_loop`/`stream` to launch a detached
/// background worker for a Task.
/// Maximum number of concurrently-running detached background agents.
/// By default there is no limit — the agent can fire off as many background
/// workers as it wants. Set `JFC_MAX_BACKGROUND_AGENTS` to cap it.
fn max_running_agents() -> usize {
    std::env::var("JFC_MAX_BACKGROUND_AGENTS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .filter(|&n| n > 0)
        .unwrap_or(usize::MAX)
}

pub fn spawn_background_agent_worker(launch: BackgroundAgentLaunch) -> std::io::Result<u32> {
    let paths = DaemonPaths::default_user();
    let cap = max_running_agents();
    // Enforce concurrency cap before forking a new worker. The count
    // check AND the slot reservation happen under a single state lock so
    // two concurrent callers can't both observe `count < cap` and then
    // both spawn (TOCTOU). We reserve by inserting a Running record for
    // this task up front; a sibling caller will count it immediately.
    // `spawn_*_with_paths` later overwrites this record with full
    // metadata, and `mark_background_agent_spawn_failed` flips it to
    // Failed (no longer counted) if the spawn errors out — that's the
    // rollback path.
    let reserved = with_state_lock(&paths, || -> std::io::Result<bool> {
        let mut state = load_state_for_update(&paths)?;
        let running_count = state
            .background_agents
            .values()
            .filter(|a| a.status == BackgroundAgentStatus::Running)
            .count();
        if running_count >= cap {
            return Ok(false);
        }
        let now = SystemTime::now();
        state.background_agents.insert(
            launch.task_id.clone(),
            BackgroundAgentInfo {
                id: launch.task_id.clone(),
                description: launch.task_input.description.clone(),
                parent_session_id: launch.parent_session_id.clone(),
                status: BackgroundAgentStatus::Running,
                started_at: now,
                updated_at: now,
                completed_at: None,
                pid: None,
                worker_epoch: 0,
                last_heartbeat_at: None,
                takeover_count: 0,
                model: Some(launch.model.as_str().to_owned()),
                worktree_path: None,
                log_path: background_agent_log_path(&paths, &launch.task_id),
                launch_path: None,
                cancel_requested: false,
                respawn_count: 0,
                summary: None,
                error: None,
                tool_use_count: 0,
                latest_input_tokens: 0,
                latest_cache_read_tokens: 0,
                latest_cache_write_tokens: 0,
                cumulative_output_tokens: 0,
                last_tool: None,
                last_tool_info: None,
            },
        );
        save_state(&paths, &state)?;
        Ok(true)
    })?;
    if !reserved {
        tracing::warn!(
            target: "jfc::daemon::worker",
            cap,
            "background agent spawn rejected — at capacity"
        );
        return Err(std::io::Error::new(
            std::io::ErrorKind::ResourceBusy,
            format!(
                "Cannot spawn background agent: {cap}/{cap} already running. \
                 Wait for one to finish or set JFC_MAX_BACKGROUND_AGENTS higher."
            ),
        ));
    }
    let task_id = launch.task_id.clone();
    let result = spawn_background_agent_worker_with_paths(&paths, launch);
    if result.is_err() {
        // Spawn failed after reservation — release the slot so it doesn't
        // leak as a phantom Running agent forever.
        let _ = with_state_lock(&paths, || -> std::io::Result<()> {
            let mut state = load_state_for_update(&paths)?;
            if let Some(agent) = state.background_agents.get_mut(&task_id)
                && agent.status == BackgroundAgentStatus::Running
                && agent.pid.is_none()
            {
                agent.status = BackgroundAgentStatus::Failed;
                agent.completed_at = Some(SystemTime::now());
            }
            save_state(&paths, &state)
        });
    }
    result
}

pub(super) fn record_background_agent_launch_path(
    paths: &DaemonPaths,
    id: &str,
    launch_path: &Path,
) -> std::io::Result<()> {
    with_state_lock(paths, || -> std::io::Result<()> {
        let mut state = load_state_for_update(paths)?;
        if let Some(agent) = state.background_agents.get_mut(id) {
            agent.launch_path = Some(launch_path.to_path_buf());
            agent.updated_at = SystemTime::now();
        }
        save_state(paths, &state)
    })
}

#[cfg(test)]
mod reaper_tests {
    use super::{join_worker_reapers, reap_worker_process, worker_reapers};

    // A reaped short-lived child is tracked, then dropped from the registry
    // once it exits (pruned on the next spawn) and joined on shutdown.
    #[test]
    fn reap_then_join_drains_handles_normal() {
        // Clear any handles left by other tests in this binary.
        join_worker_reapers();

        let child = std::process::Command::new("true")
            .spawn()
            .expect("spawn `true`");
        reap_worker_process(child);

        // Let the child exit and the reaper thread finish.
        std::thread::sleep(std::time::Duration::from_millis(200));

        join_worker_reapers();
        let remaining = worker_reapers().lock().unwrap().len();
        assert_eq!(remaining, 0, "finished reaper handles should be drained");
    }

    // Pruning keeps the registry bounded across many spawns rather than
    // growing once per lifetime worker.
    #[test]
    fn reaper_registry_prunes_finished_robust() {
        join_worker_reapers();
        for _ in 0..8 {
            let child = std::process::Command::new("true")
                .spawn()
                .expect("spawn `true`");
            reap_worker_process(child);
            std::thread::sleep(std::time::Duration::from_millis(40));
        }
        // After the children exit, the next prune (or join) collapses the vec.
        std::thread::sleep(std::time::Duration::from_millis(150));
        join_worker_reapers();
        assert_eq!(worker_reapers().lock().unwrap().len(), 0);
    }
}
