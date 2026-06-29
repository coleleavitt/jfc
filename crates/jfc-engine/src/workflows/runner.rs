//! Workflow runner — orchestrates agent dispatch with semaphore-gated concurrency.
//!
//! The runner ties together three concerns:
//!  1. The engine ([`super::engine::run_script`]) runs the user JS on a
//!     dedicated `LocalSet` and emits [`AgentRequest`]s / [`ProgressSignal`]s.
//!  2. The orchestrator loop receives those requests and dispatches each agent
//!     through [`crate::tools::execute_task`], gated by a tokio `Semaphore`
//!     (min(16, cpus-2)) and a hard 1000-agent cap.
//!  3. The journal ([`super::journal`]) records every agent call keyed by a
//!     chain hash so a resumed run replays the longest unchanged prefix.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};

use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use super::engine::{AgentRequest, ProgressSignal, SubWorkflowRequest, run_script};
use super::journal::{self, JournalCache, JournalEntry, JournalWriter};
use jfc_core::{Effort, FanoutDecision, FanoutPlan, FanoutPredictor, PlannedAgent};
use jfc_provider::{ModelId, ModelSpec, PromptCacheKey, Provider, ProviderId};

/// Cache-key namespace for workflow resume keys. Bump when the agent-request
/// hashing inputs change so a stale journal can't replay against new semantics.
const WORKFLOW_PROMPT_VERSION: &str = "workflow-agent-v1";

/// Max concurrent agent() calls = min(16, cpus - 2), floor 2.
pub fn max_concurrency() -> usize {
    let cpus = num_cpus::get();
    cpus.saturating_sub(2).clamp(2, 16)
}

/// Hard cap on total agents per workflow lifetime (CC 146: 1000).
pub const MAX_AGENTS: u32 = 1000;

/// Configuration for a single workflow run.
pub struct WorkflowRunConfig {
    pub run_id: String,
    pub script_body: String,
    pub args: serde_json::Value,
    pub provider: Arc<dyn Provider>,
    pub model: ModelId,
    /// Session id for the DB-backed resume journal.
    pub session_id: Option<String>,
    /// Session directory for temporary workflow files.
    pub session_dir: std::path::PathBuf,
    /// When set, completed agent() calls from this prior run are replayed.
    pub resume_from_run_id: Option<String>,
    /// Cancellation for the whole workflow (ESC×2 / TaskStop).
    pub cancel: CancellationToken,
    /// Event sender for live UI progress (subagent chunks, task lifecycle).
    pub tx: Option<mpsc::Sender<crate::runtime::EngineEvent>>,
    /// The workflow's own background task id (for progress routing).
    pub workflow_task_id: String,
    /// Nesting depth (0 = top-level). Max depth = 3.
    pub depth: u32,
    /// Project root for resolving sub-workflow names from the registry.
    pub cwd: std::path::PathBuf,
    /// Optional token budget (None = unlimited). When set, the orchestrator
    /// will reject new agent dispatches once tokens_spent >= token_budget.
    pub token_budget: Option<u64>,
}

/// Final outcome of a workflow run.
#[derive(Debug)]
pub struct WorkflowOutcome {
    pub result: serde_json::Value,
    pub agent_count: u32,
    pub total_agents_dispatched: u32,
    pub cache_hits: u32,
    pub logs: Vec<String>,
    pub error: Option<String>,
    /// True when the run ended because its cancellation token fired (user
    /// Ctrl+C, shutdown, or a superseding turn) rather than a genuine failure.
    /// Lets callers (e.g. auto-review) report a cancelled run distinctly
    /// instead of surfacing the orchestrator-teardown error as a crash.
    pub cancelled: bool,
}

/// Holds the orchestration state while a workflow runs. Owns the semaphore,
/// counters, journal writer, and the JoinSet of in-flight agent dispatches.
struct Orchestrator {
    semaphore: Arc<Semaphore>,
    dispatched: Arc<AtomicU32>,
    cache_hits: Arc<AtomicU32>,
    running_hash: Arc<parking_lot::Mutex<String>>,
    journal_writer: Arc<JournalWriter>,
    cache: Arc<Option<JournalCache>>,
    provider: Arc<dyn Provider>,
    model: ModelId,
    tx: Option<mpsc::Sender<crate::runtime::EngineEvent>>,
    workflow_task_id: String,
    cancel: CancellationToken,
    agent_tasks: tokio::task::JoinSet<()>,
    logs: Vec<String>,
    /// Optional hard token budget; None means unlimited.
    token_budget: Option<u64>,
    /// Running tally of tokens consumed (estimated as output_len / 4).
    tokens_spent: Arc<AtomicU64>,
    /// Predictive fan-out gate: before spawning an agent under a token budget,
    /// estimate its cost and defer if the remaining budget can't fit it, so we
    /// don't dispatch work we predict can't finish.
    fanout_predictor: FanoutPredictor,
}

impl Orchestrator {
    /// Emit a `WorkflowProgress` event if the event channel is open.
    fn emit(&self, ev: crate::runtime::WorkflowProgressEvent) {
        if let Some(tx) = &self.tx {
            let tx = tx.clone();
            tokio::spawn(async move {
                let _ = tx
                    .send(crate::runtime::EngineEvent::WorkflowProgress(ev))
                    .await;
            });
        }
    }

    fn record_progress(&mut self, sig: ProgressSignal) {
        match sig {
            ProgressSignal::Phase(ref title) => {
                self.logs.push(format!("phase: {title}"));
                self.emit(crate::runtime::WorkflowProgressEvent::Phase {
                    task_id: crate::ids::TaskId::from(self.workflow_task_id.clone()),
                    title: title.clone(),
                });
            }
            ProgressSignal::Log(ref msg) => {
                self.logs.push(msg.clone());
                self.emit(crate::runtime::WorkflowProgressEvent::Log {
                    task_id: crate::ids::TaskId::from(self.workflow_task_id.clone()),
                    message: msg.clone(),
                });
            }
        }
    }

    /// Compute the resume key for a request and advance the chain hash.
    ///
    /// The per-request material is built through [`PromptCacheKey`] so the key
    /// binds the provider, the *effective* model (a request's `model: None`
    /// inherits the orchestrator's model), and a stable hash of the request
    /// params. That prevents a resumed journal from replaying an answer
    /// produced by a different provider or model that happened to share a bare
    /// model id (e.g. litellm vs openai both exposing `gpt-4o`, or two runs
    /// under different default models). The result is still chained through
    /// `running_hash` so an agent's position in the DAG remains part of its key.
    fn next_key(&self, req: &AgentRequest) -> String {
        let effective_model = req
            .model
            .clone()
            .map(ModelId::new)
            .unwrap_or_else(|| self.model.clone());
        let params = serde_json::json!({
            "schema": req.schema,
            "agentType": req.agent_type,
            "isolation": req.isolation,
        });
        let cache_key = PromptCacheKey::new(
            WORKFLOW_PROMPT_VERSION,
            ProviderId::new(self.provider.name()),
            None,
            ModelSpec::bare(effective_model),
            &params,
            &req.prompt,
            &[],
        );
        let mut h = self.running_hash.lock();
        let k = journal::compute_key(&h, &req.prompt, &cache_key.stable_string());
        *h = k.clone();
        k
    }

