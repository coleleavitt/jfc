use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use tokio::sync::{Mutex, mpsc};
use tokio_util::sync::CancellationToken;

use crate::context::ReadDedupCache;
use crate::runtime::{EngineEvent, TaskEvent, ToolEvent, send_critical};
use crate::scheduler;
use crate::types::{ChatMessage, ToolCall, ToolInput, ToolKind};
use jfc_provider::{ModelId, Provider};

/// Max output tokens for a single `AskModel` one-shot completion. Matches the
/// council member default — enough for a substantive prose answer without
/// inviting a runaway generation on every mid-turn cross-model call.
const ASK_MODEL_MAX_TOKENS: u32 = 2048;

fn inherit_agent_isolation_if_omitted(
    task_input: &mut crate::types::TaskInput,
    agent_def: Option<&crate::agents::AgentDef>,
) {
    if task_input.isolation.is_none()
        && let Some(isolation) = agent_def.and_then(|agent| agent.isolation.clone())
    {
        task_input.isolation = Some(isolation);
    }
}

fn apply_default_task_isolation_if_omitted(
    task_input: &mut crate::types::TaskInput,
    default_isolation: Option<&str>,
) {
    if task_input.isolation.is_some() {
        return;
    }
    let Some(default_isolation) = default_isolation.map(str::trim).filter(|s| !s.is_empty()) else {
        return;
    };
    task_input.isolation = Some(default_isolation.to_owned());
}

fn apply_config_default_task_isolation_if_omitted(task_input: &mut crate::types::TaskInput) {
    let config = crate::config::load_arc();
    let default_isolation = config
        .isolation
        .as_ref()
        .and_then(|isolation| isolation.default_task_isolation.as_deref());
    apply_default_task_isolation_if_omitted(task_input, default_isolation);
}

#[derive(Clone)]
pub struct LocalAdvisorDispatchContext {
    pub targets: Vec<crate::advisor::LocalAdvisorProviderTarget>,
    pub transcript: Vec<ChatMessage>,
}

impl LocalAdvisorDispatchContext {
    pub fn from_state(state: &crate::app::EngineState) -> Option<Self> {
        if !state.advisor_enabled {
            return None;
        }
        let advisor_model = state.local_advisor_model.clone()?;
        let targets = match crate::advisor::resolve_local_advisor_provider_targets(
            &state.providers,
            Arc::clone(&state.provider),
            state.local_advisor_provider.as_ref(),
            &advisor_model,
        ) {
            Ok(targets) => targets,
            Err(e) => {
                tracing::warn!(
                    target: "jfc::advisor",
                    error = %e,
                    "local advisor provider unavailable"
                );
                return None;
            }
        };
        Some(Self {
            targets,
            transcript: state.messages.clone(),
        })
    }
}

/// Whether a failed detached-worker spawn should fall back to running the Task
/// in-process instead of surfacing a hard failure. True only for the
/// at-capacity case (`ErrorKind::ResourceBusy`) the daemon returns when the
/// background-agent pool is full; every other spawn error is a real failure.
fn spawn_error_means_run_inproc(spawn_result: &std::io::Result<u32>) -> bool {
    matches!(spawn_result, Err(e) if e.kind() == std::io::ErrorKind::ResourceBusy)
}

pub struct ToolBatchDispatch {
    pub tx: mpsc::Sender<EngineEvent>,
    pub dedup: Arc<Mutex<ReadDedupCache>>,
    pub task_store: Option<Arc<jfc_session::TaskStore>>,
    pub active_team_name: Option<String>,
    pub current_session_id: Option<String>,
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
    /// Full provider registry, so tools that pick arbitrary models (e.g. the
    /// Council with an explicit `models` list) can resolve them via
    /// [`crate::runtime::bootstrap::resolve_provider_model`]. Empty when the
    /// caller has no registry handy (falls back to the active provider).
    pub providers: Vec<Arc<dyn Provider>>,
    pub teammate_event_tx: mpsc::UnboundedSender<crate::swarm::runner::TeammateEvent>,
    pub local_advisor: Option<LocalAdvisorDispatchContext>,
    pub cancel: CancellationToken,
}

