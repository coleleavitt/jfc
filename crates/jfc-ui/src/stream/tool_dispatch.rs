use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::context::ReadDedupCache;
use crate::runtime::{AppEvent, TaskEvent, ToolEvent, send_critical};
use crate::scheduler;
use crate::types::{ChatMessage, ToolCall, ToolInput, ToolKind};
use jfc_provider::{ModelId, Provider};

#[derive(Clone)]
pub(crate) struct LocalAdvisorDispatchContext {
    pub advisor_model: ModelId,
    pub transcript: Vec<ChatMessage>,
}

impl LocalAdvisorDispatchContext {
    pub(crate) fn from_app(app: &crate::app::App) -> Option<Self> {
        if !app.advisor_enabled {
            return None;
        }
        let advisor_model = app.local_advisor_model.clone()?;
        Some(Self {
            advisor_model,
            transcript: app.messages.clone(),
        })
    }
}

#[tracing::instrument(target = "jfc::stream", skip(tool_calls, tx, dedup, task_store, provider, model, teammate_event_tx, local_advisor, cancel), fields(n = tool_calls.len()))]
#[allow(clippy::too_many_arguments)]
pub(crate) fn dispatch_tools_batched(
    tool_calls: Vec<ToolCall>,
    tx: &mpsc::Sender<AppEvent>,
    dedup: Arc<Mutex<ReadDedupCache>>,
    task_store: Option<Arc<jfc_session::TaskStore>>,
    active_team_name: Option<String>,
    current_session_id: Option<String>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    teammate_event_tx: mpsc::UnboundedSender<crate::swarm::runner::TeammateEvent>,
    local_advisor: Option<LocalAdvisorDispatchContext>,
    // wg-async: tool batches can run for minutes (Bash, subagents). Hand
    // the spawned scheduler a cancel handle so ESC×2 races the batch
    // against `.cancelled()` rather than orphaning the work.
    cancel: CancellationToken,
) {
    let cwd = std::env::current_dir().unwrap_or_default();

    let mut regular_calls: Vec<ToolCall> = Vec::new();
    let mut task_calls: Vec<ToolCall> = Vec::new();
    let mut workflow_calls: Vec<ToolCall> = Vec::new();
    let mut advisor_calls: Vec<ToolCall> = Vec::new();
    for tc in tool_calls {
        match (&tc.kind, &tc.input) {
            (ToolKind::Advisor, ToolInput::Advisor {}) => advisor_calls.push(tc),
            (_, ToolInput::Task(_)) => task_calls.push(tc),
            (_, ToolInput::Workflow { .. }) => workflow_calls.push(tc),
            _ => regular_calls.push(tc),
        }
    }

    let task_count = task_calls.len();
    let workflow_count = workflow_calls.len();
    let advisor_count = advisor_calls.len();
    let regular_count = regular_calls.len();
    tracing::info!(
        target: "jfc::stream",
        task_count, workflow_count, advisor_count, regular_count,
        "dispatch_tools_batched: splitting tool calls"
    );
    let pending = Arc::new(AtomicUsize::new(
        task_count + workflow_count + advisor_count + usize::from(!regular_calls.is_empty()),
    ));
    let tx_done = tx.clone();
    let send_all_complete = move || {
        if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            // Critical continuation signal: a dropped AllComplete permanently
            // wedges the agentic loop, so never discard it on a full channel.
            crate::runtime::send_critical(&tx_done, AppEvent::Tool(ToolEvent::AllComplete));
        }
    };

    for tc in advisor_calls {
        let tx_advisor = tx.clone();
        let done = send_all_complete.clone();
        let provider_advisor = provider.clone();
        let tool_id = tc.id.clone();
        let context = local_advisor.clone();
        let cancel_advisor = cancel.clone();
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel_advisor.cancelled() => {
                    crate::runtime::ExecutionResult::failure("Local advisor cancelled by user")
                }
                result = async {
                    match context {
                        Some(context) => match crate::advisor::ask_local_advisor_tool(
                            provider_advisor.as_ref(),
                            context.advisor_model,
                            &context.transcript,
                        )
                        .await
                        {
                            Ok(reply) => crate::runtime::ExecutionResult::success(reply),
                            Err(e) => crate::runtime::ExecutionResult::failure(format!(
                                "Local advisor error: {e}"
                            )),
                        },
                        None => crate::runtime::ExecutionResult::failure(
                            "Local advisor is not configured. Use `/advisor config <model>` or start with `--advisor [MODEL]`."
                                .to_owned(),
                        ),
                    }
                } => result,
            };
            send_critical(
                &tx_advisor,
                AppEvent::Tool(ToolEvent::Result { tool_id, result }),
            );
            done();
        });
    }

    // Pre-load agent defs once per dispatch so each spawned task can
    // resolve its `subagent_type` without redoing the directory walk.
    let agents = crate::agents::load_agents(&cwd);

    for tc in task_calls {
        let mut task_input = match tc.input.clone() {
            ToolInput::Task(ti) => ti,
            _ => unreachable!(),
        };

        // Bug fix: if the model omitted parent_task_id but there's exactly
        // one in-progress task in the store (the one the factory claimed),
        // auto-link this delegation to it. This makes task auto-completion
        // deterministic even when the model forgets to pass the field.
        if task_input.parent_task_id.is_none()
            && let Some(ref store) = task_store
        {
            let in_progress: Vec<_> = store
                .list_all()
                .into_iter()
                .filter(|t| t.status == jfc_session::TaskStatus::InProgress)
                .collect();
            if in_progress.len() == 1 {
                let inferred_id = in_progress[0].id.clone();
                tracing::info!(
                    target: "jfc::stream",
                    inferred_parent = %inferred_id,
                    "auto-linking Task delegation to sole in-progress task"
                );
                task_input.parent_task_id = Some(inferred_id.to_string());
            }
        }

        // ─── Teammate spawn path ─────────────────────────────────────────
        // When `name` + `team_name` are provided, spawn a persistent
        // teammate instead of a one-shot subagent. The teammate runs
        // in-process and communicates via the mailbox system.
        if crate::swarm::dispatch::try_spawn_teammate(
            &task_input,
            tc.id.as_str(),
            tx,
            provider.clone(),
            model.clone(),
            &agents,
            current_session_id.as_deref(),
            teammate_event_tx.clone(),
            send_all_complete.clone(),
        ) {
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
        if agent_def.is_none()
            && let Some(t) = task_input.subagent_type.as_deref()
        {
            tracing::warn!(
                target: "jfc::stream",
                requested = %t,
                available = ?agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
                "subagent_type did not match any loaded agent — running without definition"
            );
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
                worker_epoch: 0,
                active_team_name: active_team_name_task.clone(),
                created_at: std::time::SystemTime::now(),
            };
            let spawn_result = crate::daemon::spawn_background_agent_worker(launch);
            match spawn_result {
                Ok(pid) => {
                    send_critical(
                        &tx_task,
                        AppEvent::Task(TaskEvent::Started {
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
                        }),
                    );
                    let result_json = serde_json::json!({
                        "status": "background_task_started",
                        "task_id": task_id.clone(),
                        "worker_pid": pid,
                        "description": description.clone(),
                        "message": "Task is running in a detached worker. Use `jfc daemon agents`, `jfc daemon attach <task_id>`, `jfc daemon wait <task_id>`, or `jfc daemon kill <task_id>`."
                    });
                    send_critical(
                        &tx_task,
                        AppEvent::Tool(ToolEvent::Result {
                            tool_id: crate::ids::ToolId::from(task_id.clone()),
                            result: crate::runtime::ExecutionResult::success(
                                serde_json::to_string_pretty(&result_json).unwrap_or_default(),
                            ),
                        }),
                    );
                }
                Err(e) => {
                    let error = format!("failed to spawn background worker: {e}");
                    send_critical(
                        &tx_task,
                        AppEvent::Task(TaskEvent::Failed {
                            task_id: crate::ids::TaskId::from(task_id.clone()),
                            error: error.clone(),
                        }),
                    );
                    send_critical(
                        &tx_task,
                        AppEvent::Tool(ToolEvent::Result {
                            tool_id: crate::ids::ToolId::from(task_id.clone()),
                            result: crate::runtime::ExecutionResult::failure(error),
                        }),
                    );
                }
            }
            done();
            continue;
        }

        let cancel_task = cancel.clone();
        // Clone the session id for the change-set origin so the closure's use
        // doesn't move the loop-shared `current_session_id` (still needed by
        // later iterations + the daemon-registration call below).
        let changeset_session_id = current_session_id.as_ref().map(|s| s.as_str().to_string());
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
                        // Open a change-set so this isolated run becomes a
                        // durable, reviewable proposal (Dolt-style branch).
                        let origin = crate::changeset::ChangeOrigin {
                            task_id: Some(task_id.clone()),
                            agent_id: task_input
                                .subagent_type
                                .clone()
                                .or_else(|| Some("task".to_string())),
                            session_id: changeset_session_id.clone(),
                        };
                        let change_id = crate::changeset::open_for_worktree(
                            &repo_root,
                            &info.path,
                            &info.branch,
                            &origin,
                        )
                        .await;
                        Some((info, repo_root, change_id))
                    }
                    Err(e) => {
                        // Isolation was requested but couldn't be created.
                        // Default fail-closed: do NOT silently run a
                        // (potentially mutating) agent against the main
                        // checkout — that breaks the "production stays
                        // untouched" guarantee. Abort the dispatch instead.
                        match crate::changeset::isolation_fallback() {
                            crate::changeset::IsolationFallback::FailClosed => {
                                let msg = format!(
                                    "Refusing to run isolated agent in the main checkout: \
                                     worktree creation failed ({e}). Isolation is fail-closed \
                                     (set [isolation] fail_closed = false or \
                                     JFC_ISOLATION_FAIL_CLOSED=0 to allow the cwd fallback)."
                                );
                                tracing::warn!(
                                    target: "jfc::stream",
                                    repo_root = %repo_root.display(),
                                    error = %e,
                                    "task tool: worktree creation failed — failing closed"
                                );
                                let _ = tx_task
                                    .send(AppEvent::Task(TaskEvent::Failed {
                                        task_id: crate::ids::TaskId::from(task_id.clone()),
                                        error: msg.clone(),
                                    }))
                                    .await;
                                if !task_input.run_in_background {
                                    let _ = tx_task
                                        .send(AppEvent::Tool(ToolEvent::Result {
                                            tool_id: crate::ids::ToolId::from(task_id.clone()),
                                            result: crate::runtime::ExecutionResult::failure(msg),
                                        }))
                                        .await;
                                }
                                done();
                                return;
                            }
                            crate::changeset::IsolationFallback::AllowCwd => {
                                tracing::warn!(
                                    target: "jfc::stream",
                                    repo_root = %repo_root.display(),
                                    error = %e,
                                    "task tool: failed to create worktree, running in cwd (fail-open)"
                                );
                                None
                            }
                        }
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
                .map(|(info, _, _)| std::path::PathBuf::from(&info.path));
            // No daemon registration for in-process subagents — they're
            // tracked via `app.background_tasks` and the assistant
            // message's TaskStatus parts. Previously this call planted
            // them in the daemon roster too, where the reconciler would
            // later mark them stale at UI exit, confusing the next
            // session's restored "background agents" list.
            let result = tokio::select! {
                biased;
                _ = cancel_task.cancelled() => {
                    let killed = crate::bash_processes::terminate_all();
                    tracing::info!(
                        target: "jfc::stream",
                        task_id = %task_id,
                        killed,
                        "task tool cancelled via turn token"
                    );
                    // Audit: record the cancellation against the task.
                    crate::changeset::record_cancellation("task", Some(task_id.clone()));
                    crate::runtime::ExecutionResult::failure("Task cancelled by user")
                }
                result = crate::tools::execute_task(
                    &task_input,
                    provider_task.as_ref(),
                    model_task,
                    Some(&tx_task),
                    Some(&task_id),
                    agent_def.as_ref(),
                    cwd_override,
                    task_store_task,
                    active_team_name_task.as_deref(),
                ) => result,
            };
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
                change_id,
            )) =
                worktree_info
            {
                // Finalize the change-set (diff vs base → Ready, or Abandoned
                // if clean) BEFORE any cleanup, while the worktree still
                // exists to diff against.
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

    for tc in workflow_calls {
        let done = send_all_complete.clone();
        spawn_workflow(
            tc,
            tx,
            provider.clone(),
            model.clone(),
            current_session_id.clone(),
            cancel.clone(),
            done,
        );
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
        // Let the scheduler settle every started tool before emitting
        // AllComplete. Dropping this future on cancellation drops its
        // JoinHandles, and Tokio treats that as detach rather than abort:
        // stale tool tasks can keep running and report after the turn was
        // announced complete. ESCx2 still SIGTERMs tracked bash subprocesses;
        // this await keeps the transcript/event ordering coherent.
        let cancel_batch = cancel.clone();
        tokio::spawn(async move {
            scheduler::execute_batches(
                batches,
                &tx_clone,
                cwd,
                dedup,
                task_store,
                active_team_name,
                cancel_batch.clone(),
            )
            .await;
            if cancel_batch.is_cancelled() {
                tracing::info!(target: "jfc::stream", "tool batch settled after cancellation");
            }
            done();
        });
    }
}