    /// Handle one agent request: enforce the cap, check the resume cache, and
    /// otherwise spawn a semaphore-gated dispatch.
    fn dispatch(&mut self, req: AgentRequest) {
        // Enforce token budget before the agent cap.
        if let Some(budget) = self.token_budget {
            let spent = self.tokens_spent.load(Ordering::Relaxed);
            if spent >= budget {
                let _ = req
                    .reply
                    .send(Err("workflow token budget exhausted".to_owned()));
                return;
            }
            // Predictive gate: if the remaining budget can't fit even one
            // estimated agent, defer rather than spawn into a wall. Per-agent
            // effort isn't known here, so estimate at the Medium default.
            let plan = FanoutPlan {
                agents: vec![PlannedAgent::at(Effort::Medium)],
                remaining_budget: Some(budget - spent),
                concurrency: max_concurrency(),
            };
            if let FanoutDecision::Defer { reason } = self.fanout_predictor.gate(&plan) {
                let _ = req
                    .reply
                    .send(Err(format!("workflow fan-out gated by budget: {reason}")));
                return;
            }
        }

        if self.dispatched.load(Ordering::Relaxed) >= MAX_AGENTS {
            let _ = req
                .reply
                .send(Err(format!("workflow agent cap reached ({MAX_AGENTS})")));
            return;
        }

        let key = self.next_key(&req);

        if let Some(cached) = self
            .cache
            .as_ref()
            .as_ref()
            .and_then(|c| c.results.get(&key).cloned())
        {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
            self.emit(crate::runtime::WorkflowProgressEvent::AgentCacheHit {
                task_id: crate::ids::TaskId::from(self.workflow_task_id.clone()),
                index: req.index,
                label: req.label.clone(),
                phase: req.phase.clone(),
            });
            let text = cached
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| cached.to_string());
            let _ = req.reply.send(Ok(text));
            return;
        }

        self.dispatched.fetch_add(1, Ordering::Relaxed);
        tracing::debug!(
            target: "jfc::workflow",
            index = req.index,
            label = %req.label,
            phase = req.phase.as_deref().unwrap_or(""),
            "dispatching workflow agent"
        );
        self.emit(crate::runtime::WorkflowProgressEvent::AgentStarted {
            task_id: crate::ids::TaskId::from(self.workflow_task_id.clone()),
            index: req.index,
            label: req.label.clone(),
            phase: req.phase.clone(),
        });

        let permit_sem = self.semaphore.clone();
        let provider = self.provider.clone();
        let model = self.model.clone();
        let tx = self.tx.clone();
        let journal_writer = self.journal_writer.clone();
        let workflow_task_id = self.workflow_task_id.clone();
        let cancel = self.cancel.clone();
        let tokens_spent = self.tokens_spent.clone();
        self.agent_tasks.spawn(async move {
            // Race the permit acquire against cancellation. A bare
            // `acquire().await` would wedge a queued agent until a permit frees
            // even after the workflow is cancelled, so the run can't abort
            // promptly. On cancel, reply with an error and bail.
            let _permit = tokio::select! {
                biased;
                _ = cancel.cancelled() => {
                    let _ = req.reply.send(Err("workflow cancelled".to_owned()));
                    return;
                }
                permit = permit_sem.acquire() => match permit {
                    Ok(p) => p,
                    // Semaphore closed — treat as cancellation.
                    Err(_) => {
                        let _ = req.reply.send(Err("workflow cancelled".to_owned()));
                        return;
                    }
                },
            };
            run_one_agent(
                req,
                key,
                provider,
                model,
                tx,
                journal_writer,
                workflow_task_id,
                cancel,
                tokens_spent,
            )
            .await;
        });
    }
}