#[tracing::instrument(target = "jfc::stream", skip(tool_calls, dispatch), fields(n = tool_calls.len()))]
pub fn dispatch_tools_batched(tool_calls: Vec<ToolCall>, dispatch: ToolBatchDispatch) {
    let ToolBatchDispatch {
        tx,
        dedup,
        task_store,
        active_team_name,
        current_session_id,
        provider,
        model,
        providers,
        teammate_event_tx,
        local_advisor,
        // wg-async: tool batches can run for minutes (Bash, subagents). Hand
        // the spawned scheduler a cancel handle so ESC×2 races the batch
        // against `.cancelled()` rather than orphaning the work.
        cancel,
    } = dispatch;
    let tx = &tx;
    let cwd = std::env::current_dir().unwrap_or_default();

    let mut regular_calls: Vec<ToolCall> = Vec::new();
    let mut task_calls: Vec<ToolCall> = Vec::new();
    let mut workflow_calls: Vec<ToolCall> = Vec::new();
    let mut advisor_calls: Vec<ToolCall> = Vec::new();
    let mut council_calls: Vec<ToolCall> = Vec::new();
    let mut ask_model_calls: Vec<ToolCall> = Vec::new();
    let mut research_calls: Vec<ToolCall> = Vec::new();
    for tc in tool_calls {
        match (&tc.kind, &tc.input) {
            (ToolKind::Advisor, ToolInput::Advisor {}) => advisor_calls.push(tc),
            (ToolKind::Council, ToolInput::Council { .. }) => council_calls.push(tc),
            (ToolKind::AskModel, ToolInput::AskModel { .. }) => ask_model_calls.push(tc),
            (ToolKind::Research, ToolInput::Research { .. }) => research_calls.push(tc),
            (_, ToolInput::Task(_)) => task_calls.push(tc),
            (_, ToolInput::Workflow { .. }) => workflow_calls.push(tc),
            _ => regular_calls.push(tc),
        }
    }

    let task_count = task_calls.len();
    let workflow_count = workflow_calls.len();
    let advisor_count = advisor_calls.len();
    let council_count = council_calls.len();
    let ask_model_count = ask_model_calls.len();
    let research_count = research_calls.len();
    let regular_count = regular_calls.len();
    tracing::info!(
        target: "jfc::stream",
        task_count, workflow_count, advisor_count, council_count, ask_model_count, research_count, regular_count,
        "dispatch_tools_batched: splitting tool calls"
    );
    let pending = Arc::new(AtomicUsize::new(
        task_count
            + workflow_count
            + advisor_count
            + council_count
            + ask_model_count
            + research_count
            + usize::from(!regular_calls.is_empty()),
    ));
    let tx_done = tx.clone();
    let send_all_complete = move || {
        if pending.fetch_sub(1, Ordering::AcqRel) == 1 {
            // Critical continuation signal: a dropped AllComplete permanently
            // wedges the agentic loop, so never discard it on a full channel.
            crate::runtime::send_critical(&tx_done, EngineEvent::Tool(ToolEvent::AllComplete));
        }
    };

    for tc in advisor_calls {
        let tx_advisor = tx.clone();
        let done = send_all_complete.clone();
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
                            &context.targets,
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
                EngineEvent::Tool(ToolEvent::Result { tool_id, result }),
            );
            done();
        });
    }

    // Council: fan the question out to the active model plus the local advisor
    // model (when distinct/available), then synthesise. Runs out-of-band like
    // the advisor — providers come from the dispatch context, so no transcript
    // or full provider registry is needed.
    for tc in council_calls {
        let tx_council = tx.clone();
        let done = send_all_complete.clone();
        let tool_id = tc.id.clone();
        let cancel_council = cancel.clone();
        let active_provider = provider.clone();
        let active_model = model.clone();
        let advisor_ctx = local_advisor.clone();
        let registry = providers.clone();
        let council_task_store = task_store.clone();
        let council_team_name = active_team_name.clone();
        let council_cwd = cwd.clone();
        let (question, requested_models, overrides) = match tc.input.clone() {
            ToolInput::Council {
                question,
                models,
                intent,
                mode,
                archive,
                quorum,
                retry_on_fail,
                member_timeout_ms,
            } => (
                question,
                models,
                CouncilToolOverrides {
                    intent,
                    mode,
                    archive,
                    quorum,
                    retry_on_fail,
                    member_timeout_ms,
                },
            ),
            _ => (String::new(), Vec::new(), CouncilToolOverrides::default()),
        };
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel_council.cancelled() => {
                    crate::runtime::ExecutionResult::failure("Council cancelled by user")
                }
                result = run_council_tool(
                    question,
                    requested_models,
                    overrides,
                    active_provider,
                    active_model,
                    advisor_ctx,
                    registry,
                    council_task_store,
                    council_team_name,
                    council_cwd,
                ) => result,
            };
            send_critical(
                &tx_council,
                EngineEvent::Tool(ToolEvent::Result { tool_id, result }),
            );
            done();
        });
    }

    // AskModel: a single direct, tool-less completion against ONE arbitrary
    // model resolved from the provider registry, threaded back as the tool
    // result. Runs out-of-band like the advisor/council. This is the mid-turn
    // cross-model handoff primitive (e.g. Claude asks gpt-5.5 inline).
    for tc in ask_model_calls {
        let tx_ask = tx.clone();
        let done = send_all_complete.clone();
        let tool_id = tc.id.clone();
        let cancel_ask = cancel.clone();
        let active_provider = provider.clone();
        let active_model = model.clone();
        let registry = providers.clone();
        let (req_model, prompt, system) = match tc.input.clone() {
            ToolInput::AskModel {
                model,
                prompt,
                system,
            } => (model, prompt, system),
            _ => (String::new(), String::new(), None),
        };
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel_ask.cancelled() => {
                    crate::runtime::ExecutionResult::failure("AskModel cancelled by user")
                }
                result = run_ask_model_tool(
                    req_model,
                    prompt,
                    system,
                    active_provider,
                    active_model,
                    registry,
                ) => result,
            };
            send_critical(
                &tx_ask,
                EngineEvent::Tool(ToolEvent::Result { tool_id, result }),
            );
            done();
        });
    }

    // Research: an agentic web+codebase research loop driven by the active
    // model (planner reformulates queries from evidence, synthesizer writes the
    // cited answer). Runs out-of-band like the advisor/council — it gets the
    // active provider + model from the dispatch context, not the main stream.
    for tc in research_calls {
        let tx_research = tx.clone();
        let done = send_all_complete.clone();
        let tool_id = tc.id.clone();
        let cancel_research = cancel.clone();
        let active_provider = provider.clone();
        let active_model = model.clone();
        let (question, export) = match tc.input.clone() {
            ToolInput::Research { question, export } => (question, export),
            _ => (String::new(), false),
        };
        tokio::spawn(async move {
            let result = tokio::select! {
                biased;
                _ = cancel_research.cancelled() => {
                    crate::runtime::ExecutionResult::failure("Research cancelled by user")
                }
                result = crate::tools::research::execute_research_agentic(
                    &question,
                    export,
                    active_provider,
                    active_model,
                ) => result,
            };
            send_critical(
                &tx_research,
                EngineEvent::Tool(ToolEvent::Result { tool_id, result }),
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
            &providers,
            send_all_complete.clone(),
        ) {
            continue;
        }

        // ─── Normal subagent path ────────────────────────────────────────
        let tx_task = tx.clone();
        let task_registry = providers.clone();
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
        let mut agent_def = task_input
            .subagent_type
            .as_deref()
            .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)))
            .cloned();
        inherit_agent_isolation_if_omitted(&mut task_input, agent_def.as_ref());
        apply_config_default_task_isolation_if_omitted(&mut task_input);
        // Subagent context inheritance: when enabled, seed the subagent's
        // `forks_parent_context` with a compact CLAUDE.md summary so it
        // doesn't need to re-scan the codebase. Injected into the system
        // prompt by `inject_parent_context` inside `execute_task_inner`.
        if crate::config::load_arc().subagent_context_inheritance {
            if let Some(ref mut def) = agent_def {
                let context_seed = crate::tools::build_parent_context_seed(&cwd);
                def.forks_parent_context = Some(context_seed);
            }
        }
        // Provider-qualified specs ("openai/gpt-5.2") route through the
        // registry and may switch providers — same addressing the council
        // uses. Resolved here (not inside execute_task) so the background
        // launch record and the Started event both carry the real target.
        let resolved_spawn = crate::tools::selected_subagent_provider_model(
            &task_input,
            agent_def.as_ref(),
            provider.clone(),
            model.clone(),
            &task_registry,
        );
        let (provider_task, model_task) = match &resolved_spawn {
            Ok((p, m)) => (p.clone(), m.clone()),
            Err(_) => (provider_task, model_task),
        };
        let model_used = resolved_spawn
            .as_ref()
            .ok()
            .map(|(_, model)| model.as_str().to_string());
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
            let project_root = task_input
                .cwd
                .as_deref()
                .map(std::path::Path::new)
                .unwrap_or(cwd.as_path());
            let background_launch_plan =
                match crate::agents::select_background_task_agent_launch_plan(
                    &task_input,
                    project_root,
                ) {
                    Ok(plan) => plan,
                    Err(error) => {
                        let error =
                            format!("background agent launch descriptor unavailable: {error}");
                        send_critical(
                            &tx_task,
                            EngineEvent::Task(TaskEvent::Failed {
                                task_id: crate::ids::TaskId::from(task_id.clone()),
                                error: error.clone(),
                            }),
                        );
                        send_critical(
                            &tx_task,
                            EngineEvent::Tool(ToolEvent::Result {
                                tool_id: crate::ids::ToolId::from(task_id.clone()),
                                result: crate::runtime::ExecutionResult::failure(error),
                            }),
                        );
                        done();
                        continue;
                    }
                };
            let launch_backend = match &background_launch_plan.backend {
                crate::agents::AgentLaunchBackend::BackgroundWorker => "background_worker",
                crate::agents::AgentLaunchBackend::InProcess => "in_process",
                crate::agents::AgentLaunchBackend::ProcessBridge { .. } => "process_bridge",
            };
            match background_launch_plan.backend {
                crate::agents::AgentLaunchBackend::BackgroundWorker => {}
                crate::agents::AgentLaunchBackend::InProcess => {
                    let error = format!(
                        "background agent launcher {} resolved to an in-process backend",
                        background_launch_plan.descriptor.name
                    );
                    send_critical(
                        &tx_task,
                        EngineEvent::Task(TaskEvent::Failed {
                            task_id: crate::ids::TaskId::from(task_id.clone()),
                            error: error.clone(),
                        }),
                    );
                    send_critical(
                        &tx_task,
                        EngineEvent::Tool(ToolEvent::Result {
                            tool_id: crate::ids::ToolId::from(task_id.clone()),
                            result: crate::runtime::ExecutionResult::failure(error),
                        }),
                    );
                    done();
                    continue;
                }
                crate::agents::AgentLaunchBackend::ProcessBridge { .. } => {}
            }
            tracing::debug!(
                target: "jfc::stream",
                task_id = %task_id,
                launcher = %background_launch_plan.descriptor.name,
                handler = %background_launch_plan.descriptor.executor.handler,
                "selected descriptor-owned background agent launch backend"
            );
            let launch = crate::daemon::BackgroundAgentLaunch {
                task_id: task_id.clone(),
                task_input: task_input.clone(),
                parent_session_id: current_session_id.clone(),
                model: model_task.clone(),
                provider_name: Some(provider_task.name().to_owned()),
                agent_def: agent_def.clone(),
                cwd: task_input
                    .cwd
                    .as_deref()
                    .map(std::path::PathBuf::from)
                    .unwrap_or_else(|| {
                        std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
                    }),
                worker_exe: None,
                worker_epoch: 0,
                active_team_name: active_team_name_task.clone(),
                created_at: std::time::SystemTime::now(),
            };
            let spawn_result = crate::daemon::spawn_background_agent_worker(launch);
            // When the detached-worker pool is at capacity, don't hard-fail the
            // Task — fall through to run it IN-PROCESS (foreground) instead, so
            // the work still completes. This turns the 8/8 cap from a failure
            // into transparent queuing onto the in-process executor.
            if spawn_error_means_run_inproc(&spawn_result) {
                tracing::info!(
                    target: "jfc::stream",
                    task_id = %task_id,
                    "background agent pool at capacity — running this Task in-process instead"
                );
                // Flip the flag so the in-process path below treats this as a
                // true foreground Task: it gates result emission on
                // `!run_in_background` (e.g. the worktree fail-closed branch),
                // so leaving it `true` would drop the tool_result and hang the
                // model. Control then falls through (no done()/continue here).
                task_input.run_in_background = false;
            } else {
                match spawn_result {
                    Ok(pid) => {
                        send_critical(
                            &tx_task,
                            EngineEvent::Task(TaskEvent::Started {
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
                            "launcher": background_launch_plan.descriptor.name,
                            "launch_backend": launch_backend,
                            "description": description.clone(),
                            "message": "Task is running in a detached worker. Use `jfc daemon agents`, `jfc daemon attach <task_id>`, `jfc daemon wait <task_id>`, or `jfc daemon kill <task_id>`."
                        });
                        send_critical(
                            &tx_task,
                            EngineEvent::Tool(ToolEvent::Result {
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
                            EngineEvent::Task(TaskEvent::Failed {
                                task_id: crate::ids::TaskId::from(task_id.clone()),
                                error: error.clone(),
                            }),
                        );
                        send_critical(
                            &tx_task,
                            EngineEvent::Tool(ToolEvent::Result {
                                tool_id: crate::ids::ToolId::from(task_id.clone()),
                                result: crate::runtime::ExecutionResult::failure(error),
                            }),
                        );
                    }
                }
                done();
                continue;
            } // end else (non-capacity spawn outcome)
            // Reached only on the ResourceBusy capacity fallback: fall through
            // to the in-process subagent path below (no done()/continue above).
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
                                    .send(EngineEvent::Task(TaskEvent::Failed {
                                        task_id: crate::ids::TaskId::from(task_id.clone()),
                                        error: msg.clone(),
                                    }))
                                    .await;
                                if !task_input.run_in_background {
                                    let _ = tx_task
                                        .send(EngineEvent::Tool(ToolEvent::Result {
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
                .send(EngineEvent::Task(TaskEvent::Started {
                    task_id: crate::ids::TaskId::from(task_id.clone()),
                    description: description.clone(),
                    model_used: model_used.clone(),
                    max_input_tokens,
                    // In-process subagent (foreground Task tool, no
                    // `run_in_background`). Skip daemon registration; the
                    // BackgroundTask row in `state.background_tasks` is the
                    // authoritative UI state.
                    is_detached: false,
                    parent_task_id: task_input.parent_task_id.clone(),
                }))
                .await;
            let started = std::time::Instant::now();
            // Forward the subagent's streaming text into the main event
            // loop (`EngineEvent::Task(TaskEvent::AgentChunk)`) so the task view fills live
            // rather than showing "No messages yet" until the agent
            // finishes. tx + task_id are passed through; the producer
            // (`execute_task`) emits one event per `TextDelta`.
            //
            // When isolation requested a worktree, hand its path to the
            // subagent as `cwd_override` so any tools it calls (Read,
            // Bash, Edit, etc.) operate inside the isolated checkout.
            // Without this, "isolation" was a name only — the worktree
            // existed on disk but the agent ran against the parent cwd.
            // cwd_override precedence: worktree isolation path > explicit
            // task_input.cwd > None (execute_task falls back to current_dir).
            let cwd_override = worktree_info
                .as_ref()
                .map(|(info, _, _)| std::path::PathBuf::from(&info.path))
                .or_else(|| task_input.cwd.as_deref().map(std::path::PathBuf::from));
            // No daemon registration for in-process subagents — they're
            // tracked via `state.background_tasks` and the assistant
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
                    .send(EngineEvent::Task(TaskEvent::Failed {
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
                    .send(EngineEvent::Task(TaskEvent::Completed {
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
                    .send(EngineEvent::Tool(ToolEvent::Result {
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
        spawn_workflow(WorkflowSpawn {
            tc,
            tx,
            provider: provider.clone(),
            model: model.clone(),
            current_session_id: current_session_id.clone(),
            cancel: cancel.clone(),
            done,
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
        let done = send_all_complete;
        // Let the scheduler settle every started tool before emitting
        // AllComplete. Dropping this future on cancellation drops its
        // JoinHandles, and Tokio treats that as detach rather than abort:
        // stale tool tasks can keep running and report after the turn was
        // announced complete. ESCx2 still SIGTERMs tracked bash subprocesses;
        // this await keeps the transcript/event ordering coherent.
        let cancel_batch = cancel;
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

struct WorkflowSpawn<'a, F> {
    tc: ToolCall,
    tx: &'a mpsc::Sender<EngineEvent>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    current_session_id: Option<String>,
    cancel: CancellationToken,
    done: F,
}

/// Resolve, register, and spawn a Workflow tool call. Returns immediately
/// after sending the `async_launched` ToolResult; the workflow runs in the
/// background and injects a `<task-notification>` when it completes.
fn spawn_workflow<F>(spawn: WorkflowSpawn<'_, F>)
where
    F: FnOnce() + Send + 'static,
{
    let WorkflowSpawn {
        tc,
        tx,
        provider,
        model,
        current_session_id,
        cancel,
        done,
    } = spawn;
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

    let tool_id = tc.id;
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
            .send(EngineEvent::Task(crate::runtime::TaskEvent::Started {
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
        let workflow_args = args.unwrap_or(serde_json::Value::Null);
        let outcome = crate::workflows::run_workflow(crate::workflows::WorkflowRunConfig {
            run_id: run_id.clone(),
            script_body: body,
            args: workflow_args.clone(),
            provider,
            model,
            session_id: current_session_id.as_ref().map(|id| id.as_str().to_owned()),
            session_dir: session_dir.clone(),
            resume_from_run_id,
            cancel,
            tx: Some(tx.clone()),
            workflow_task_id: bg_task_id.clone(),
            depth: 0,
            cwd: cwd.clone(),
            token_budget: None,
        })
        .await;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        if meta.name == "code-review" {
            let source = if workflow_args
                .get("auto")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false)
            {
                "auto"
            } else {
                "workflow"
            };
            crate::auto_review::persist_code_review_outcome(
                &cwd,
                &run_id,
                source,
                &workflow_args,
                &outcome.result,
                outcome.error.as_deref(),
            )
            .await;
        }

        // ── mark the background task terminal ───────────────────────────
        // The notification body becomes the task summary so the standard
        // background-completion path surfaces it to the model and re-engages
        // the agentic loop (`maybe_resume_after_background`).
        let notification = build_task_notification(&bg_task_id, &meta.name, &outcome, elapsed_ms);
        if let Some(err) = &outcome.error {
            let _ = tx
                .send(EngineEvent::Task(crate::runtime::TaskEvent::Failed {
                    task_id: crate::ids::TaskId::from(bg_task_id.clone()),
                    error: err.clone(),
                }))
                .await;
        } else {
            let _ = tx
                .send(EngineEvent::Task(crate::runtime::TaskEvent::Completed {
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
    tx: &mpsc::Sender<EngineEvent>,
    tool_id: &crate::ids::ToolId,
    result: crate::runtime::ExecutionResult,
) {
    let tx = tx.clone();
    let tool_id = tool_id.clone();
    tokio::spawn(async move {
        let _ = tx
            .send(EngineEvent::Tool(ToolEvent::Result { tool_id, result }))
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

/// Execute a model-invocable `AskModel` tool call out-of-band: resolve the
/// requested model to a `(provider, model)`, run one tool-less prose completion
/// via the shared one-shot executor, and return its reply. The active model's
/// provider is the fallback when the requested id resolves to nothing concrete
/// but the active provider can run it (e.g. an unprefixed sibling model).
async fn run_ask_model_tool(
    requested_model: String,
    prompt: String,
    system: Option<String>,
    active_provider: Arc<dyn Provider>,
    active_model: ModelId,
    registry: Vec<Arc<dyn Provider>>,
) -> crate::runtime::ExecutionResult {
    use jfc_provider::{ProviderContent, ProviderMessage, ProviderRole, StreamOptions};

    let requested_model = requested_model.trim().to_owned();
    let prompt = prompt.trim().to_owned();
    if requested_model.is_empty() {
        return crate::runtime::ExecutionResult::failure("AskModel requires a non-empty `model`.");
    }
    if prompt.is_empty() {
        return crate::runtime::ExecutionResult::failure("AskModel requires a non-empty `prompt`.");
    }

    // Resolve the requested id against the full registry; fall back to the
    // active (provider, model) when it can't be resolved but the active model
    // matches the request, so a bare sibling id still works without a registry.
    let (target_provider, target_model) =
        match crate::runtime::bootstrap::resolve_provider_model(&registry, &requested_model) {
            Some(res) => (res.provider, res.model),
            None => {
                if active_model.as_str() == requested_model {
                    (active_provider, active_model.clone())
                } else {
                    return crate::runtime::ExecutionResult::failure(format!(
                        "AskModel could not resolve model `{requested_model}` against any \
                         configured provider."
                    ));
                }
            }
        };

    let qualified =
        crate::runtime::bootstrap::qualified_model_id(target_provider.as_ref(), &target_model);

    let mut opts = StreamOptions::new(target_model.clone()).max_tokens(ASK_MODEL_MAX_TOKENS);
    if let Some(sys) = system.as_deref().filter(|s| !s.trim().is_empty()) {
        opts = opts.system(sys.to_owned());
    }
    let messages = vec![ProviderMessage {
        role: ProviderRole::User,
        content: vec![ProviderContent::Text(prompt)],
    }];

    match crate::prompt_executor::complete_once(target_provider.as_ref(), messages, &opts).await {
        Ok(resp) => crate::runtime::ExecutionResult::success(format!(
            "{}\n\n_(answered by `{qualified}`)_",
            resp.content
        )),
        Err(e) => crate::runtime::ExecutionResult::failure(format!(
            "AskModel call to `{qualified}` failed: {e}"
        )),
    }
}

/// Execute a model-invocable `Council` tool call out-of-band. Members are the
/// active model plus the local advisor model (when configured + distinct), each
/// already resolved to a `(provider, model)` by the dispatch context — so the
/// council needs neither the transcript nor the full provider registry. The
/// active model's provider also serves as the arbiter.
#[derive(Clone, Default)]
struct CouncilToolOverrides {
    intent: Option<String>,
    mode: Option<String>,
    archive: Option<bool>,
    quorum: Option<u64>,
    retry_on_fail: Option<u64>,
    member_timeout_ms: Option<u64>,
}

async fn run_council_tool(
    question: String,
    requested_models: Vec<String>,
    overrides: CouncilToolOverrides,
    active_provider: Arc<dyn Provider>,
    active_model: ModelId,
    advisor_ctx: Option<LocalAdvisorDispatchContext>,
    registry: Vec<Arc<dyn Provider>>,
    task_store: Option<Arc<jfc_session::TaskStore>>,
    active_team_name: Option<String>,
    cwd: std::path::PathBuf,
) -> crate::runtime::ExecutionResult {
    use crate::council::{CouncilIntent, CouncilRequest, run_agentic_council, run_council};

    let question = question.trim().to_owned();
    if question.is_empty() {
        return crate::runtime::ExecutionResult::failure(
            "Council requires a non-empty `question`.",
        );
    }

    let cfg = crate::config::load_arc();
    let council_cfg = cfg.council.as_ref();
    let mode = council_mode_name(council_cfg, overrides.mode.as_deref());
    let agentic = match mode.as_deref() {
        Some("direct") | None => false,
        Some("agentic") => true,
        Some(other) => {
            return crate::runtime::ExecutionResult::failure(format!(
                "Council mode `{other}` is not supported. Use `direct` or `agentic`."
            ));
        }
    };

    let configured_members = council_cfg
        .map(|c| c.members.as_slice())
        .unwrap_or_default();
    let (members, unresolved) = if requested_models.is_empty() && !configured_members.is_empty() {
        resolve_configured_council_members(configured_members, &registry)
    } else {
        resolve_council_members(
            &requested_models,
            &active_provider,
            &active_model,
            advisor_ctx.as_ref(),
            &registry,
        )
    };

    if members.is_empty() {
        return crate::runtime::ExecutionResult::failure(format!(
            "Council could not resolve any models{}.",
            if unresolved.is_empty() {
                String::new()
            } else {
                format!(" from: {}", unresolved.join(", "))
            }
        ));
    }

    let mut request = CouncilRequest::new(question, members);
    if let Some(cfg) = council_cfg {
        request = apply_council_config(request, cfg);
    }
    request = apply_council_tool_overrides(request, &overrides);
    if request.options.intent.is_none()
        && let Some(intent) = overrides.intent.as_deref().and_then(CouncilIntent::parse)
    {
        request = request.with_intent(Some(intent));
    }
    let council_result = if agentic {
        run_agentic_council(
            request,
            task_store,
            active_team_name.as_deref(),
            cwd.clone(),
        )
        .await
    } else {
        run_council(request).await
    };
    match council_result {
        Ok(report) => {
            let mut body = report.to_markdown();
            if !unresolved.is_empty() {
                body.push_str(&format!(
                    "\n\n_(skipped unresolved models: {})_",
                    unresolved.join(", ")
                ));
            }
            crate::runtime::ExecutionResult::success(body)
        }
        Err(e) => crate::runtime::ExecutionResult::failure(format!("Council failed: {e}")),
    }
}

fn council_mode_name(
    cfg: Option<&jfc_config::CouncilConfig>,
    override_mode: Option<&str>,
) -> Option<String> {
    override_mode
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.to_ascii_lowercase())
        .or_else(|| {
            cfg.map(|cfg| match cfg.mode {
                jfc_config::CouncilMode::Direct => "direct".to_owned(),
                jfc_config::CouncilMode::Agentic => "agentic".to_owned(),
            })
        })
}

fn apply_council_config(
    mut request: crate::council::CouncilRequest,
    cfg: &jfc_config::CouncilConfig,
) -> crate::council::CouncilRequest {
    request = request
        .with_quorum(cfg.quorum)
        .with_retry_on_fail(cfg.retry_on_fail)
        .with_archive(
            cfg.archive,
            Some(std::env::current_dir().unwrap_or_default()),
        );
    request = if cfg.member_timeout_ms == 0 {
        request.with_member_timeout(None)
    } else {
        request.with_member_timeout(Some(std::time::Duration::from_millis(
            cfg.member_timeout_ms,
        )))
    };
    if let Some(intent) = cfg
        .intent
        .as_deref()
        .and_then(crate::council::CouncilIntent::parse)
    {
        request = request.with_intent(Some(intent));
    }
    request
}

fn apply_council_tool_overrides(
    mut request: crate::council::CouncilRequest,
    overrides: &CouncilToolOverrides,
) -> crate::council::CouncilRequest {
    if let Some(quorum) = overrides.quorum {
        request = request.with_quorum(Some(quorum.max(1) as usize));
    }
    if let Some(retry) = overrides.retry_on_fail {
        request = request.with_retry_on_fail(retry.min(u32::MAX as u64) as u32);
    }
    if let Some(ms) = overrides.member_timeout_ms {
        request = if ms == 0 {
            request.with_member_timeout(None)
        } else {
            request.with_member_timeout(Some(std::time::Duration::from_millis(ms)))
        };
    }
    if let Some(archive) = overrides.archive {
        request = request.with_archive(archive, Some(std::env::current_dir().unwrap_or_default()));
    }
    if let Some(intent) = overrides
        .intent
        .as_deref()
        .and_then(crate::council::CouncilIntent::parse)
    {
        request = request.with_intent(Some(intent));
    }
    request
}

/// Build the council's de-duplicated member list. With an explicit `requested`
/// list, ids are resolved against the full provider `registry` (unresolved ids
/// returned separately); otherwise the council defaults to the active model
/// plus all local advisor targets (when distinct).
fn resolve_council_members(
    requested: &[String],
    active_provider: &Arc<dyn Provider>,
    active_model: &ModelId,
    advisor_ctx: Option<&LocalAdvisorDispatchContext>,
    registry: &[Arc<dyn Provider>],
) -> (Vec<crate::council::CouncilMember>, Vec<String>) {
    use crate::council::CouncilMember;

    let mut members: Vec<CouncilMember> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut unresolved: Vec<String> = Vec::new();

    let mut add = |provider: Arc<dyn Provider>,
                   model: ModelId,
                   members: &mut Vec<CouncilMember>| {
        // Dedup on the fully-qualified provider/model, not the bare model
        // id: a council's whole point is fanning out to genuinely distinct
        // models, and two providers can legitimately serve the same id
        // (e.g. an OpenRouter `claude-opus` vs a first-party one). Keying on
        // the bare id collapsed those into one member. The label is
        // qualified too so the report shows which provider answered.
        let qualified = crate::runtime::bootstrap::qualified_model_id(provider.as_ref(), &model);
        if seen.insert(qualified.clone()) {
            members.push(CouncilMember::new(provider, model).with_label(qualified));
        }
    };

    if requested.is_empty() {
        add(active_provider.clone(), active_model.clone(), &mut members);
        if let Some(ctx) = advisor_ctx {
            for target in &ctx.targets {
                add(target.provider.clone(), target.model.clone(), &mut members);
            }
        }
    } else {
        for id in requested {
            match crate::runtime::bootstrap::resolve_provider_model(registry, id) {
                Some(res) => add(res.provider, res.model, &mut members),
                None => unresolved.push(id.clone()),
            }
        }
    }

    (members, unresolved)
}

fn resolve_configured_council_members(
    configured: &[jfc_config::CouncilMemberConfig],
    registry: &[Arc<dyn Provider>],
) -> (Vec<crate::council::CouncilMember>, Vec<String>) {
    use crate::council::CouncilMember;

    let mut members = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut unresolved = Vec::new();

    for member in configured {
        let model = member.model.trim();
        if model.is_empty() {
            continue;
        }
        match crate::runtime::bootstrap::resolve_provider_model(registry, model) {
            Some(res) => {
                let qualified = crate::runtime::bootstrap::qualified_model_id(
                    res.provider.as_ref(),
                    &res.model,
                );
                if seen.insert(qualified.clone()) {
                    let label = member
                        .name
                        .as_deref()
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(&qualified)
                        .to_owned();
                    members.push(CouncilMember::new(res.provider, res.model).with_label(label));
                }
            }
            None => unresolved.push(model.to_owned()),
        }
    }

    (members, unresolved)
}

#[cfg(test)]
mod isolation_inheritance_tests {
    use super::{apply_default_task_isolation_if_omitted, inherit_agent_isolation_if_omitted};
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn task_input(isolation: Option<&str>) -> crate::types::TaskInput {
        crate::types::TaskInput {
            description: "inspect".into(),
            prompt: "inspect".into(),
            subagent_type: Some("implementer".into()),
            category: None,
            run_in_background: false,
            model: None,
            launcher: None,
            effort: None,
            name: None,
            team_name: None,
            mode: None,
            isolation: isolation.map(str::to_owned),
            parent_task_id: None,
            schema: None,
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            cwd: None,
        }
    }

    fn agent_def(isolation: Option<&str>) -> crate::agents::AgentDef {
        crate::agents::AgentDef {
            name: "implementer".into(),
            source: PathBuf::from("test"),
            model: None,
            isolation: isolation.map(str::to_owned),
            skills: Vec::new(),
            allowed_tools: Vec::new(),
            disallowed_tools: Vec::new(),
            permission_mode: None,
            forks_parent_context: None,
            background: None,
            color: None,
            effort: None,
            max_turns: None,
            max_input_tokens: None,
            memory: None,
            mcp_servers: Vec::new(),
            hooks: HashMap::new(),
            key_trigger: None,
            use_when: Vec::new(),
            avoid_when: Vec::new(),
            cost: None,
            system_prompt: String::new(),
        }
    }

    #[test]
    fn task_inherits_agent_worktree_isolation_when_omitted_regression() {
        let mut task = task_input(None);
        let agent = agent_def(Some("worktree"));

        inherit_agent_isolation_if_omitted(&mut task, Some(&agent));

        assert_eq!(task.isolation.as_deref(), Some("worktree"));
    }

    #[test]
    fn explicit_task_isolation_wins_over_agent_default_normal() {
        let mut task = task_input(Some("worktree"));
        let agent = agent_def(Some("other"));

        inherit_agent_isolation_if_omitted(&mut task, Some(&agent));

        assert_eq!(task.isolation.as_deref(), Some("worktree"));
    }

    #[test]
    fn task_inherits_config_default_isolation_after_agent_default_normal() {
        let mut task = task_input(None);
        let agent = agent_def(None);

        inherit_agent_isolation_if_omitted(&mut task, Some(&agent));
        apply_default_task_isolation_if_omitted(&mut task, Some(" worktree "));

        assert_eq!(task.isolation.as_deref(), Some("worktree"));
    }

    #[test]
    fn agent_isolation_wins_over_config_default_normal() {
        let mut task = task_input(None);
        let agent = agent_def(Some("worktree"));

        inherit_agent_isolation_if_omitted(&mut task, Some(&agent));
        apply_default_task_isolation_if_omitted(&mut task, Some("other"));

        assert_eq!(task.isolation.as_deref(), Some("worktree"));
    }
}

#[cfg(test)]
mod spawn_fallback_tests {
    use super::spawn_error_means_run_inproc;

    #[test]
    fn resource_busy_triggers_inproc_fallback_normal() {
        let busy: std::io::Result<u32> = Err(std::io::Error::new(
            std::io::ErrorKind::ResourceBusy,
            "8/8 already running",
        ));
        assert!(spawn_error_means_run_inproc(&busy));
    }

    #[test]
    fn other_errors_do_not_fall_back_robust() {
        let other: std::io::Result<u32> = Err(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "nope",
        ));
        assert!(!spawn_error_means_run_inproc(&other));
        let not_found: std::io::Result<u32> =
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "x"));
        assert!(!spawn_error_means_run_inproc(&not_found));
    }

    #[test]
    fn success_does_not_fall_back_normal() {
        let ok: std::io::Result<u32> = Ok(12345);
        assert!(!spawn_error_means_run_inproc(&ok));
    }
}

#[cfg(test)]
#[path = "tool_dispatch_task_fanout_tests.rs"]
mod task_fanout_tests;

#[cfg(test)]
mod council_member_tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use async_trait::async_trait;
    use jfc_provider::{
        CompletionResponse, EventStream, ModelInfo, ProviderMessage as PMsg, StreamConvention,
        StreamOptions as SOpts,
    };

    struct NamedProvider {
        name: &'static str,
        /// When set, `complete()` returns this canned answer; otherwise it errors.
        reply: Option<&'static str>,
    }

    impl NamedProvider {
        fn new(name: &'static str) -> Self {
            Self { name, reply: None }
        }
        fn answering(name: &'static str, reply: &'static str) -> Self {
            Self {
                name,
                reply: Some(reply),
            }
        }
    }

    #[async_trait]
    impl Provider for NamedProvider {
        fn name(&self) -> &str {
            self.name
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        fn stream_convention(&self) -> StreamConvention {
            StreamConvention::AnthropicNative
        }
        async fn stream(&self, _m: Vec<PMsg>, _o: &SOpts) -> Result<EventStream> {
            Err(anyhow!("unused"))
        }
        async fn complete(&self, _m: Vec<PMsg>, _o: &SOpts) -> Result<CompletionResponse> {
            match self.reply {
                Some(content) => Ok(CompletionResponse {
                    content: content.to_owned(),
                    usage: jfc_provider::TokenUsage {
                        input_tokens: 10,
                        output_tokens: 5,
                        thinking_tokens: None,
                        cache_read_tokens: 0,
                        cache_creation_tokens: 0,
                    },
                    context_signals: None,
                    reasoning: None,
                }),
                None => Err(anyhow!("unused")),
            }
        }
    }
    impl jfc_provider::seal::Sealed for NamedProvider {}

    fn registry() -> Vec<Arc<dyn Provider>> {
        vec![
            Arc::new(NamedProvider::new("alpha")) as Arc<dyn Provider>,
            Arc::new(NamedProvider::new("beta")) as Arc<dyn Provider>,
        ]
    }

    fn active() -> (Arc<dyn Provider>, ModelId) {
        (
            Arc::new(NamedProvider::new("alpha")) as Arc<dyn Provider>,
            ModelId::new("active-model"),
        )
    }

    #[test]
    fn explicit_models_resolve_against_registry_normal() {
        let (ap, am) = active();
        let (members, unresolved) = resolve_council_members(
            &["alpha/model-a".to_owned(), "beta/model-b".to_owned()],
            &ap,
            &am,
            None,
            &registry(),
        );
        assert_eq!(members.len(), 2);
        assert!(unresolved.is_empty());
    }

    #[test]
    fn unresolved_models_are_reported_robust() {
        let (ap, am) = active();
        let (members, unresolved) = resolve_council_members(
            &["alpha/ok".to_owned(), "ghost/nope".to_owned()],
            &ap,
            &am,
            None,
            &registry(),
        );
        assert_eq!(members.len(), 1);
        assert_eq!(unresolved, vec!["ghost/nope".to_owned()]);
    }

    #[test]
    fn same_model_id_distinct_providers_are_kept_normal() {
        let (ap, am) = active();
        let (members, _unresolved) = resolve_council_members(
            &["alpha/dup".to_owned(), "beta/dup".to_owned()],
            &ap,
            &am,
            None,
            &registry(),
        );
        // Same bare id `dup` from two *different* providers is the whole point
        // of a council fan-out: both members are kept, distinguished by their
        // qualified labels.
        assert_eq!(members.len(), 2);
        let labels: Vec<&str> = members
            .iter()
            .map(|m| m.label.as_deref().unwrap_or(""))
            .collect();
        assert!(labels.contains(&"alpha/dup"), "{labels:?}");
        assert!(labels.contains(&"beta/dup"), "{labels:?}");
    }

    #[test]
    fn same_provider_model_is_deduped_robust() {
        let (ap, am) = active();
        // The identical qualified spec listed twice still collapses to one.
        let (members, _unresolved) = resolve_council_members(
            &["alpha/dup".to_owned(), "alpha/dup".to_owned()],
            &ap,
            &am,
            None,
            &registry(),
        );
        assert_eq!(members.len(), 1);
    }

    #[test]
    fn empty_request_falls_back_to_active_model_normal() {
        let (ap, am) = active();
        let (members, unresolved) = resolve_council_members(&[], &ap, &am, None, &registry());
        assert_eq!(members.len(), 1);
        assert!(unresolved.is_empty());
    }

    #[test]
    fn empty_request_uses_all_advisor_targets_regression() {
        let (ap, am) = active();
        let advisor_ctx = LocalAdvisorDispatchContext {
            targets: vec![
                crate::advisor::LocalAdvisorProviderTarget {
                    provider: Arc::new(NamedProvider::new("beta")) as Arc<dyn Provider>,
                    model: ModelId::new("advisor-a"),
                },
                crate::advisor::LocalAdvisorProviderTarget {
                    provider: Arc::new(NamedProvider::new("alpha")) as Arc<dyn Provider>,
                    model: ModelId::new("advisor-b"),
                },
                crate::advisor::LocalAdvisorProviderTarget {
                    provider: ap.clone(),
                    model: am.clone(),
                },
            ],
            transcript: Vec::new(),
        };

        let (members, unresolved) =
            resolve_council_members(&[], &ap, &am, Some(&advisor_ctx), &registry());

        assert!(unresolved.is_empty());
        assert_eq!(members.len(), 3);
        let labels: Vec<&str> = members
            .iter()
            .map(|m| m.label.as_deref().unwrap_or(""))
            .collect();
        assert!(labels.contains(&"alpha/active-model"), "{labels:?}");
        assert!(labels.contains(&"beta/advisor-a"), "{labels:?}");
        assert!(labels.contains(&"alpha/advisor-b"), "{labels:?}");
    }

    // ─── AskModel dispatch path ──────────────────────────────────────────────

    #[tokio::test]
    async fn ask_model_resolves_and_answers_normal() {
        // A registry provider that actually answers; AskModel resolves
        // `beta/m` to it and threads the reply back as a success result.
        let registry: Vec<Arc<dyn Provider>> = vec![
            Arc::new(NamedProvider::new("alpha")) as Arc<dyn Provider>,
            Arc::new(NamedProvider::answering("beta", "Rayleigh scattering.")) as Arc<dyn Provider>,
        ];
        let (ap, am) = active();
        let result = run_ask_model_tool(
            "beta/m".to_owned(),
            "why is the sky blue?".to_owned(),
            None,
            ap,
            am,
            registry,
        )
        .await;
        assert!(!result.is_error(), "expected success: {}", result.output);
        assert!(result.output.contains("Rayleigh scattering."));
        assert!(result.output.contains("answered by `beta/m`"));
    }

    #[tokio::test]
    async fn ask_model_unresolved_is_error_robust() {
        let (ap, am) = active();
        let result = run_ask_model_tool(
            "ghost/nope".to_owned(),
            "hi".to_owned(),
            None,
            ap,
            am,
            registry(),
        )
        .await;
        assert!(result.is_error());
        assert!(
            result
                .output
                .contains("could not resolve model `ghost/nope`")
        );
    }

    #[tokio::test]
    async fn ask_model_empty_prompt_is_error_robust() {
        let (ap, am) = active();
        let result = run_ask_model_tool(
            "beta/m".to_owned(),
            "   ".to_owned(),
            None,
            ap,
            am,
            registry(),
        )
        .await;
        assert!(result.is_error());
        assert!(result.output.contains("non-empty `prompt`"));
    }
}
