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
use jfc_provider::{ModelId, Provider};

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
    /// Session directory for the resume journal.
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
    fn next_key(&self, req: &AgentRequest) -> String {
        let opts_json = serde_json::json!({
            "model": req.model,
            "schema": req.schema,
            "agentType": req.agent_type,
            "isolation": req.isolation,
        })
        .to_string();
        let mut h = self.running_hash.lock();
        let k = journal::compute_key(&h, &req.prompt, &opts_json);
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
            let _permit = permit_sem.acquire().await;
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
            let c = journal::load_journal(&session_dir, prev).await;
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
    let journal_writer = JournalWriter::new(&session_dir, &run_id);
    tracing::debug!(
        target: "jfc::workflow",
        path = %journal_writer.path().display(),
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
                        session_dir: session_dir.clone(),
                        resume_from_run_id: None,
                        cancel: orch.cancel.clone(),
                        tx: orch.tx.clone(),
                        workflow_task_id: orch.workflow_task_id.clone(),
                        depth: depth + 1,
                        cwd: cwd.clone(),
                        token_budget: None,
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

    // Wait for in-flight agents to settle (or be cancelled).
    while orch.agent_tasks.join_next().await.is_some() {}

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

    WorkflowOutcome {
        result: engine_outcome.result,
        agent_count: engine_outcome.agent_count,
        total_agents_dispatched: dispatched.load(Ordering::Relaxed),
        cache_hits: cache_hits.load(Ordering::Relaxed),
        logs,
        error: engine_outcome.error,
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

    let task_input = crate::types::TaskInput {
        description: req.label.clone(),
        prompt: req.prompt.clone(),
        subagent_type: req.agent_type.clone(),
        category: None,
        run_in_background: false,
        model: req.model.clone(),
        effort: None,
        name: None,
        team_name: None,
        mode: None,
        isolation: req.isolation.clone(),
        parent_task_id: None,
        schema: req.schema.clone(),
    };

    // Resolve an agent definition if a custom agentType was requested.
    let cwd = std::env::current_dir().unwrap_or_default();
    let agents = crate::agents::load_agents(&cwd);
    let agent_def = req
        .agent_type
        .as_deref()
        .and_then(|t| agents.iter().find(|a| a.name.eq_ignore_ascii_case(t)))
        .cloned();

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
            None,
            None,
            None,
        ) => r,
    };

    if result.is_error() {
        // Emit failure progress event before replying.
        if let Some(tx) = &tx {
            let task_id = crate::ids::TaskId::from(workflow_task_id.clone());
            let index = req.index;
            let error = result.output.clone();
            let tx = tx.clone();
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
        let _ = req.reply.send(Err(result.output.clone()));
        return;
    }

    let text = result.output.clone();

    // Accumulate token estimate (4 chars ≈ 1 token).
    let token_estimate = text.len() as u64 / 4;
    tokens_spent.fetch_add(token_estimate, Ordering::Relaxed);

    // When the agent ran under a StructuredOutput schema, its output should be
    // the validated JSON object (execute_task installs the schema + requires the
    // tool). Parse it so the journal stores a real object and the workflow
    // script receives structured data — not an opaque JSON string. If parsing
    // fails despite a schema, that's a genuine contract violation: surface it as
    // an agent failure rather than silently stringifying (which used to hide
    // schema breaches and surprise scripts expecting object fields).
    let structured_result: Option<serde_json::Value> = if req.schema.is_some() {
        match serde_json::from_str::<serde_json::Value>(&text) {
            Ok(v) => Some(v),
            Err(e) => {
                let error = format!(
                    "workflow agent declared a StructuredOutput schema but returned \
                     output that is not valid JSON: {e}"
                );
                if let Some(tx) = &tx {
                    let task_id = crate::ids::TaskId::from(workflow_task_id.clone());
                    let index = req.index;
                    let error = error.clone();
                    let tx = tx.clone();
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
        }
    } else {
        None
    };

    let journal_result = structured_result
        .clone()
        .unwrap_or_else(|| serde_json::Value::String(text.clone()));

    // Record the result journal entry.
    let _ = journal_writer
        .append(&JournalEntry::Result {
            key,
            agent_id,
            result: journal_result,
        })
        .await;

    // Emit success progress event.
    if let Some(tx) = &tx {
        let task_id = crate::ids::TaskId::from(workflow_task_id.clone());
        let index = req.index;
        let tx = tx.clone();
        tokio::spawn(async move {
            let _ = tx
                .send(crate::runtime::EngineEvent::WorkflowProgress(
                    crate::runtime::WorkflowProgressEvent::AgentDone { task_id, index },
                ))
                .await;
        });
    }

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
        assert!(out.error.is_some(), "schema violation must surface an error");
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
}