/// Run a workflow end-to-end. Spawns the engine on a dedicated thread, then
/// services agent requests on the current runtime until the script settles.
pub async fn run_workflow(config: WorkflowRunConfig) -> WorkflowOutcome {
    let WorkflowRunConfig {
        run_id,
        script_body,
        args,
        provider,
        model,
        session_id,
        session_dir,
        resume_from_run_id,
        cancel,
        tx,
        workflow_task_id,
        depth,
        cwd,
        token_budget,
    } = config;

    // Resume cache: load the prior run's journal if requested.
    let cache: Option<JournalCache> = match &resume_from_run_id {
        Some(prev) => {
            let c = journal::load_journal(session_id.as_deref(), prev).await;
            // Warn about agents that were started but never completed in the
            // prior run — their results won't be in the cache and will re-run.
            let incomplete: Vec<&String> = c
                .started
                .keys()
                .filter(|k| !c.results.contains_key(*k))
                .collect();
            if !incomplete.is_empty() {
                tracing::debug!(
                    target: "jfc::workflow",
                    count = incomplete.len(),
                    "resume: {} agent(s) from prior run were interrupted and will re-run",
                    incomplete.len()
                );
            }
            Some(c)
        }
        None => None,
    };
    let journal_writer = JournalWriter::new(session_id.as_deref(), &run_id);
    tracing::debug!(
        target: "jfc::workflow",
        journal = %journal_writer.label(),
        "workflow journal opened"
    );

    // Channels bridging the engine thread to this orchestrator.
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentRequest>();
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<ProgressSignal>();
    let (sub_wf_tx, mut sub_wf_rx) = mpsc::unbounded_channel::<SubWorkflowRequest>();

    // Spawn the engine on a dedicated OS thread with its own LocalSet — boa's
    // Context and promise futures are !Send so they cannot live on the shared
    // multithreaded runtime.
    let engine_args = args.clone();
    let engine_body = script_body.clone();
    let engine_handle = std::thread::spawn(move || {
        let rt = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                return super::engine::EngineOutcome {
                    result: serde_json::Value::Null,
                    agent_count: 0,
                    error: Some(format!("failed to build engine runtime: {e}")),
                };
            }
        };
        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async move {
            run_script(
                &engine_body,
                engine_args,
                agent_tx,
                progress_tx,
                sub_wf_tx,
                token_budget,
            )
            .await
        })
    });

    // Orchestration state.
    let mut orch = Orchestrator {
        semaphore: Arc::new(Semaphore::new(max_concurrency())),
        dispatched: Arc::new(AtomicU32::new(0)),
        cache_hits: Arc::new(AtomicU32::new(0)),
        running_hash: Arc::new(parking_lot::Mutex::new(String::new())),
        journal_writer: Arc::new(journal_writer),
        cache: Arc::new(cache),
        provider,
        model,
        tx,
        workflow_task_id,
        cancel: cancel.clone(),
        agent_tasks: tokio::task::JoinSet::new(),
        logs: Vec::new(),
        token_budget,
        tokens_spent: Arc::new(AtomicU64::new(0)),
        fanout_predictor: FanoutPredictor::default(),
    };

    // Drive both channels until the engine thread finishes.
    let mut cancelled = false;
    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
                cancelled = true;
                agent_rx.close();
                sub_wf_rx.close();
                orch.logs.push("workflow cancelled".to_owned());
                break;
            }

            Some(sig) = progress_rx.recv() => orch.record_progress(sig),

            maybe_req = agent_rx.recv() => {
                match maybe_req {
                    Some(req) => orch.dispatch(req),
                    // Engine dropped the agent sender — script finished.
                    None => break,
                }
            }

            maybe_sub = sub_wf_rx.recv() => {
                if let Some(sub_req) = maybe_sub {
                    // Enforce max depth.
                    if depth >= 3 {
                        let _ = sub_req.reply.send(Err(format!(
                            "workflow nesting limit reached (max depth 3, current depth {})",
                            depth
                        )));
                        continue;
                    }
                    // Resolve the named workflow from the registry.
                    let resolved = crate::workflows::resolve(&cwd, &sub_req.name);
                    let Some(registered) = resolved else {
                        let _ = sub_req.reply.send(Err(format!(
                            "sub-workflow '{}' not found",
                            sub_req.name
                        )));
                        continue;
                    };
                    // Parse meta + body.
                    let body = match crate::workflows::parse_meta(&registered.script) {
                        Ok((_, b)) => b,
                        Err(e) => {
                            let _ = sub_req.reply.send(Err(format!(
                                "failed to parse sub-workflow '{}': {e}",
                                sub_req.name
                            )));
                            continue;
                        }
                    };
                    // Build a fresh run_id for the child.
                    let child_run_id = format!(
                        "{}_sub{}_{}",
                        run_id,
                        depth + 1,
                        crate::workflows::generate_run_id()
                    );
                    let child_config = WorkflowRunConfig {
                        run_id: child_run_id,
                        script_body: body,
                        args: sub_req.args,
                        provider: orch.provider.clone(),
                        model: orch.model.clone(),
                        session_id: session_id.clone(),
                        session_dir: session_dir.clone(),
                        resume_from_run_id: None,
                        cancel: orch.cancel.clone(),
                        tx: orch.tx.clone(),
                        workflow_task_id: orch.workflow_task_id.clone(),
                        depth: depth + 1,
                        cwd: cwd.clone(),
                        // Propagate the parent's remaining budget so a child
                        // workflow can't bypass a token cap. `None` stays
                        // unlimited; otherwise pass what's left after the
                        // parent's spend so far (saturating at 0).
                        token_budget: orch.token_budget.map(|budget| {
                            budget.saturating_sub(orch.tokens_spent.load(Ordering::Relaxed))
                        }),
                    };
                    // Run the child workflow and reply.
                    let sub_outcome = Box::pin(run_workflow(child_config)).await;
                    let reply_val = if let Some(err) = sub_outcome.error {
                        Err(err)
                    } else {
                        Ok(sub_outcome.result)
                    };
                    let _ = sub_req.reply.send(reply_val);
                }
            }
        }
    }

    if cancelled {
        let rejected = reject_pending_requests(&mut agent_rx, &mut sub_wf_rx);
        if rejected > 0 {
            orch.logs
                .push(format!("rejected {rejected} queued workflow request(s)"));
        }
    }

    // Wait for in-flight agents to settle. On cancellation, cap the grace
    // period so a stuck provider/tool future cannot wedge workflow teardown.
    if drain_agent_tasks(&mut orch.agent_tasks, cancelled).await {
        orch.logs
            .push("aborted lingering workflow agent task(s) after cancellation".to_owned());
    }

    // Drain any trailing progress signals.
    while let Ok(sig) = progress_rx.try_recv() {
        orch.record_progress(sig);
    }

    let dispatched = orch.dispatched.clone();
    let cache_hits = orch.cache_hits.clone();
    let logs = std::mem::take(&mut orch.logs);
    drop(orch);

    // Join the engine thread to collect the script's return value.
    let engine_outcome = match engine_handle.join() {
        Ok(o) => o,
        Err(_) => super::engine::EngineOutcome {
            result: serde_json::Value::Null,
            agent_count: dispatched.load(Ordering::Relaxed),
            error: Some("workflow engine thread panicked".to_owned()),
        },
    };

    // When the run was cancelled, the engine thread's error is the
    // orchestrator-teardown artifact ("workflow orchestrator unavailable" /
    // "workflow cancelled") rather than a real failure. Surface cancellation
    // explicitly so callers don't treat a deliberately-stopped run as a crash.
    WorkflowOutcome {
        result: engine_outcome.result,
        agent_count: engine_outcome.agent_count,
        total_agents_dispatched: dispatched.load(Ordering::Relaxed),
        cache_hits: cache_hits.load(Ordering::Relaxed),
        logs,
        error: engine_outcome.error,
        cancelled,
    }
}

fn reject_pending_requests(
    agent_rx: &mut mpsc::UnboundedReceiver<AgentRequest>,
    sub_wf_rx: &mut mpsc::UnboundedReceiver<SubWorkflowRequest>,
) -> usize {
    let mut rejected = 0;
    while let Ok(req) = agent_rx.try_recv() {
        let _ = req.reply.send(Err("workflow cancelled".to_owned()));
        rejected += 1;
    }
    while let Ok(req) = sub_wf_rx.try_recv() {
        let _ = req.reply.send(Err("workflow cancelled".to_owned()));
        rejected += 1;
    }
    rejected
}

async fn drain_agent_tasks(agent_tasks: &mut tokio::task::JoinSet<()>, cancelled: bool) -> bool {
    drain_agent_tasks_with_grace(agent_tasks, cancelled, std::time::Duration::from_secs(5)).await
}

