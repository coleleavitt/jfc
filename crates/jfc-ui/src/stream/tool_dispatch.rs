use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::context::ReadDedupCache;
use crate::provider::{ModelId, Provider};
use crate::runtime::{AppEvent, TaskEvent, TeamEvent, ToolEvent};
use crate::scheduler;
use crate::types::{ToolCall, ToolInput};

#[tracing::instrument(target = "jfc::stream", skip(tx, dedup, task_store, provider, model, teammate_event_tx), fields(n = tool_calls.len()))]
pub(crate) fn dispatch_tools_batched(
    tool_calls: Vec<ToolCall>,
    tx: &mpsc::Sender<AppEvent>,
    dedup: Arc<Mutex<ReadDedupCache>>,
    task_store: Option<Arc<crate::tasks::TaskStore>>,
    active_team_name: Option<String>,
    current_session_id: Option<String>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    teammate_event_tx: mpsc::UnboundedSender<crate::swarm::runner::TeammateEvent>,
    // wg-async: tool batches can run for minutes (Bash, subagents). Hand
    // the spawned scheduler a cancel handle so ESC×2 races the batch
    // against `.cancelled()` rather than orphaning the work.
    cancel: CancellationToken,
) {
    let cwd = std::env::current_dir().unwrap_or_default();

    let mut regular_calls: Vec<ToolCall> = Vec::new();
    let mut task_calls: Vec<ToolCall> = Vec::new();
    for tc in tool_calls {
        match &tc.input {
            ToolInput::Task(_) => task_calls.push(tc),
            _ => regular_calls.push(tc),
        }
    }

    let task_count = task_calls.len();
    let regular_count = regular_calls.len();
    tracing::info!(
        target: "jfc::stream",
        task_count, regular_count,
        "dispatch_tools_batched: splitting tool calls"
    );
    let pending = Arc::new(AtomicUsize::new(
        task_count + usize::from(!regular_calls.is_empty()),
    ));
    let tx_done = tx.clone();
    let send_all_complete = move || {
        if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            let _ = tx_done.try_send(AppEvent::Tool(ToolEvent::AllComplete));
        }
    };

    // Pre-load agent defs once per dispatch so each spawned task can
    // resolve its `subagent_type` without redoing the directory walk.
    let agents = crate::agents::load_agents(&cwd);

    for tc in task_calls {
        let task_input = match tc.input.clone() {
            ToolInput::Task(ti) => ti,
            _ => unreachable!(),
        };

        // ─── Teammate spawn path ─────────────────────────────────────────
        // When `name` + `team_name` are provided, spawn a persistent
        // teammate instead of a one-shot subagent. The teammate runs
        // in-process and communicates via the mailbox system.
        if task_input.is_teammate_spawn() {
            let tx_task = tx.clone();
            let task_id = tc.id.as_str().to_owned();
            let done = send_all_complete.clone();

            let name = task_input.name.clone().unwrap_or_default();
            let team_name = task_input.team_name.clone().unwrap_or_default();
            let agent_id = crate::swarm::types::make_agent_id(&name, &team_name);
            let color = crate::swarm::runner::assign_teammate_color();
            let agent_def = task_input
                .subagent_type
                .as_deref()
                .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)));
            let teammate_model = match crate::tools::selected_subagent_model(
                &task_input,
                agent_def,
                model.clone(),
                provider.name(),
            ) {
                Ok(model) => model,
                Err(error) => {
                    let _ = tx_task.try_send(AppEvent::Tool(ToolEvent::Result {
                        tool_id: crate::ids::ToolId::from(task_id),
                        result: crate::runtime::ExecutionResult::failure(error),
                    }));
                    done();
                    continue;
                }
            };
            let teammate_model_name = teammate_model.as_str().to_string();

            let config = crate::swarm::runner::TeammateRunnerConfig {
                identity: crate::swarm::TeammateIdentity {
                    agent_id: agent_id.clone(),
                    agent_name: name.clone(),
                    team_name: team_name.clone(),
                    color: Some(color.clone()),
                    plan_mode_required: task_input.mode.as_deref() == Some("plan"),
                    parent_session_id: current_session_id.clone().unwrap_or_default(),
                },
                prompt: task_input.prompt.clone(),
                description: task_input.description.clone(),
                model: Some(teammate_model_name.clone()),
                agent_type: task_input.subagent_type.clone(),
                provider: provider.clone(),
                model_id: teammate_model,
                system_prompt: None,
                task_store: Some(crate::tasks::TaskStore::open_team(&team_name)),
            };

            let teammate_event_tx = teammate_event_tx.clone();
            let (runner_task_id, abort_tx) =
                crate::swarm::runner::start_teammate(config, teammate_event_tx);
            let _ = runner_task_id;

            // Persist the new member into the team file so the team
            // roster on disk matches the runtime spawn list. Without
            // this, `team_helpers::set_member_active` /
            // `set_member_mode` (which look up by name) silently no-op
            // because members are never actually added.
            let member = crate::swarm::types::TeamMember {
                agent_id: agent_id.clone(),
                name: name.clone(),
                agent_type: task_input.subagent_type.clone(),
                model: Some(teammate_model_name.clone()),
                color: Some(color.clone()),
                plan_mode_required: Some(task_input.mode.as_deref() == Some("plan")),
                joined_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                cwd: None,
                worktree_path: None,
                backend_type: Some(crate::swarm::types::BackendType::InProcess),
                is_active: Some(true),
                mode: task_input.mode.clone(),
            };
            {
                let team_name = team_name.clone();
                tokio::spawn(async move {
                    if let Err(e) = crate::swarm::team_helpers::add_member(&team_name, member).await
                    {
                        tracing::warn!(
                            target: "jfc::swarm",
                            error = %e,
                            "failed to register spawned teammate in team file"
                        );
                    }
                });
            }

            // Report spawn as a successful tool result
            let result_json = serde_json::json!({
                "status": "teammate_spawned",
                "teammate_id": agent_id,
                "name": name,
                "team_name": team_name,
                "color": color,
                "message": format!("Spawned successfully.\nagent_id: {agent_id}\nname: {name}\nteam_name: {team_name}\nThe agent is now running and will receive instructions via mailbox.")
            });

            // Two task IDs in play here:
            //   - `task_id` (= `tc.id`, e.g. "tooluse_xOqQ…") is the
            //     wire id the API uses to match the tool_use request
            //     with our tool_result reply. It MUST be on the
            //     ToolResult.
            //   - `runner_task_id` (= "teammate-name@team") is the id
            //     the runner stamps onto every Progress / TextDelta /
            //     Completed / Failed event.
            // Register the BackgroundTask under the *runner* id so
            // when those events arrive their lookups hit. Otherwise
            // the task panel reads "No messages yet" forever even
            // though the runner is streaming.
            let runner_task_id = crate::swarm::runner::teammate_task_id(&agent_id);
            // Notify the leader's main loop that a teammate exists so
            // `app.team_context.team_name` and `app.team_context.teammates`
            // get populated. Previously these stayed empty for the
            // entire session, so the team-mode tree (`team-lead` leader,
            // teammate rows) never activated and we fell through to
            // the generic subagent tree even though we were in a team.
            let _ = tx_task.try_send(AppEvent::Team(TeamEvent::Spawned {
                name: name.clone(),
                team_name: team_name.clone(),
                agent_id: agent_id.clone(),
                color: Some(color.clone()),
                agent_type: task_input.subagent_type.clone(),
                cwd: std::env::current_dir()
                    .ok()
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or_default(),
                // Hand the abort handle to the main loop. It moves into
                // app.team_context.teammates[agent_id].abort_tx where it
                // stays alive for the teammate's lifetime. Previously the
                // sender was named `_abort_tx` and dropped on the next
                // line, immediately closing the channel and forcing the
                // runner into an Aborted exit on its first stream poll.
                abort_tx: Some(abort_tx),
            }));
            let _ = tx_task.try_send(AppEvent::Task(TaskEvent::Started {
                task_id: crate::ids::TaskId::from(runner_task_id.clone()),
                description: format!("spawn teammate: {name}"),
                model_used: Some(teammate_model_name),
                max_input_tokens: agent_def.and_then(|a| a.max_input_tokens),
                // Teammates are in-process (the runner runs inside this
                // event loop) — DON'T let the UI's TaskStarted handler
                // register them as detached daemon workers. The daemon
                // reconciler would later mark them stale when the UI
                // exits, mis-labeling foreground teammates as Failed.
                is_detached: false,
                parent_task_id: task_input.parent_task_id.clone(),
            }));

            let _ = tx_task.try_send(AppEvent::Tool(ToolEvent::Result {
                tool_id: crate::ids::ToolId::from(task_id),
                result: crate::runtime::ExecutionResult::success(
                    serde_json::to_string_pretty(&result_json).unwrap_or_default(),
                ),
            }));
            done();
            continue;
        }

        // ─── Normal subagent path ────────────────────────────────────────
        let tx_task = tx.clone();
        let provider_task = provider.clone();
        let model_task = model.clone();
        let task_id = tc.id.as_str().to_owned();
        let description = task_input.description.clone();
        let done = send_all_complete.clone();
        let task_store_task = task_store.clone();
        let active_team_name_task = active_team_name.clone();

        // Resolve `subagent_type` to a concrete `AgentDef`. When unset
        // or unknown, falls back to `None` and `execute_task` runs with
        // no system prompt (mirrors the prior, agent-less behavior).
        // Case-insensitive lookup: the model has historically called
        // Task with `subagent_type: "explore"` while we ship agents
        // named "Explore" (markdown-friendly title-case) and v126 also
        // mixes the two. An exact-match miss silently drops the
        // definition — the subagent then runs without its system
        // prompt or tool restrictions and usually exits in <5s with
        // empty output. Fall through with `eq_ignore_ascii_case` so
        // any reasonable casing routes correctly.
        let agent_def = task_input
            .subagent_type
            .as_deref()
            .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)))
            .cloned();
        let model_used = crate::tools::selected_subagent_model(
            &task_input,
            agent_def.as_ref(),
            model.clone(),
            provider.name(),
        )
        .ok()
        .map(|model| model.as_str().to_string());
        let max_input_tokens = agent_def.as_ref().and_then(|a| a.max_input_tokens);
        if agent_def.is_none() {
            if let Some(t) = task_input.subagent_type.as_deref() {
                tracing::warn!(
                    target: "jfc::stream",
                    requested = %t,
                    available = ?agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
                    "subagent_type did not match any loaded agent — running without definition"
                );
            }
        }

        if task_input.run_in_background {
            let launch = crate::daemon::BackgroundAgentLaunch {
                task_id: task_id.clone(),
                task_input: task_input.clone(),
                parent_session_id: current_session_id.clone(),
                model: model.clone(),
                provider_name: Some(provider.name().to_owned()),
                agent_def: agent_def.clone(),
                cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
                worker_exe: None,
                active_team_name: active_team_name_task.clone(),
                created_at: std::time::SystemTime::now(),
            };
            let spawn_result = crate::daemon::spawn_background_agent_worker(launch);
            match spawn_result {
                Ok(pid) => {
                    let _ = tx_task.try_send(AppEvent::Task(TaskEvent::Started {
                        task_id: crate::ids::TaskId::from(task_id.clone()),
                        description: description.clone(),
                        model_used: model_used.clone(),
                        max_input_tokens,
                        // True detached background worker: the worker
                        // process already called
                        // `record_background_agent_started_at` with its
                        // own PID + launch_path. The UI's TaskStarted
                        // handler must skip the registry write so it
                        // doesn't clobber that record.
                        is_detached: true,
                        parent_task_id: task_input.parent_task_id.clone(),
                    }));
                    let result_json = serde_json::json!({
                        "status": "background_task_started",
                        "task_id": task_id.clone(),
                        "worker_pid": pid,
                        "description": description.clone(),
                        "message": "Task is running in a detached worker. Use `jfc daemon agents`, `jfc daemon attach <task_id>`, `jfc daemon wait <task_id>`, or `jfc daemon kill <task_id>`."
                    });
                    let _ = tx_task.try_send(AppEvent::Tool(ToolEvent::Result {
                        tool_id: crate::ids::ToolId::from(task_id.clone()),
                        result: crate::runtime::ExecutionResult::success(
                            serde_json::to_string_pretty(&result_json).unwrap_or_default(),
                        ),
                    }));
                }
                Err(e) => {
                    let error = format!("failed to spawn background worker: {e}");
                    let _ = tx_task.try_send(AppEvent::Task(TaskEvent::Failed {
                        task_id: crate::ids::TaskId::from(task_id.clone()),
                        error: error.clone(),
                    }));
                    let _ = tx_task.try_send(AppEvent::Tool(ToolEvent::Result {
                        tool_id: crate::ids::ToolId::from(task_id.clone()),
                        result: crate::runtime::ExecutionResult::failure(error),
                    }));
                }
            }
            done();
            continue;
        }

        tokio::spawn(async move {
            // If isolation: "worktree", create a git worktree for this agent
            let worktree_info = if task_input.isolation.as_deref() == Some("worktree") {
                let name = format!(
                    "agent-{}",
                    task_id
                        .replace("toolu_", "")
                        .chars()
                        .take(8)
                        .collect::<String>()
                );
                let cwd = std::env::current_dir().unwrap_or_default();
                let repo_root = match crate::worktrees::find_repo_root_async(&cwd).await {
                    Ok(root) => root,
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::stream",
                            cwd = %cwd.display(),
                            error = %e,
                            "task tool: failed to resolve git root, falling back to cwd for worktree"
                        );
                        cwd
                    }
                };
                match crate::worktrees::create_worktree_async(&repo_root, &name).await {
                    Ok(info) => {
                        tracing::info!(
                            target: "jfc::stream",
                            repo_root = %repo_root.display(),
                            worktree = %info.path,
                            "task tool: created worktree for isolated agent"
                        );
                        Some((info, repo_root))
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::stream",
                            repo_root = %repo_root.display(),
                            error = %e,
                            "task tool: failed to create worktree, running in cwd"
                        );
                        None
                    }
                }
            } else {
                None
            };

            tracing::info!(
                target: "jfc::stream",
                task_id = %task_id,
                subagent_type = ?task_input.subagent_type,
                description = %description,
                has_agent_def = agent_def.is_some(),
                "task tool: spawning execute_task"
            );
            let _ = tx_task
                .send(AppEvent::Task(TaskEvent::Started {
                    task_id: crate::ids::TaskId::from(task_id.clone()),
                    description: description.clone(),
                    model_used: model_used.clone(),
                    max_input_tokens,
                    // In-process subagent (foreground Task tool, no
                    // `run_in_background`). Skip daemon registration; the
                    // BackgroundTask row in `app.background_tasks` is the
                    // authoritative UI state.
                    is_detached: false,
                    parent_task_id: task_input.parent_task_id.clone(),
                }))
                .await;
            let started = std::time::Instant::now();
            // Forward the subagent's streaming text into the main event
            // loop (`AppEvent::Task(TaskEvent::AgentChunk)`) so the task view fills live
            // rather than showing "No messages yet" until the agent
            // finishes. tx + task_id are passed through; the producer
            // (`execute_task`) emits one event per `TextDelta`.
            //
            // When isolation requested a worktree, hand its path to the
            // subagent as `cwd_override` so any tools it calls (Read,
            // Bash, Edit, etc.) operate inside the isolated checkout.
            // Without this, "isolation" was a name only — the worktree
            // existed on disk but the agent ran against the parent cwd.
            let cwd_override = worktree_info
                .as_ref()
                .map(|(info, _)| std::path::PathBuf::from(&info.path));
            // No daemon registration for in-process subagents — they're
            // tracked via `app.background_tasks` and the assistant
            // message's TaskStatus parts. Previously this call planted
            // them in the daemon roster too, where the reconciler would
            // later mark them stale at UI exit, confusing the next
            // session's restored "background agents" list.
            let result = crate::tools::execute_task(
                &task_input,
                provider_task.as_ref(),
                model_task,
                Some(&tx_task),
                Some(&task_id),
                agent_def.as_ref(),
                cwd_override,
                task_store_task,
                active_team_name_task.as_deref(),
            )
            .await;
            let elapsed_ms = started.elapsed().as_millis() as u64;

            if result.is_error() {
                tracing::warn!(
                    target: "jfc::stream",
                    task_id = %task_id,
                    elapsed_ms,
                    output_preview = %&result.output[..result.output.len().min(200)],
                    "task tool: execute_task failed"
                );
                let _ = tx_task
                    .send(AppEvent::Task(TaskEvent::Failed {
                        task_id: crate::ids::TaskId::from(task_id.clone()),
                        error: result.output.clone(),
                    }))
                    .await;
            } else {
                tracing::info!(
                    target: "jfc::stream",
                    task_id = %task_id,
                    elapsed_ms,
                    output_len = result.output.len(),
                    "task tool: execute_task succeeded"
                );
                let _ = tx_task
                    .send(AppEvent::Task(TaskEvent::Completed {
                        task_id: crate::ids::TaskId::from(task_id.clone()),
                        summary: result.output.clone(),
                        elapsed_ms,
                    }))
                    .await;
            }

            // Decide the worktree's fate BEFORE sending the ToolResult so the
            // user-visible message can mention the preserved branch
            // when there are uncommitted changes. Mirrors the Claude
            // Code Agent docs: "the worktree is automatically cleaned
            // up if the agent makes no changes; otherwise the path and
            // branch are returned in the result." `git status
            // --porcelain` is the standard "is the working tree clean"
            // signal — quiet, scriptable, exit-code aware.
            let worktree_outcome: Option<(crate::worktrees::WorktreeInfo, bool)> = if let Some((
                wt,
                repo_root,
            )) =
                worktree_info
            {
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
                        tracing::warn!(
                            target: "jfc::stream",
                            worktree = %wt.path,
                            stderr = %String::from_utf8_lossy(&out.stderr),
                            "git status in worktree returned non-zero — keeping worktree to be safe"
                        );
                        true
                    }
                    Err(e) => {
                        tracing::warn!(
                            target: "jfc::stream",
                            worktree = %wt.path,
                            error = %e,
                            "git status spawn failed — keeping worktree"
                        );
                        true
                    }
                };
                if !dirty {
                    let wt_name = std::path::Path::new(&wt.path)
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("");
                    match crate::worktrees::remove_worktree_async(&repo_root, wt_name).await {
                        Ok(_) => tracing::info!(
                            target: "jfc::stream",
                            repo_root = %repo_root.display(),
                            worktree = %wt.path,
                            "worktree had no changes — removed"
                        ),
                        Err(e) => tracing::warn!(
                            target: "jfc::stream",
                            worktree = %wt.path,
                            error = %e,
                            "worktree cleanup failed"
                        ),
                    }
                    Some((wt, false))
                } else {
                    tracing::info!(
                        target: "jfc::stream",
                        worktree = %wt.path,
                        "worktree has uncommitted changes — preserving"
                    );
                    Some((wt, true))
                }
            } else {
                None
            };

            if !task_input.run_in_background {
                let _ = tx_task
                    .send(AppEvent::Tool(ToolEvent::Result {
                        tool_id: crate::ids::ToolId::from(task_id),
                        result: match &worktree_outcome {
                            Some((wt, true)) => crate::runtime::ExecutionResult::success(format!(
                                "{}\n\n[worktree preserved with uncommitted changes]\n\
                             path: {}\nbranch: {}\n\
                             To inspect: cd {} && git diff\n\
                             To merge:   git merge {}\n\
                             To discard: git worktree remove {} && git branch -D {}",
                                result.output,
                                wt.path,
                                wt.branch,
                                wt.path,
                                wt.branch,
                                wt.path,
                                wt.branch,
                            )),
                            Some((_, false)) | None => result,
                        },
                    }))
                    .await;

                done();
            }
        });
    }

    if !regular_calls.is_empty() {
        let batches = scheduler::schedule_tools(regular_calls);
        tracing::debug!(
            target: "jfc::stream",
            batch_count = batches.len(),
            "dispatch_tools_batched: scheduled regular tool batches"
        );
        let tx_clone = tx.clone();
        let done = send_all_complete.clone();
        // wg-async cancellation: race the batch executor against the
        // turn's cancel token. The scheduler itself runs synchronous
        // tool work; a token-cancel cuts off the *await* between tools
        // and lets the spawn return early so its capture set drops.
        let cancel_batch = cancel.clone();
        tokio::spawn(async move {
            tokio::select! {
                biased;
                _ = cancel_batch.cancelled() => {
                    tracing::info!(target: "jfc::stream", "tool batch cancelled via token");
                }
                _ = scheduler::execute_batches(batches, &tx_clone, cwd, dedup, task_store, active_team_name) => {}
            }
            done();
        });
    }
}
