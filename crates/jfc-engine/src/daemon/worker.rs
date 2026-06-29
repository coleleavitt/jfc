//! Detached background-agent worker — spawn path and worker entry point.
//!
//! # Layout
//!
//! - Worker binary resolution: `resolve_worker_exe` tries persisted exe →
//!   `JFC_WORKER_BIN` → `current_exe` → workspace `target/{release,debug}`
//!   → `PATH` → `cargo build` rebuild. Shell aliases are intentionally
//!   ignored: `Command::spawn` cannot see them.
//! - Spawn (`spawn_background_agent_worker_with_paths`): stores the launch
//!   spec in the DB, records a roster entry, forks a detached `jfc daemon
//!   worker --launch <handle>` process (setsid on Unix), captures its PID.
//! - Entry (`run_background_agent_worker`): the worker process re-enters
//!   here, rebuilds providers, prepares a worktree if requested, drives
//!   `tools::execute_task`, and writes terminal state to the daemon roster.
//! - Lifecycle helpers (`mark_background_agent_spawn_failed`,
//!   `record_background_agent_worker_pid`, `reap_worker_process`) are
//!   `pub(super)` so `reconcile` can use them on respawn.

use std::path::PathBuf;
use std::time::Instant;

use super::background_worktree::{
    BackgroundIsolation, finish_background_worktree, prepare_background_worktree,
};
use super::logs::{append_log_line, background_agent_log_path};
use super::registry::{
    record_background_agent_finished_at_epoch, record_background_agent_heartbeat,
    record_background_agent_log_at_epoch, record_background_agent_progress_at_epoch,
    record_background_agent_started_at,
};
use super::state::{BackgroundAgentLaunch, BackgroundAgentStatus, DaemonPaths};
use super::worker_mcp::register_background_worker_mcp_registry;
use super::worker_state::{
    mark_background_agent_spawn_failed, record_background_agent_launch_path,
};

pub fn spawn_background_agent_worker(launch: BackgroundAgentLaunch) -> std::io::Result<u32> {
    let _linkscope_spawn = linkscope::phase("engine.worker.spawn_background_agent_worker");
    linkscope::event_fields(
        "engine.worker.spawn_background_agent_worker",
        [
            linkscope::TraceField::text("task_id", launch.task_id.clone()),
            linkscope::TraceField::text("cwd", launch.cwd.display().to_string()),
            linkscope::TraceField::text("model", launch.model.as_str().to_owned()),
        ],
    );
    jfc_daemon::spawn_background_agent_worker(launch)
}

// ─────────────────────────────────────────────────────────────────────────────
// Worker entry point
// ─────────────────────────────────────────────────────────────────────────────

/// Worker-side entry: re-enter from `jfc daemon worker --launch <path>`,
/// rebuild providers, drive `execute_task`, and write terminal state.
pub async fn run_background_agent_worker(launch_path: PathBuf) -> std::io::Result<()> {
    let _linkscope_worker = linkscope::phase("engine.worker.run_background_agent_worker");
    linkscope::event_fields(
        "engine.worker.run_background_agent_worker",
        [linkscope::TraceField::text(
            "launch_path",
            launch_path.display().to_string(),
        )],
    );
    let paths = DaemonPaths::default_user();
    paths.ensure_dirs()?;
    let launch = jfc_daemon::worker::load_background_agent_launch(&paths, &launch_path)?;
    linkscope::event_fields(
        "engine.worker.launch_loaded",
        [
            linkscope::TraceField::text("task_id", launch.task_id.clone()),
            linkscope::TraceField::text("cwd", launch.cwd.display().to_string()),
            linkscope::TraceField::text("model", launch.model.as_str().to_owned()),
            linkscope::TraceField::count("worker_epoch", launch.worker_epoch),
        ],
    );

    if let Err(e) = std::env::set_current_dir(&launch.cwd) {
        linkscope::event_fields(
            "engine.worker.cwd.result",
            [
                linkscope::TraceField::text("status", "failed"),
                linkscope::TraceField::text("error", e.to_string()),
            ],
        );
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
    linkscope::event_fields(
        "engine.worker.provider_selected",
        [
            linkscope::TraceField::text("provider", provider.name().to_owned()),
            linkscope::TraceField::text(
                "requested_provider",
                launch.provider_name.clone().unwrap_or_default(),
            ),
            linkscope::TraceField::text("model", launch.model.as_str().to_owned()),
        ],
    );

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
        let _linkscope_heartbeat = linkscope::phase("engine.worker.heartbeat_task");
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
    let (configured_mcp_servers, active_mcp_servers) =
        register_background_worker_mcp_registry().await;
    linkscope::event_fields(
        "engine.worker.mcp_registered",
        [
            linkscope::TraceField::count(
                "configured",
                u64::try_from(configured_mcp_servers).unwrap_or(u64::MAX),
            ),
            linkscope::TraceField::count(
                "active",
                u64::try_from(active_mcp_servers).unwrap_or(u64::MAX),
            ),
        ],
    );
    if configured_mcp_servers > 0 {
        append_log_line(
            &background_agent_log_path(&paths, &launch.task_id),
            &format!(
                "[worker-mcp] configured={configured_mcp_servers} active={active_mcp_servers}"
            ),
        );
    }

    let (worktree_info, cwd_override) = match prepare_background_worktree(&launch).await {
        BackgroundIsolation::Proceed(wt, cwd) => (wt, cwd),
        BackgroundIsolation::FailClosed(msg) => {
            linkscope::event_fields(
                "engine.worker.result",
                [linkscope::TraceField::text(
                    "status",
                    "worktree_fail_closed",
                )],
            );
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
        let _linkscope_events = linkscope::phase("engine.worker.event_collector");
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
                    last_tool_info,
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
                        last_tool_info.as_deref(),
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
    let project_root = cwd_override.as_deref().unwrap_or(&launch.cwd);
    let started = Instant::now();
    let result = {
        let _linkscope_execute = linkscope::phase("engine.worker.execute_task");
        match crate::agents::select_background_task_agent_launch_plan(
            &launch.task_input,
            project_root,
        ) {
            Ok(plan) => {
                let worker_task_input = crate::agents::background_worker_execution_task_input(
                    &launch.task_input,
                    &plan,
                );
                crate::tools::execute_task(
                    &worker_task_input,
                    provider.as_ref(),
                    launch.model.clone(),
                    Some(&tx),
                    Some(&launch.task_id),
                    launch.agent_def.as_ref(),
                    cwd_override.clone(),
                    task_store,
                    launch.active_team_name.as_deref(),
                )
                .await
            }
            Err(error) => crate::runtime::ExecutionResult::failure(format!(
                "background agent launch descriptor unavailable: {error}"
            )),
        }
    };
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
        linkscope::event_fields(
            "engine.worker.result",
            [
                linkscope::TraceField::text(
                    "status",
                    if was_cancelled { "cancelled" } else { "failed" },
                ),
                linkscope::TraceField::count("recorded", u64::from(recorded)),
                linkscope::TraceField::count("elapsed_ms", elapsed_ms),
                linkscope::TraceField::bytes(
                    "output_bytes",
                    u64::try_from(result.output.len()).unwrap_or(u64::MAX),
                ),
            ],
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
        linkscope::event_fields(
            "engine.worker.result",
            [
                linkscope::TraceField::text("status", "completed"),
                linkscope::TraceField::count("recorded", u64::from(recorded)),
                linkscope::TraceField::count("elapsed_ms", elapsed_ms),
                linkscope::TraceField::bytes(
                    "output_bytes",
                    u64::try_from(result.output.len()).unwrap_or(u64::MAX),
                ),
            ],
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