async fn drain_agent_tasks_with_grace(
    agent_tasks: &mut tokio::task::JoinSet<()>,
    cancelled: bool,
    grace: std::time::Duration,
) -> bool {
    if !cancelled {
        while agent_tasks.join_next().await.is_some() {}
        return false;
    }

    let timed_out = tokio::time::timeout(grace, async {
        while agent_tasks.join_next().await.is_some() {}
    })
    .await
    .is_err();

    if timed_out {
        agent_tasks.abort_all();
        while agent_tasks.join_next().await.is_some() {}
    }
    timed_out
}

/// Dispatch one agent through `execute_task`, write the journal, and reply.
async fn run_one_agent(
    req: AgentRequest,
    key: String,
    provider: Arc<dyn Provider>,
    model: ModelId,
    tx: Option<mpsc::Sender<crate::runtime::EngineEvent>>,
    journal_writer: Arc<JournalWriter>,
    workflow_task_id: String,
    cancel: CancellationToken,
    tokens_spent: Arc<AtomicU64>,
) {
    let agent_id = format!("{workflow_task_id}:agent_{}", req.index);

    // Record the started journal entry.
    let _ = journal_writer
        .append(&JournalEntry::Started {
            key: key.clone(),
            agent_id: agent_id.clone(),
        })
        .await;

    // In-process workflow sub-agents get a daemon-registry row created lazily
    // the first time their streamed output is recorded (under `agent_id`).
    // Nothing ever recorded a *terminal* status for that row, so after the UI
    // exited the reconcile pass marked every one "stale: owning process
    // exited" — a phantom Failed per sub-agent that leaked into
    // `daemon-state.json` forever. We now finalize the row on every exit path
    // below via `finalize_agent_row`.

    let agent_def = resolve_agent_def(&req);
    let task_input = build_agent_task_input(&req, agent_def.as_ref());
    let worktree_info = match prepare_workflow_agent_worktree(&task_input, &agent_id).await {
        Ok(info) => info,
        Err(error) => {
            finalize_agent_row(&agent_id, AgentRowOutcome::Failed(&error));
            emit_agent_failed(&tx, &workflow_task_id, req.index, error.clone());
            let _ = req.reply.send(Err(error));
            return;
        }
    };
    let cwd_override = worktree_info
        .as_ref()
        .map(|info| std::path::PathBuf::from(&info.worktree.path))
        .or_else(|| task_input.cwd.as_deref().map(std::path::PathBuf::from));

    // Race the dispatch against cancellation.
    let result = tokio::select! {
        biased;
        _ = cancel.cancelled() => {
            crate::runtime::ExecutionResult::failure("workflow cancelled")
        }
        r = crate::tools::execute_task(
            &task_input,
            provider.as_ref(),
            model,
            tx.as_ref(),
            Some(&agent_id),
            agent_def.as_ref(),
            cwd_override,
            None,
            None,
        ) => r,
    };
    finish_workflow_agent_worktree(&agent_id, worktree_info).await;

    if result.is_error() {
        // Finalize the daemon row so it never goes stale-Failed after the UI
        // exits. Cancellation is recorded distinctly from a genuine failure.
        finalize_agent_row(
            &agent_id,
            agent_error_outcome(cancel.is_cancelled(), &result.output),
        );
        emit_agent_failed(&tx, &workflow_task_id, req.index, result.output.clone());
        let _ = req.reply.send(Err(result.output.clone()));
        return;
    }

    let text = result.output.clone();

    // Accumulate token estimate (4 chars ≈ 1 token).
    let token_estimate = text.len() as u64 / 4;
    tokens_spent.fetch_add(token_estimate, Ordering::Relaxed);

    let structured_result = match parse_structured_result(&req, &text) {
        Ok(v) => v,
        Err(error) => {
            // Schema contract violation: surface as an agent failure rather
            // than silently stringifying (which used to hide schema breaches
            // and surprise scripts expecting object fields).
            emit_agent_failed(&tx, &workflow_task_id, req.index, error.clone());
            finalize_agent_row(
                &agent_id,
                agent_error_outcome(cancel.is_cancelled(), &error),
            );
            let _ = journal_writer
                .append(&JournalEntry::Result {
                    key,
                    agent_id,
                    result: serde_json::Value::String(text.clone()),
                })
                .await;
            let _ = req.reply.send(Err(error));
            return;
        }
    };

    let journal_result = structured_result
        .clone()
        .unwrap_or_else(|| serde_json::Value::String(text.clone()));

    // Finalize the daemon row as Completed before consuming `agent_id`.
    finalize_agent_row(&agent_id, AgentRowOutcome::Completed);

    // Record the result journal entry.
    let _ = journal_writer
        .append(&JournalEntry::Result {
            key,
            agent_id,
            result: journal_result,
        })
        .await;

    emit_agent_done(&tx, &workflow_task_id, req.index);

    // For schema agents, hand back the canonical JSON serialization of the
    // parsed object so the workflow bridge resolves it to a JS object with real
    // fields (rather than the raw, possibly-pretty-printed text). Non-schema
    // agents return their text verbatim.
    let reply_text = match &structured_result {
        Some(v) => serde_json::to_string(v).unwrap_or(text),
        None => text,
    };
    let _ = req.reply.send(Ok(reply_text));
}

/// Build the `TaskInput` for a workflow sub-agent dispatch from its request.
fn build_agent_task_input(
    req: &AgentRequest,
    agent_def: Option<&crate::agents::AgentDef>,
) -> crate::types::TaskInput {
    let mut task_input = crate::types::TaskInput {
        description: req.label.clone(),
        prompt: req.prompt.clone(),
        subagent_type: req.agent_type.clone(),
        category: None,
        run_in_background: false,
        model: req.model.clone(),
        launcher: None,
        effort: None,
        name: None,
        team_name: None,
        mode: None,
        isolation: req.isolation.clone(),
        parent_task_id: None,
        schema: req.schema.clone(),
        allowed_tools: Vec::new(),
        disallowed_tools: Vec::new(),
        cwd: None,
    };
    apply_workflow_agent_isolation_defaults(
        &mut task_input,
        agent_def,
        crate::config::load_arc()
            .isolation
            .as_ref()
            .and_then(|isolation| isolation.default_task_isolation.as_deref()),
    );
    task_input
}

fn apply_workflow_agent_isolation_defaults(
    task_input: &mut crate::types::TaskInput,
    agent_def: Option<&crate::agents::AgentDef>,
    config_default: Option<&str>,
) {
    if task_input.isolation.is_none()
        && let Some(isolation) = agent_def.and_then(|agent| agent.isolation.as_deref())
    {
        task_input.isolation = Some(isolation.to_owned());
    }
    if task_input.isolation.is_none()
        && let Some(default) = config_default.map(str::trim).filter(|s| !s.is_empty())
    {
        task_input.isolation = Some(default.to_owned());
    }
}

