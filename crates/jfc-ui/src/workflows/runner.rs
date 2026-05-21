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
use std::sync::atomic::{AtomicU32, Ordering};

use tokio::sync::{Semaphore, mpsc};
use tokio_util::sync::CancellationToken;

use super::engine::{AgentRequest, ProgressSignal, run_script};
use super::journal::{self, JournalCache, JournalEntry, JournalWriter};
use jfc_provider::{ModelId, Provider};

/// Max concurrent agent() calls = min(16, cpus - 2), floor 2.
pub fn max_concurrency() -> usize {
    let cpus = num_cpus::get();
    std::cmp::min(16, std::cmp::max(2, cpus.saturating_sub(2)))
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
    pub tx: Option<mpsc::Sender<crate::runtime::AppEvent>>,
    /// The workflow's own background task id (for progress routing).
    pub workflow_task_id: String,
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
    tx: Option<mpsc::Sender<crate::runtime::AppEvent>>,
    workflow_task_id: String,
    cancel: CancellationToken,
    agent_tasks: tokio::task::JoinSet<()>,
    logs: Vec<String>,
}

impl Orchestrator {
    fn record_progress(&mut self, sig: ProgressSignal) {
        match sig {
            ProgressSignal::Phase(title) => self.logs.push(format!("phase: {title}")),
            ProgressSignal::Log(msg) => self.logs.push(msg),
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
            let text = cached
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| cached.to_string());
            let _ = req.reply.send(Ok(text));
            return;
        }

        self.dispatched.fetch_add(1, Ordering::Relaxed);

        let permit_sem = self.semaphore.clone();
        let provider = self.provider.clone();
        let model = self.model.clone();
        let tx = self.tx.clone();
        let journal_writer = self.journal_writer.clone();
        let workflow_task_id = self.workflow_task_id.clone();
        let cancel = self.cancel.clone();
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
    } = config;

    // Resume cache: load the prior run's journal if requested.
    let cache: Option<JournalCache> = match &resume_from_run_id {
        Some(prev) => Some(journal::load_journal(&session_dir, prev).await),
        None => None,
    };
    let journal_writer = JournalWriter::new(&session_dir, &run_id);

    // Channels bridging the engine thread to this orchestrator.
    let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentRequest>();
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel::<ProgressSignal>();

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
            run_script(&engine_body, engine_args, agent_tx, progress_tx).await
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
    };

    // Drive both channels until the engine thread finishes.
    loop {
        tokio::select! {
            biased;

            _ = cancel.cancelled() => {
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

/// Dispatch one agent through `execute_task`, write the journal, and reply.
#[allow(clippy::too_many_arguments)]
async fn run_one_agent(
    req: AgentRequest,
    key: String,
    provider: Arc<dyn Provider>,
    model: ModelId,
    tx: Option<mpsc::Sender<crate::runtime::AppEvent>>,
    journal_writer: Arc<JournalWriter>,
    workflow_task_id: String,
    cancel: CancellationToken,
) {
    let agent_id = format!("{workflow_task_id}:agent_{}", req.index);

    // Record the started journal entry.
    let _ = journal_writer
        .append(&JournalEntry::Started {
            key: key.clone(),
            agent_id: agent_id.clone(),
        })
        .await;

    // Build the subagent prompt. When a schema was requested, append a
    // structured-output instruction so the agent returns parseable JSON.
    let prompt = match &req.schema {
        Some(schema) => format!(
            "{}\n\n---\nReturn ONLY a JSON value matching this schema (no prose, no code fences):\n{}",
            req.prompt, schema
        ),
        None => req.prompt.clone(),
    };

    let task_input = crate::types::TaskInput {
        description: req.label.clone(),
        prompt,
        subagent_type: req.agent_type.clone(),
        category: None,
        run_in_background: false,
        model: req.model.clone(),
        name: None,
        team_name: None,
        mode: None,
        isolation: req.isolation.clone(),
        parent_task_id: None,
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
        let _ = req.reply.send(Err(result.output.clone()));
        return;
    }

    let text = result.output.clone();

    // Record the result journal entry (store as a JSON string).
    let _ = journal_writer
        .append(&JournalEntry::Result {
            key,
            agent_id,
            result: serde_json::Value::String(text.clone()),
        })
        .await;

    let _ = req.reply.send(Ok(text));
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

    fn cfg(script: &str, dir: &std::path::Path) -> WorkflowRunConfig {
        WorkflowRunConfig {
            run_id: "wf_test01".into(),
            script_body: script.into(),
            args: serde_json::Value::Null,
            provider: Arc::new(EchoProvider {
                text: "AGENT_OUTPUT".into(),
                calls: AtomicUsize::new(0),
            }),
            model: jfc_provider::ModelId::new("claude-opus-4-7"),
            session_dir: dir.to_path_buf(),
            resume_from_run_id: None,
            cancel: CancellationToken::new(),
            tx: None,
            workflow_task_id: "bgwf_1".into(),
        }
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
        assert_eq!(out.result, serde_json::json!(["AGENT_OUTPUT", "AGENT_OUTPUT"]));
        assert_eq!(out.total_agents_dispatched, 2);
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