/// Resolve, register, and spawn a Workflow tool call. Returns immediately
/// after sending the `async_launched` ToolResult; the workflow runs in the
/// background and injects a `<task-notification>` when it completes.
#[allow(clippy::too_many_arguments)]
fn spawn_workflow(
    tc: ToolCall,
    tx: &mpsc::Sender<AppEvent>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    current_session_id: Option<String>,
    cancel: CancellationToken,
    done: impl FnOnce() + Send + 'static,
) {
    let (script, name, script_path, args, resume_from_run_id) = match tc.input.clone() {
        ToolInput::Workflow {
            script,
            name,
            script_path,
            args,
            resume_from_run_id,
        } => (script, name, script_path, args, resume_from_run_id),
        _ => {
            done();
            return;
        }
    };

    let tool_id = tc.id.clone();
    let tx = tx.clone();
    let cwd = std::env::current_dir().unwrap_or_default();

    tokio::spawn(async move {
        // ── resolve script from inline | name | scriptPath ──────────────
        let resolved = resolve_workflow_script(&cwd, script, name.clone(), script_path).await;
        let (script_text, source_path) = match resolved {
            Ok(v) => v,
            Err(e) => {
                send_workflow_result(&tx, &tool_id, crate::runtime::ExecutionResult::failure(e));
                done();
                return;
            }
        };

        // ── parse meta + validate determinism ───────────────────────────
        let meta = match crate::workflows::meta::parse_meta(&script_text) {
            Ok((m, _body)) => m,
            Err(e) => {
                send_workflow_result(
                    &tx,
                    &tool_id,
                    crate::runtime::ExecutionResult::failure(format!("invalid workflow: {e}")),
                );
                done();
                return;
            }
        };
        if let Err(e) = crate::workflows::meta::validate_script(&script_text) {
            send_workflow_result(&tx, &tool_id, crate::runtime::ExecutionResult::failure(e));
            done();
            return;
        }

        // ── named-workflow permission gate ──────────────────────────────
        let config = crate::config::load_arc();
        // Allow and Ask both proceed here — the upstream opt-in
        // (ultrawork / explicit request) is the gate for Ask; a future
        // interactive dialog can refine this.
        if crate::workflows::permissions::decide(&config, name.as_deref())
            == crate::workflows::permissions::WorkflowPermission::Deny
        {
            send_workflow_result(
                &tx,
                &tool_id,
                crate::runtime::ExecutionResult::failure(format!(
                    "Workflow '{}' is denied by permission rules",
                    meta.name
                )),
            );
            done();
            return;
        }

        // ── runId + session dir + persist inline script ─────────────────
        let run_id = crate::workflows::generate_run_id();
        let session_dir = workflow_session_dir(current_session_id.as_deref(), &run_id);
        let _ = tokio::fs::create_dir_all(&session_dir).await;

        let persisted_path = match source_path {
            Some(p) => Some(p),
            None => {
                let p = session_dir.join("script.js");
                match tokio::fs::write(&p, &script_text).await {
                    Ok(_) => Some(p),
                    Err(e) => {
                        tracing::warn!(target: "jfc::workflow", error = %e, "failed to persist inline script");
                        None
                    }
                }
            }
        };

        // ── register the workflow as a background task ──────────────────
        let bg_task_id = format!("bgwf_{run_id}");
        let _ = tx
            .send(AppEvent::Task(crate::runtime::TaskEvent::Started {
                task_id: crate::ids::TaskId::from(bg_task_id.clone()),
                description: format!("workflow: {}", meta.name),
                model_used: Some(model.as_str().to_string()),
                max_input_tokens: None,
                is_detached: false,
                parent_task_id: None,
            }))
            .await;

        // ── return async_launched immediately ───────────────────────────
        let launch = serde_json::json!({
            "status": "async_launched",
            "taskId": bg_task_id,
            "runId": run_id,
            "scriptPath": persisted_path.as_ref().map(|p| p.display().to_string()),
            "summary": meta.description,
        });
        send_workflow_result(
            &tx,
            &tool_id,
            crate::runtime::ExecutionResult::success(
                serde_json::to_string_pretty(&launch).unwrap_or_default(),
            ),
        );
        // The Workflow tool result is delivered; release the AllComplete latch
        // so the agentic loop continues while the workflow runs in background.
        done();

        // ── parse the body (strip the meta block) and run ───────────────
        let body = crate::workflows::meta::parse_meta(&script_text)
            .map(|(_, b)| b)
            .unwrap_or(script_text.clone());

        let started = std::time::Instant::now();
        let outcome = crate::workflows::run_workflow(crate::workflows::WorkflowRunConfig {
            run_id: run_id.clone(),
            script_body: body,
            args: args.unwrap_or(serde_json::Value::Null),
            provider,
            model,
            session_dir: session_dir.clone(),
            resume_from_run_id,
            cancel,
            tx: Some(tx.clone()),
            workflow_task_id: bg_task_id.clone(),
            depth: 0,
            cwd: std::env::current_dir().unwrap_or_default(),
            token_budget: None,
        })
        .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;

        // ── mark the background task terminal ───────────────────────────
        // The notification body becomes the task summary so the standard
        // background-completion path surfaces it to the model and re-engages
        // the agentic loop (`maybe_resume_after_background`).
        let notification = build_task_notification(&bg_task_id, &meta.name, &outcome, elapsed_ms);
        if let Some(err) = &outcome.error {
            let _ = tx
                .send(AppEvent::Task(crate::runtime::TaskEvent::Failed {
                    task_id: crate::ids::TaskId::from(bg_task_id.clone()),
                    error: err.clone(),
                }))
                .await;
        } else {
            let _ = tx
                .send(AppEvent::Task(crate::runtime::TaskEvent::Completed {
                    task_id: crate::ids::TaskId::from(bg_task_id.clone()),
                    summary: notification,
                    elapsed_ms,
                }))
                .await;
        }
    });
}

