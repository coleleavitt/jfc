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

use super::logs::{append_log_line, background_agent_log_path};
use super::registry::{
    record_background_agent_finished_at_epoch, record_background_agent_heartbeat,
    record_background_agent_log, record_background_agent_log_at_epoch,
    record_background_agent_progress_at_epoch, record_background_agent_started_at,
};
use super::state::{
    BackgroundAgentLaunch, BackgroundAgentStatus, DaemonPaths, load_state, save_state,
    with_state_lock,
};

pub fn mark_background_agent_spawn_failed(
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

pub fn spawn_background_agent_worker(launch: BackgroundAgentLaunch) -> std::io::Result<u32> {
    jfc_daemon::spawn_background_agent_worker(launch)
}

pub fn record_background_agent_launch_path(
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

    let worker_epoch = launch.worker_epoch;
    if worker_epoch != 0 && !record_background_agent_heartbeat(&launch.task_id, worker_epoch) {
        append_log_line(
            &background_agent_log_path(&paths, &launch.task_id),
            &format!("[worker-superseded] epoch={worker_epoch} before start"),
        );
        return Ok(());
    }

    let provider_init = crate::runtime::bootstrap::build_providers();
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
        .or_else(|| {
            crate::runtime::bootstrap::provider_for_model(
                &provider_init.providers,
                launch.model.as_str(),
            )
        })
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
            "[worker-running] pid={} epoch={} provider={} cwd={}",
            std::process::id(),
            worker_epoch,
            provider.name(),
            launch.cwd.display()
        ),
    );

    let heartbeat_task_id = launch.task_id.clone();
    let heartbeat_log_path = background_agent_log_path(&paths, &launch.task_id);
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(5));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            interval.tick().await;
            if !record_background_agent_heartbeat(&heartbeat_task_id, worker_epoch) {
                append_log_line(
                    &heartbeat_log_path,
                    &format!("[worker-superseded] epoch={worker_epoch}; exiting"),
                );
                std::process::exit(0);
            }
        }
    });

    let (worktree_info, cwd_override) = match prepare_background_worktree(&launch).await {
        BackgroundIsolation::Proceed(wt, cwd) => (wt, cwd),
        BackgroundIsolation::FailClosed(msg) => {
            // Isolation requested, creation failed, fail-closed policy: record
            // the spawn failure and exit rather than mutate the main checkout.
            mark_background_agent_spawn_failed(&paths, &launch.task_id, &msg)?;
            return Ok(());
        }
    };
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
    // Audit: a background (daemon-driven) agent job has started. Tagged with
    // the task + originating session so the ledger answers "what background
    // work ran, for which session".
    crate::changeset::record_daemon_job(
        &launch.task_id,
        &launch.task_input.description,
        launch.parent_session_id.clone(),
    );

    let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::runtime::EngineEvent>(512);
    let event_task_id = launch.task_id.clone();
    let event_collector = tokio::spawn(async move {
        while let Some(event) = rx.recv().await {
            match event {
                crate::runtime::EngineEvent::Task(crate::runtime::TaskEvent::AgentChunk {
                    task_id,
                    text,
                }) if task_id.as_str() == event_task_id => {
                    let _ =
                        record_background_agent_log_at_epoch(&event_task_id, worker_epoch, &text);
                }
                crate::runtime::EngineEvent::Task(crate::runtime::TaskEvent::Progress {
                    task_id,
                    last_tool,
                    tool_use_count,
                    input_tokens,
                    cache_read_tokens,
                    cache_write_tokens,
                    output_tokens,
                    ..
                }) if task_id.as_str() == event_task_id => {
                    let _ = record_background_agent_progress_at_epoch(
                        &event_task_id,
                        worker_epoch,
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

    // Background workers run in their own process, so they need their own
    // TaskStore handle to honour TaskUpdate/TaskDone/TaskList. In team mode
    // that's the shared team store; otherwise it's the parent session's
    // store (`~/.config/jfc/tasks/<session>.json`). Without the non-team
    // branch, detached agents in a normal session got `None` and every
    // task tool failed with "Task store not available".
    let task_store = match (
        launch.active_team_name.as_deref(),
        launch.parent_session_id.as_deref(),
    ) {
        (Some(team), _) => Some(jfc_session::TaskStore::open_team(team)),
        (None, Some(session_id)) => Some(jfc_session::TaskStore::open(session_id)),
        (None, None) => None,
    };
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
    heartbeat.abort();
    let _ = heartbeat.await;

    let elapsed_ms = started.elapsed().as_millis() as u64;
    finish_background_worktree(&launch.task_id, worktree_info).await;
    if result.is_error() {
        let was_cancelled = result
            .output
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("cancelled:");
        let recorded = record_background_agent_finished_at_epoch(
            &launch.task_id,
            worker_epoch,
            if was_cancelled {
                BackgroundAgentStatus::Cancelled
            } else {
                BackgroundAgentStatus::Failed
            },
            &result.output,
        );
        if !recorded {
            append_log_line(
                &background_agent_log_path(&paths, &launch.task_id),
                &format!("[worker-superseded] epoch={worker_epoch}; skipped failed result"),
            );
        }
    } else {
        let recorded = record_background_agent_finished_at_epoch(
            &launch.task_id,
            worker_epoch,
            BackgroundAgentStatus::Completed,
            &result.output,
        );
        if !recorded {
            append_log_line(
                &background_agent_log_path(&paths, &launch.task_id),
                &format!("[worker-superseded] epoch={worker_epoch}; skipped completed result"),
            );
        }
    }
    append_log_line(
        &background_agent_log_path(&paths, &launch.task_id),
        &format!("[worker-exited] elapsed_ms={elapsed_ms}"),
    );
    Ok(())
}

type BackgroundWorktree = (crate::worktrees::WorktreeInfo, PathBuf, Option<String>);

/// Outcome of attempting worktree isolation for a background agent.
enum BackgroundIsolation {
    /// No isolation requested, or isolation failed but the policy allows the
    /// cwd fall-back. Carries the optional worktree handle + cwd override.
    Proceed(Option<BackgroundWorktree>, Option<PathBuf>),
    /// Isolation was requested, creation failed, and the policy is
    /// fail-closed: the worker must NOT run in the main checkout.
    FailClosed(String),
}

async fn prepare_background_worktree(launch: &BackgroundAgentLaunch) -> BackgroundIsolation {
    if launch.task_input.isolation.as_deref() != Some("worktree") {
        return BackgroundIsolation::Proceed(None, None);
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
            // Open a change-set so the background agent's isolated run is a
            // durable, reviewable proposal — same as the foreground Task path.
            let origin = crate::changeset::ChangeOrigin {
                task_id: Some(launch.task_id.clone()),
                agent_id: launch
                    .task_input
                    .subagent_type
                    .clone()
                    .or_else(|| Some("background".to_string())),
                session_id: launch.parent_session_id.clone(),
            };
            let change_id =
                crate::changeset::open_for_worktree(&repo_root, &info.path, &info.branch, &origin)
                    .await;
            BackgroundIsolation::Proceed(Some((info, repo_root, change_id)), Some(path))
        }
        Err(e) => match crate::changeset::isolation_fallback() {
            crate::changeset::IsolationFallback::FailClosed => {
                let msg = format!(
                    "[worktree] creation failed ({e}); isolation is fail-closed — \
                     refusing to run in the main checkout"
                );
                record_background_agent_log(&launch.task_id, &msg);
                BackgroundIsolation::FailClosed(msg)
            }
            crate::changeset::IsolationFallback::AllowCwd => {
                record_background_agent_log(
                    &launch.task_id,
                    &format!("[worktree] failed to create worktree: {e}; using cwd (fail-open)"),
                );
                BackgroundIsolation::Proceed(None, None)
            }
        },
    }
}

async fn finish_background_worktree(task_id: &str, worktree_info: Option<BackgroundWorktree>) {
    let Some((wt, repo_root, change_id)) = worktree_info else {
        return;
    };
    // Finalize the change-set (diff vs base → Ready, or Abandoned if clean)
    // while the worktree still exists to diff against.
    if let Some(ref cid) = change_id {
        crate::changeset::finalize_for_worktree(&repo_root, cid, &wt.path).await;
    }
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