/// Resolve a custom agent definition when the request named an `agentType`.
fn resolve_agent_def(req: &AgentRequest) -> Option<crate::agents::AgentDef> {
    let cwd = std::env::current_dir().unwrap_or_default();
    let agents = crate::agents::load_agents(&cwd);
    req.agent_type
        .as_deref()
        .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)))
        .cloned()
}

struct WorkflowAgentWorktree {
    worktree: crate::worktrees::WorktreeInfo,
    repo_root: std::path::PathBuf,
    change_id: Option<String>,
}

async fn prepare_workflow_agent_worktree(
    task_input: &crate::types::TaskInput,
    agent_id: &str,
) -> Result<Option<WorkflowAgentWorktree>, String> {
    if task_input.isolation.as_deref() != Some("worktree") {
        return Ok(None);
    }

    let suffix: String = agent_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(12)
        .collect();
    let name = format!(
        "agent-{}",
        if suffix.is_empty() {
            "workflow"
        } else {
            suffix.as_str()
        }
    );
    let cwd = task_input
        .cwd
        .as_deref()
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
    let repo_root = match crate::worktrees::find_repo_root_async(&cwd).await {
        Ok(root) => root,
        Err(e) => {
            tracing::warn!(
                target: "jfc::workflow",
                cwd = %cwd.display(),
                error = %e,
                "workflow agent: failed to resolve git root, falling back to cwd for worktree"
            );
            cwd
        }
    };

    match crate::worktrees::create_worktree_async(&repo_root, &name).await {
        Ok(worktree) => {
            tracing::info!(
                target: "jfc::workflow",
                repo_root = %repo_root.display(),
                worktree = %worktree.path,
                "workflow agent: created worktree for isolated agent"
            );
            let origin = crate::changeset::ChangeOrigin {
                task_id: Some(agent_id.to_owned()),
                agent_id: task_input
                    .subagent_type
                    .clone()
                    .or_else(|| Some("workflow".to_owned())),
                session_id: None,
            };
            let change_id = crate::changeset::open_for_worktree(
                &repo_root,
                &worktree.path,
                &worktree.branch,
                &origin,
            )
            .await;
            Ok(Some(WorkflowAgentWorktree {
                worktree,
                repo_root,
                change_id,
            }))
        }
        Err(e) => match crate::changeset::isolation_fallback() {
            crate::changeset::IsolationFallback::FailClosed => Err(format!(
                "Refusing to run isolated workflow agent in the main checkout: \
                 worktree creation failed ({e}). Isolation is fail-closed \
                 (set [isolation] fail_closed = false or JFC_ISOLATION_FAIL_CLOSED=0 \
                 to allow the cwd fallback)."
            )),
            crate::changeset::IsolationFallback::AllowCwd => {
                tracing::warn!(
                    target: "jfc::workflow",
                    repo_root = %repo_root.display(),
                    error = %e,
                    "workflow agent: failed to create worktree, running in cwd (fail-open)"
                );
                Ok(None)
            }
        },
    }
}