/// Resolve a workflow's script text + optional on-disk path from the three
/// possible inputs (scriptPath > name > inline script), matching CC 146's
/// precedence.
async fn resolve_workflow_script(
    cwd: &std::path::Path,
    script: Option<String>,
    name: Option<String>,
    script_path: Option<String>,
) -> Result<(String, Option<std::path::PathBuf>), String> {
    if let Some(p) = script_path {
        let path = if std::path::Path::new(&p).is_absolute() {
            std::path::PathBuf::from(&p)
        } else {
            cwd.join(&p)
        };
        let text = tokio::fs::read_to_string(&path)
            .await
            .map_err(|e| format!("read {}: {e}", path.display()))?;
        return Ok((text, Some(path)));
    }
    if let Some(n) = name {
        let wf = crate::workflows::resolve(cwd, &n)
            .ok_or_else(|| format!("workflow '{n}' not found"))?;
        return Ok((wf.script, wf.path));
    }
    if let Some(s) = script {
        return Ok((s, None));
    }
    Err("Workflow requires one of: script, name, or scriptPath".to_owned())
}

/// Compute the session directory that holds a workflow's journal + script.
fn workflow_session_dir(session_id: Option<&str>, run_id: &str) -> std::path::PathBuf {
    let base = jfc_session::sessions_dir();
    match session_id {
        Some(sid) => base.join(sid).join("workflows").join(run_id),
        None => base.join("workflows").join(run_id),
    }
}

/// Send a ToolResult for a workflow call.
fn send_workflow_result(
    tx: &mpsc::Sender<AppEvent>,
    tool_id: &crate::ids::ToolId,
    result: crate::runtime::ExecutionResult,
) {
    let tx = tx.clone();
    let tool_id = tool_id.clone();
    tokio::spawn(async move {
        let _ = tx
            .send(AppEvent::Tool(ToolEvent::Result { tool_id, result }))
            .await;
    });
}

/// Build the `<task-notification>` injected when a workflow completes.
fn build_task_notification(
    task_id: &str,
    name: &str,
    outcome: &crate::workflows::WorkflowOutcome,
    elapsed_ms: u64,
) -> String {
    let status = if outcome.error.is_some() {
        "failed"
    } else {
        "completed"
    };
    let mut body = format!(
        "<task-notification>\n<task-id>{task_id}</task-id>\n<status>{status}</status>\n\
         <summary>Workflow \"{name}\" {status}</summary>"
    );
    if let Some(err) = &outcome.error {
        body.push_str(&format!("\n<error>{err}</error>"));
    } else {
        let result_json = serde_json::to_string(&outcome.result).unwrap_or_default();
        let truncated: String = result_json.chars().take(8000).collect();
        body.push_str(&format!("\n<result>{truncated}</result>"));
    }
    if !outcome.logs.is_empty() {
        let log_text: String = outcome
            .logs
            .iter()
            .map(|l| format!("  {l}"))
            .collect::<Vec<_>>()
            .join("\n");
        body.push_str(&format!("\n<logs>\n{log_text}\n</logs>"));
    }
    body.push_str(&format!(
        "\n<usage><agent_count>{}</agent_count><agents_dispatched>{}</agents_dispatched>\
         <cache_hits>{}</cache_hits><duration_ms>{}</duration_ms></usage>\n</task-notification>",
        outcome.agent_count, outcome.total_agents_dispatched, outcome.cache_hits, elapsed_ms
    ));
    body
}