async fn finish_workflow_agent_worktree(
    agent_id: &str,
    worktree_info: Option<WorkflowAgentWorktree>,
) {
    let Some(info) = worktree_info else { return };
    if let Some(ref change_id) = info.change_id {
        crate::changeset::finalize_for_worktree(&info.repo_root, change_id, &info.worktree.path)
            .await;
    }
    let dirty = match tokio::process::Command::new("git")
        .arg("-C")
        .arg(&info.worktree.path)
        .arg("status")
        .arg("--porcelain")
        .output()
        .await
    {
        Ok(out) if out.status.success() => !out.stdout.is_empty(),
        Ok(out) => {
            tracing::warn!(
                target: "jfc::workflow",
                agent_id,
                worktree = %info.worktree.path,
                stderr = %String::from_utf8_lossy(&out.stderr),
                "git status in workflow worktree returned non-zero — preserving worktree"
            );
            true
        }
        Err(e) => {
            tracing::warn!(
                target: "jfc::workflow",
                agent_id,
                worktree = %info.worktree.path,
                error = %e,
                "git status spawn failed for workflow worktree — preserving worktree"
            );
            true
        }
    };
    if dirty {
        tracing::info!(
            target: "jfc::workflow",
            agent_id,
            worktree = %info.worktree.path,
            branch = %info.worktree.branch,
            "workflow worktree has uncommitted changes — preserving"
        );
        return;
    }
    let wt_name = std::path::Path::new(&info.worktree.path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("");
    match crate::worktrees::remove_worktree_async(&info.repo_root, wt_name).await {
        Ok(_) => tracing::info!(
            target: "jfc::workflow",
            agent_id,
            worktree = %info.worktree.path,
            "workflow worktree had no changes — removed"
        ),
        Err(e) => tracing::warn!(
            target: "jfc::workflow",
            agent_id,
            worktree = %info.worktree.path,
            error = %e,
            "workflow worktree cleanup failed"
        ),
    }
}

/// Parse a schema-bound agent's output into a JSON object. Returns `Ok(None)`
/// for non-schema agents, `Ok(Some(value))` on success, and `Err(message)`
/// when a schema was declared but the output isn't valid JSON.
fn parse_structured_result(
    req: &AgentRequest,
    text: &str,
) -> Result<Option<serde_json::Value>, String> {
    if req.schema.is_none() {
        return Ok(None);
    }
    serde_json::from_str::<serde_json::Value>(text)
        .map(Some)
        .map_err(|e| {
            format!(
                "workflow agent declared a StructuredOutput schema but returned \
                 output that is not valid JSON: {e}"
            )
        })
}

/// Spawn a fire-and-forget send of an `AgentFailed` progress event. A no-op
/// when no event sender is wired (e.g. tests). Keeping the spawn here keeps
/// `run_one_agent`'s nesting shallow.
fn emit_agent_failed(
    tx: &Option<mpsc::Sender<crate::runtime::EngineEvent>>,
    workflow_task_id: &str,
    index: u32,
    error: String,
) {
    let Some(tx) = tx.clone() else { return };
    let task_id = crate::ids::TaskId::from(workflow_task_id.to_owned());
    tokio::spawn(async move {
        let _ = tx
            .send(crate::runtime::EngineEvent::WorkflowProgress(
                crate::runtime::WorkflowProgressEvent::AgentFailed {
                    task_id,
                    index,
                    error,
                },
            ))
            .await;
    });
}

/// Spawn a fire-and-forget send of an `AgentDone` progress event. A no-op when
/// no event sender is wired.
fn emit_agent_done(
    tx: &Option<mpsc::Sender<crate::runtime::EngineEvent>>,
    workflow_task_id: &str,
    index: u32,
) {
    let Some(tx) = tx.clone() else { return };
    let task_id = crate::ids::TaskId::from(workflow_task_id.to_owned());
    tokio::spawn(async move {
        let _ = tx
            .send(crate::runtime::EngineEvent::WorkflowProgress(
                crate::runtime::WorkflowProgressEvent::AgentDone { task_id, index },
            ))
            .await;
    });
}

/// Outcome of a single workflow sub-agent dispatch, used to finalize its
/// daemon-registry row with the right terminal status.
enum AgentRowOutcome<'a> {
    Completed,
    Cancelled,
    Failed(&'a str),
}

/// Record a terminal status for an in-process workflow sub-agent's daemon
/// registry row.
///
/// The row is created lazily (epoch 0) the first time the sub-agent streams
/// output under `agent_id`; if it was never created (no streamed output) the
/// underlying registry call is a no-op because it looks the id up with
/// `get_mut`. Without this finalize the reconcile pass later marks the row
/// "stale: owning process exited", producing a phantom Failed entry per
/// sub-agent. Cancellation is recorded as `Cancelled`, not `Failed`, so a user
/// Ctrl+C isn't misreported as a crash.
fn finalize_agent_row(agent_id: &str, outcome: AgentRowOutcome<'_>) {
    let (status, message) = match outcome {
        AgentRowOutcome::Completed => (
            jfc_daemon::BackgroundAgentStatus::Completed,
            "workflow agent completed",
        ),
        AgentRowOutcome::Cancelled => (
            jfc_daemon::BackgroundAgentStatus::Cancelled,
            "workflow cancelled",
        ),
        AgentRowOutcome::Failed(err) => (jfc_daemon::BackgroundAgentStatus::Failed, err),
    };
    jfc_daemon::record_background_agent_finished(agent_id, status, message);
}

/// Map a sub-agent error string + cancellation flag to a row outcome.
fn agent_error_outcome(cancelled: bool, error: &str) -> AgentRowOutcome<'_> {
    if cancelled {
        AgentRowOutcome::Cancelled
    } else {
        AgentRowOutcome::Failed(error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::AtomicUsize;

    #[test]
    fn max_concurrency_is_bounded_normal() {
        let mc = max_concurrency();
        assert!(mc >= 2);
        assert!(mc <= 16);
    }

    /// A provider that returns a fixed text for every stream call. Each agent
    /// dispatch ends after one `Done(EndTurn)`, so the subagent loop returns
    /// the emitted text as the agent's result.
    struct EchoProvider {
        text: String,
        calls: AtomicUsize,
    }

    #[async_trait::async_trait]
    impl jfc_provider::Provider for EchoProvider {
        fn name(&self) -> &str {
            "anthropic"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            use futures::stream;
            self.calls.fetch_add(1, Ordering::SeqCst);
            let events = vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: self.text.clone(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ];
            Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
        }
    }
    impl jfc_provider::seal::Sealed for EchoProvider {}

    struct SequenceProvider {
        streams: Mutex<VecDeque<Vec<jfc_provider::StreamEvent>>>,
    }

    #[async_trait::async_trait]
    impl jfc_provider::Provider for SequenceProvider {
        fn name(&self) -> &str {
            "anthropic"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            use futures::stream;
            let events = self
                .streams
                .lock()
                .expect("sequence provider mutex poisoned")
                .pop_front()
                .unwrap_or_else(|| {
                    vec![jfc_provider::StreamEvent::Done {
                        stop_reason: jfc_provider::StopReason::EndTurn,
                    }]
                });
            Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
        }
    }
    impl jfc_provider::seal::Sealed for SequenceProvider {}

    fn cfg_with_provider(
        script: &str,
        dir: &std::path::Path,
        provider: Arc<dyn jfc_provider::Provider>,
    ) -> WorkflowRunConfig {
        cfg_with_provider_and_args(script, dir, serde_json::Value::Null, provider)
    }

    fn cfg_with_provider_and_args(
        script: &str,
        dir: &std::path::Path,
        args: serde_json::Value,
        provider: Arc<dyn jfc_provider::Provider>,
    ) -> WorkflowRunConfig {
        WorkflowRunConfig {
            run_id: "wf_test01".into(),
            script_body: script.into(),
            args,
            provider,
            model: jfc_provider::ModelId::new("claude-opus-4-7"),
            session_id: None,
            session_dir: dir.to_path_buf(),
            resume_from_run_id: None,
            cancel: CancellationToken::new(),
            tx: None,
            workflow_task_id: "bgwf_1".into(),
            depth: 0,
            cwd: dir.to_path_buf(),
            token_budget: None,
        }
    }

    fn cfg(script: &str, dir: &std::path::Path) -> WorkflowRunConfig {
        cfg_with_provider(
            script,
            dir,
            Arc::new(EchoProvider {
                text: "AGENT_OUTPUT".into(),
                calls: AtomicUsize::new(0),
            }),
        )
    }

    fn workflow_task_input(isolation: Option<&str>) -> crate::types::TaskInput {
        crate::types::TaskInput {
            description: "inspect".into(),
            prompt: "inspect".into(),
            subagent_type: Some("reviewer".into()),
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

    fn workflow_agent_def(isolation: Option<&str>) -> crate::agents::AgentDef {
        crate::agents::AgentDef {
            name: "reviewer".into(),
            source: std::path::PathBuf::from("test"),
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
            hooks: std::collections::HashMap::new(),
            key_trigger: None,
            use_when: Vec::new(),
            avoid_when: Vec::new(),
            cost: None,
            system_prompt: String::new(),
        }
    }

    #[test]
    fn workflow_agent_isolation_defaults_preserve_precedence_normal() {
        let mut task = workflow_task_input(None);
        let agent = workflow_agent_def(Some("worktree"));

        apply_workflow_agent_isolation_defaults(&mut task, Some(&agent), Some("other"));

        assert_eq!(task.isolation.as_deref(), Some("worktree"));

        let mut task = workflow_task_input(None);
        apply_workflow_agent_isolation_defaults(&mut task, None, Some(" worktree "));
        assert_eq!(task.isolation.as_deref(), Some("worktree"));

        let mut task = workflow_task_input(Some("explicit"));
        let agent = workflow_agent_def(Some("worktree"));
        apply_workflow_agent_isolation_defaults(&mut task, Some(&agent), Some("other"));
        assert_eq!(task.isolation.as_deref(), Some("explicit"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_workflow_agent_tasks_abort_after_grace_regression() {
        let mut tasks = tokio::task::JoinSet::new();
        tasks.spawn(async {
            std::future::pending::<()>().await;
        });

        let aborted =
            drain_agent_tasks_with_grace(&mut tasks, true, std::time::Duration::from_millis(10))
                .await;

        assert!(aborted, "cancelled drain must abort stuck agent tasks");
        assert_eq!(tasks.len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn workflow_cancel_rejects_queued_agent_requests_regression() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config = cfg("return await agent('cancel me');", tmp.path());
        config.cancel.cancel();

        let out = tokio::time::timeout(std::time::Duration::from_secs(2), run_workflow(config))
            .await
            .expect("cancelled workflow must not hang waiting for engine thread");

        assert!(
            out.logs.iter().any(|line| line == "workflow cancelled"),
            "logs: {:?}",
            out.logs
        );
        assert!(
            out.error.is_some(),
            "cancelled workflow should surface an error"
        );
        // The distinguishing signal for callers (auto-review): the run is
        // flagged `cancelled`, so the orchestrator-teardown error is NOT
        // misreported as a genuine failure.
        assert!(
            out.cancelled,
            "cancelled workflow must set the cancelled flag"
        );
    }

    // A single agent() call flows engine → orchestrator → execute_task →
    // EchoProvider and the result returns to the script.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_workflow_dispatches_agent_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = run_workflow(cfg("return await agent('do a thing');", tmp.path())).await;
        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result, serde_json::json!("AGENT_OUTPUT"));
        assert_eq!(out.total_agents_dispatched, 1);
        assert_eq!(out.cache_hits, 0);
        // A normal run is never flagged cancelled.
        assert!(!out.cancelled, "successful workflow must not be cancelled");
    }

    // parallel() dispatches multiple agents through the orchestrator.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn run_workflow_parallel_dispatch_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let out = run_workflow(cfg(
            "return await parallel([() => agent('a'), () => agent('b')]);",
            tmp.path(),
        ))
        .await;
        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(
            out.result,
            serde_json::json!(["AGENT_OUTPUT", "AGENT_OUTPUT"])
        );
        assert_eq!(out.total_agents_dispatched, 2);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn workflow_agent_schema_requires_structured_output_regression() {
        let tmp = tempfile::TempDir::new().unwrap();
        let script = r#"
            return await agent('return a review candidate', {
                schema: {
                    type: 'object',
                    properties: { summary: { type: 'string' } },
                    required: ['summary'],
                    additionalProperties: false
                }
            });
        "#;
        let out = run_workflow(cfg_with_provider(
            script,
            tmp.path(),
            Arc::new(EchoProvider {
                text: r#"{"summary":"plain text json is not enough"}"#.into(),
                calls: AtomicUsize::new(0),
            }),
        ))
        .await;

        assert!(out.error.is_some());
        assert!(
            out.error
                .unwrap()
                .contains("without calling StructuredOutput")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn workflow_agent_schema_returns_validated_structured_output_regression() {
        let tmp = tempfile::TempDir::new().unwrap();
        let script = r#"
            return await agent('return a review candidate', {
                schema: {
                    type: 'object',
                    properties: { summary: { type: 'string' } },
                    required: ['summary'],
                    additionalProperties: false
                }
            });
        "#;
        let streams = VecDeque::from([
            vec![
                jfc_provider::StreamEvent::ToolDone {
                    index: 0,
                    tool_name: "StructuredOutput".into(),
                    tool_use_id: "toolu_1".into(),
                    input_json: r#"{"summary":"validated"}"#.into(),
                    thought_signature: None,
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::ToolUse,
                },
            ],
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "done".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
        ]);

        let out = run_workflow(cfg_with_provider(
            script,
            tmp.path(),
            Arc::new(SequenceProvider {
                streams: Mutex::new(streams),
            }),
        ))
        .await;

        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result, serde_json::json!(r#"{"summary":"validated"}"#));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn workflow_schema_agent_invalid_json_surfaces_error_regression() {
        // A schema agent that calls StructuredOutput but whose result text is
        // somehow not valid JSON must FAIL loudly, not be silently stringified.
        // We simulate this with a StructuredOutput tool whose input_json is
        // not parseable, which the validator lets through to the runner's
        // post-parse step.
        let tmp = tempfile::TempDir::new().unwrap();
        let script = r#"
            return await agent('return a candidate', {
                schema: {
                    type: 'object',
                    properties: { summary: { type: 'string' } },
                    required: ['summary'],
                    additionalProperties: false
                }
            });
        "#;
        // EchoProvider returns plain text (no StructuredOutput call). The
        // schema path already rejects this with the "without calling
        // StructuredOutput" nudge — asserting the schema contract is enforced
        // end to end (the runner never silently accepts non-JSON under schema).
        let out = run_workflow(cfg_with_provider(
            script,
            tmp.path(),
            Arc::new(EchoProvider {
                text: "this is not json at all".into(),
                calls: AtomicUsize::new(0),
            }),
        ))
        .await;
        assert!(
            out.error.is_some(),
            "schema violation must surface an error"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn builtin_code_review_low_effort_smoke_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wf = crate::workflows::resolve(tmp.path(), "code-review").unwrap();
        let (_meta, body) = crate::workflows::parse_meta(&wf.script).unwrap();
        let streams = VecDeque::from([
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "scope notes".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
            vec![
                jfc_provider::StreamEvent::ToolDone {
                    index: 0,
                    tool_name: "StructuredOutput".into(),
                    tool_use_id: "toolu_find".into(),
                    input_json: r#"{"candidates":[{"file":"src/lib.rs","line":10,"summary":"bug summary","severity":"high","category":"logic","evidence":"specific evidence","confidence":0.91}]}"#.into(),
                    thought_signature: None,
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::ToolUse,
                },
            ],
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "found".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
            vec![
                jfc_provider::StreamEvent::ToolDone {
                    index: 0,
                    tool_name: "StructuredOutput".into(),
                    tool_use_id: "toolu_verify".into(),
                    input_json: r#"{"valid":true,"file":"src/lib.rs","line":10,"severity":"high","category":"logic","summary":"bug summary","evidence":"specific evidence","fix":"apply the targeted fix","confidence":0.91,"reason":"verified against the code"}"#.into(),
                    thought_signature: None,
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::ToolUse,
                },
            ],
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "verified".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "final report".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
        ]);

        let out = run_workflow(cfg_with_provider_and_args(
            &body,
            tmp.path(),
            serde_json::json!({ "level": "low", "target": "current diff" }),
            Arc::new(SequenceProvider {
                streams: Mutex::new(streams),
            }),
        ))
        .await;

        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result["level"], serde_json::json!("low"));
        assert_eq!(out.result["target"], serde_json::json!("current diff"));
        assert_eq!(
            out.result["final_report"],
            serde_json::json!("final report")
        );
        assert_eq!(out.result["findings"].as_array().unwrap().len(), 1);
        assert_eq!(out.result["dismissed"].as_array().unwrap().len(), 0);
        assert_eq!(out.result["diagnostics"].as_array().unwrap().len(), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn builtin_code_review_records_finder_failures_regression() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wf = crate::workflows::resolve(tmp.path(), "code-review").unwrap();
        let (_meta, body) = crate::workflows::parse_meta(&wf.script).unwrap();

        let out = run_workflow(cfg_with_provider_and_args(
            &body,
            tmp.path(),
            serde_json::json!({ "level": "low", "target": "current diff" }),
            Arc::new(EchoProvider {
                text: "plain text cannot satisfy schema".into(),
                calls: AtomicUsize::new(0),
            }),
        ))
        .await;

        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result["findings"].as_array().unwrap().len(), 0);
        assert_eq!(out.result["diagnostics"].as_array().unwrap().len(), 1);
        assert_eq!(
            out.result["diagnostics"][0]["stage"],
            serde_json::json!("Find")
        );
        assert_eq!(
            out.result["diagnostics"][0]["angle"],
            serde_json::json!("bugs")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn builtin_code_review_records_verifier_failures_regression() {
        let tmp = tempfile::TempDir::new().unwrap();
        let wf = crate::workflows::resolve(tmp.path(), "code-review").unwrap();
        let (_meta, body) = crate::workflows::parse_meta(&wf.script).unwrap();
        let streams = VecDeque::from([
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "scope notes".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
            vec![
                jfc_provider::StreamEvent::ToolDone {
                    index: 0,
                    tool_name: "StructuredOutput".into(),
                    tool_use_id: "toolu_find".into(),
                    input_json: r#"{"candidates":[{"file":"src/lib.rs","line":10,"summary":"bug summary","severity":"high","category":"logic","evidence":"specific evidence","confidence":0.91}]}"#.into(),
                    thought_signature: None,
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::ToolUse,
                },
            ],
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "found".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "plain verifier text cannot satisfy schema".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
            vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: "final report".into(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ],
        ]);

        let out = run_workflow(cfg_with_provider_and_args(
            &body,
            tmp.path(),
            serde_json::json!({ "level": "low", "target": "current diff" }),
            Arc::new(SequenceProvider {
                streams: Mutex::new(streams),
            }),
        ))
        .await;

        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result["findings"].as_array().unwrap().len(), 0);
        assert_eq!(out.result["dismissed"].as_array().unwrap().len(), 1);
        assert_eq!(
            out.result["dismissed"][0]["valid"],
            serde_json::json!(false)
        );
        assert_eq!(
            out.result["dismissed"][0]["file"],
            serde_json::json!("src/lib.rs")
        );
        assert_eq!(out.result["diagnostics"].as_array().unwrap().len(), 1);
        assert_eq!(
            out.result["diagnostics"][0]["stage"],
            serde_json::json!("Verify")
        );
    }

    // A second run resuming from the first replays cached results without
    // dispatching any new agents.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_workflow_resume_replays_cache_normal() {
        let tmp = tempfile::TempDir::new().unwrap();
        let script = "return await agent('cached work');";

        let first = run_workflow(cfg(script, tmp.path())).await;
        assert!(first.error.is_none());
        assert_eq!(first.total_agents_dispatched, 1);

        // Resume: same script + same args ⇒ 100% cache hit, 0 dispatched.
        let mut resume = cfg(script, tmp.path());
        resume.run_id = "wf_test02".into();
        resume.resume_from_run_id = Some("wf_test01".into());
        let second = run_workflow(resume).await;
        assert!(second.error.is_none(), "error: {:?}", second.error);
        assert_eq!(second.result, serde_json::json!("AGENT_OUTPUT"));
        assert_eq!(second.total_agents_dispatched, 0);
        assert_eq!(second.cache_hits, 1);
    }

    // Resume keys bind provider + effective model: a journal written by one
    // provider must NOT replay for a different provider that exposes the same
    // bare model id. Here the second run uses a provider with a different name
    // (and a model that emits different text), so the prior journal entry must
    // miss and the agent must re-run rather than serve a stale, cross-provider
    // answer.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn run_workflow_resume_key_isolates_providers_regression() {
        let tmp = tempfile::TempDir::new().unwrap();
        let script = "return await agent('shared-model work');";

        // First run: provider "anthropic" emits "FIRST_PROVIDER".
        let first_cfg = cfg_with_provider(
            script,
            tmp.path(),
            Arc::new(EchoProvider {
                text: "FIRST_PROVIDER".into(),
                calls: AtomicUsize::new(0),
            }),
        );
        let first = run_workflow(first_cfg).await;
        assert!(first.error.is_none());
        assert_eq!(first.total_agents_dispatched, 1);

        // Resume with a DIFFERENT provider (name "openai") emitting different
        // text. Same script/args/model id, but the provider differs, so the
        // resume key must not match the prior journal entry.
        let mut resume = cfg_with_provider(
            script,
            tmp.path(),
            Arc::new(OtherEchoProvider {
                text: "SECOND_PROVIDER".into(),
            }),
        );
        resume.run_id = "wf_test02".into();
        resume.resume_from_run_id = Some("wf_test01".into());
        let second = run_workflow(resume).await;
        assert!(second.error.is_none(), "error: {:?}", second.error);
        // The cross-provider entry missed: the agent re-ran and produced the
        // new provider's output, with zero cache hits.
        assert_eq!(second.result, serde_json::json!("SECOND_PROVIDER"));
        assert_eq!(second.total_agents_dispatched, 1);
        assert_eq!(second.cache_hits, 0);
    }

    /// Echo provider that reports a different provider name ("openai") so the
    /// resume-key provider-isolation test can prove two providers sharing a
    /// model id don't collide in the cache.
    struct OtherEchoProvider {
        text: String,
    }

    #[async_trait::async_trait]
    impl jfc_provider::Provider for OtherEchoProvider {
        fn name(&self) -> &str {
            "openai"
        }
        fn available_models(&self) -> Vec<jfc_provider::ModelInfo> {
            vec![]
        }
        async fn stream(
            &self,
            _messages: Vec<jfc_provider::ProviderMessage>,
            _options: &jfc_provider::StreamOptions,
        ) -> anyhow::Result<jfc_provider::EventStream> {
            use futures::stream;
            let events = vec![
                jfc_provider::StreamEvent::TextDelta {
                    index: 0,
                    delta: self.text.clone(),
                },
                jfc_provider::StreamEvent::Done {
                    stop_reason: jfc_provider::StopReason::EndTurn,
                },
            ];
            Ok(Box::pin(stream::iter(events.into_iter().map(Ok))))
        }
    }
    impl jfc_provider::seal::Sealed for OtherEchoProvider {}
}
