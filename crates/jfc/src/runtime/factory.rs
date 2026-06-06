use crate::runtime::{ControlEvent, EngineEvent, EventSender};
use jfc_session::{TaskPatch, TaskRisk, TaskStatus};
use crate::app::EngineState;

pub(crate) fn factory_mode_enabled() -> bool {
    !matches!(
        std::env::var("JFC_FACTORY_MODE").as_deref(),
        Ok("0" | "false" | "off" | "no")
    )
}

/// Mark the leader as having committed to a factory turn, then enqueue the
/// prompt. Setting `turn_started_at` *before* the `Submit` lands closes a
/// same-burst re-entry race: the event loop drains a burst of events into a
/// snapshot vec and processes them serially, and several of them (`Tick`,
/// `TaskFailed`, `AllComplete`, compaction `Done`) each call this factory.
/// The `Submit` we send here lands in a *later* burst, so without this flag a
/// second factory-triggering event in the *same* burst would see an idle
/// leader and claim/submit a second task — two concurrent turns racing the
/// same conversation buffer (and, with the stuck-task reaper, a requeue of the
/// task we just claimed). `turn_started_at.is_some()` is the very first guard
/// in this function, so setting it here makes any same-burst re-entry a no-op.
/// It is cleared on the next genuine `stream_done`/failure like any other turn.
async fn commit_factory_turn(state: &mut EngineState, tx: &EventSender, prompt: String) {
    state.turn_started_at = Some(std::time::Instant::now());
    let _ = tx.send(EngineEvent::Control(ControlEvent::SubmitPrompt(prompt))).await;
}

pub(crate) async fn maybe_continue_task_factory(state: &mut EngineState, tx: &EventSender) {
    if !factory_mode_enabled()
        || state.is_streaming
        // A live user/agentic turn keeps `turn_started_at` set until it
        // genuinely concludes (stream_done clears it only on EndTurn with
        // nothing pending). The factory must not inject a new task prompt
        // while the leader is still mid-turn — between sub-streams of an
        // agentic loop, or while it still has tool results to process,
        // `is_streaming` briefly drops to false but the turn is NOT over.
        // Firing here would race a second concurrent turn into the same
        // conversation buffer (the "random task queue / partially committed"
        // symptom). Only inject when the leader is fully idle.
        || state.turn_started_at.is_some()
        || state.pending_approval.is_some()
        || !state.approval_queue.is_empty()
        || !state.pending_tool_calls.is_empty()
        || !state.queued_prompts.is_empty()
        || state
            .background_tasks
            .values()
            .any(|task| task.status.is_alive())
    {
        return;
    }

    // Reaper: we only reach here when the leader is fully idle — no live
    // agent, no active turn, nothing pending in any queue (all guarded above).
    // Any task still `in_progress` under the factory owner is therefore stuck:
    // a previous turn ended without marking it done (or a crash left the claim
    // dangling). Reset those to pending+unowned so the claim below can pick
    // them back up, instead of stalling forever and forcing a manual "continue"
    // nudge. Only factory-owned claims are touched — live subagent work uses a
    // different owner string and is never disturbed.
    let requeued = state.task_store.requeue_stuck("jfc-factory");
    if !requeued.is_empty() {
        tracing::info!(
            target: "jfc::tasks::factory",
            count = requeued.len(),
            "reaped stuck in_progress tasks back to pending"
        );
    }

    let counts = state.task_store.counts();
    // Plan-verify gate: for a non-trivial batch (≥3 pending) ask the leader to
    // sanity-check the DAG once before execution. Smaller batches (1-2 tasks)
    // fall straight through to `claim_next_available` below and auto-continue
    // without the verification round-trip.
    if counts.pending >= 3 && counts.in_progress == 0 && !state.plan_verified_this_batch {
        state.plan_verified_this_batch = true;
        // Run the task-graph validator and surface its *computed* findings —
        // dependency cycles (Tarjan), upstream-failed propagation, the ready
        // frontier, and parallelization opportunities — instead of asking the
        // leader to eyeball them.
        let validation = state.task_store.validate();
        let tasks = state.task_store.list_all();
        let pending: Vec<_> = tasks
            .iter()
            .filter(|task| task.status == TaskStatus::Pending)
            .collect();
        let task_list = pending
            .iter()
            .map(|task| {
                let mut line = format!(
                    "- {} (blocked_by: {:?}): {}",
                    task.id, task.blocked_by, task.subject
                );
                if let Some(ref risk) = task.risk {
                    line.push_str(&format!(" [risk: {risk:?}]"));
                }
                if let Some(ref criteria) = task.acceptance_criteria {
                    line.push_str(&format!(" | criteria: {criteria}"));
                }
                if let Some(ref kind) = task.kind {
                    line.push_str(&format!(" | kind: {kind:?}"));
                }
                line
            })
            .collect::<Vec<_>>()
            .join("\n");

        // Computed graph findings.
        let mut findings = String::new();
        if !validation.dependency_cycles.is_empty() {
            findings.push_str("\n\n⚠ DEPENDENCY CYCLES detected — break these before execution:\n");
            for cycle in &validation.dependency_cycles {
                let chain = cycle
                    .iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(" -> ");
                findings.push_str(&format!("  - {chain}\n"));
            }
        }
        if !validation.upstream_failed.is_empty() {
            findings.push_str(&format!(
                "\nBlocked by a failed/deleted upstream task (won't run until rerouted): {}\n",
                validation
                    .upstream_failed
                    .iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        if !validation.parallelization_opportunities.is_empty() {
            findings.push_str("\nParallelizable (share blockers, independent of each other):\n");
            for opp in &validation.parallelization_opportunities {
                findings.push_str(&format!("  - {opp}\n"));
            }
        }
        if !validation.ready.is_empty() {
            findings.push_str(&format!(
                "\nReady to start now (no incomplete blockers): {}\n",
                validation
                    .ready
                    .iter()
                    .map(|id| id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }

        // Plan reuse: surface a similar prior decomposition as advisory
        // context, then cache this batch's decomposition under its signature.
        let subjects: Vec<String> = pending.iter().map(|t| t.subject.clone()).collect();
        let signature = jfc_core::normalize_signature(&subjects.join(" "));
        let prior_note = state
            .plan_cache
            .get_similar(&signature, 0.6)
            .map(|p| {
                format!(
                    "\n\nA similar plan ran before ('{}') with steps: {}. \
                     Reuse what still applies.",
                    p.source_description,
                    p.steps.join("; ")
                )
            })
            .unwrap_or_default();
        state.plan_cache.insert(&subjects.join(" "), subjects.clone());

        let prompt = format!(
            "Before executing the task queue, verify this plan is sound:\n\n{task_list}\n{findings}{prior_note}\n\
             The cycle/blocker/parallelism findings above are computed by the task-graph validator — \
             trust them. Also check for: tasks too broad to finish in one agent turn, high-risk tasks \
             that need user review, and tasks missing acceptance criteria. \
             If the plan is good, say 'Plan verified' and I'll start execution. \
             If changes are needed, use TaskUpdate/TaskCreate/TaskDone to revise, then say 'Plan revised'."
        );
        commit_factory_turn(state, tx, prompt).await;
        return;
    }

    let Some(task) = state.task_store.claim_next_available("jfc-factory") else {
        return;
    };

    if matches!(task.risk, Some(TaskRisk::High)) {
        let _ = state.task_store.update(
            task.id.as_str(),
            TaskPatch {
                status: Some(TaskStatus::Pending),
                owner: None,
                ..Default::default()
            },
        );
        tracing::info!(
            target: "jfc::tasks::factory",
            task_id = %task.id,
            "high-risk task requires user approval; skipping auto-execution"
        );
        let prompt = format!(
            "Task `{}` ('{}') is marked high-risk. Please review and approve before I execute it.\n\
             Description: {}\n\
             Acceptance criteria: {}",
            task.id,
            task.subject,
            task.description,
            task.acceptance_criteria.as_deref().unwrap_or("(none)")
        );
        commit_factory_turn(state, tx, prompt).await;
        return;
    }

    let mut prompt = format!(
        "Continue the task queue. Work on task `{}`: {}\n\n{}",
        task.id, task.subject, task.description
    );
    if let Some(ref criteria) = task.acceptance_criteria {
        prompt.push_str(&format!("\n\nAcceptance criteria: {criteria}"));
    }
    if let Some(ref command) = task.verification_command {
        prompt.push_str(&format!("\nVerification command: `{command}`"));
    }
    // Per-task model/effort overrides: surface them so the leader can switch
    // model (`/model …`) or reasoning effort for this task before working it,
    // honoring the Task.effort > AgentDef.effort > global precedence used for
    // subagents. (Applied advisory-style: the leader owns the model switch.)
    if let Some(ref model) = task.model {
        prompt.push_str(&format!(
            "\nPreferred model for this task: `{model}` — switch with `/model {model}` if not already active."
        ));
    }
    if let Some(ref effort) = task.effort {
        prompt.push_str(&format!(
            "\nPreferred reasoning effort for this task: `{effort}`."
        ));
    }
    prompt.push_str(&format!(
        "\n\nWhen this task is done, update its task status before stopping. \
         If you delegate this work via the Task tool, pass `parent_task_id: \"{}\"` \
         so the runtime auto-marks the task in_progress/completed/failed as the \
         subagent runs - no separate TaskUpdate/TaskDone needed. \
         If more unblocked tasks remain, continue with the next one.",
        task.id
    ));
    tracing::info!(
        target: "jfc::tasks::factory",
        task_id = %task.id,
        subject = %task.subject,
        "auto-continuing next available task"
    );
    commit_factory_turn(state, tx, prompt).await;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use crate::app::App;

    use jfc_provider::{EventStream, ModelInfo, Provider, ProviderMessage, StreamOptions};
    use tokio::sync::mpsc;

    use super::*;
    use crate::runtime::EngineEvent;

    struct TestProvider;

    #[async_trait::async_trait]
    impl Provider for TestProvider {
        fn name(&self) -> &str {
            "test"
        }
        fn available_models(&self) -> Vec<ModelInfo> {
            Vec::new()
        }
        async fn stream(
            &self,
            _messages: Vec<ProviderMessage>,
            _options: &StreamOptions,
        ) -> anyhow::Result<EventStream> {
            Ok(Box::pin(futures::stream::empty()))
        }
    }
    impl jfc_provider::seal::Sealed for TestProvider {}

    fn factory_app() -> App {
        // SAFETY: tests are single-threaded per process here; we force factory
        // mode on so the gate doesn't depend on ambient env.
        unsafe {
            std::env::set_var("JFC_FACTORY_MODE", "1");
        }
        let mut app = App::new(Arc::new(TestProvider), "test-model");
        app.engine.task_store = jfc_session::TaskStore::in_memory();
        app
    }

    fn submit_count(rx: &mut mpsc::Receiver<EngineEvent>) -> usize {
        let mut n = 0;
        while let Ok(ev) = rx.try_recv() {
            if matches!(ev, EngineEvent::Control(ControlEvent::SubmitPrompt(_))) {
                n += 1;
            }
        }
        n
    }

    #[tokio::test]
    async fn single_pending_task_auto_continues_and_commits_turn() {
        let mut app = factory_app();
        app.engine.task_store
            .create(
                "only task".into(),
                "do it".into(),
                None,
                Vec::<String>::new(),
            )
            .unwrap();
        let (tx, mut rx) = mpsc::channel::<EngineEvent>(16);

        // Even a single pending task (below the plan-verify threshold of 3)
        // must auto-continue rather than stall.
        maybe_continue_task_factory(&mut app.engine, &tx).await;

        assert!(
            app.engine.turn_started_at.is_some(),
            "factory must mark the turn committed so same-burst re-entry no-ops"
        );
        assert_eq!(submit_count(&mut rx), 1, "exactly one Submit enqueued");
        let claimed = app.engine.task_store.get("t1").unwrap();
        assert_eq!(claimed.status, jfc_session::TaskStatus::InProgress);
    }

    #[tokio::test]
    async fn same_burst_reentry_is_noop_no_double_submit() {
        let mut app = factory_app();
        app.engine.task_store
            .create("a".into(), "x".into(), None, Vec::<String>::new())
            .unwrap();
        app.engine.task_store
            .create("b".into(), "y".into(), None, Vec::<String>::new())
            .unwrap();
        let (tx, mut rx) = mpsc::channel::<EngineEvent>(16);

        // Two factory-triggering events in one burst: the first claims+commits,
        // the second must bail on the `turn_started_at.is_some()` guard instead
        // of claiming a second task into a concurrent turn.
        maybe_continue_task_factory(&mut app.engine, &tx).await;
        maybe_continue_task_factory(&mut app.engine, &tx).await;

        assert_eq!(
            submit_count(&mut rx),
            1,
            "second same-burst call must not enqueue a second Submit"
        );
        let counts = app.engine.task_store.counts();
        assert_eq!(counts.in_progress, 1, "only one task claimed");
        assert_eq!(counts.pending, 1, "the other stays pending");
    }

    #[tokio::test]
    async fn reaper_requeues_stuck_then_reclaims_when_idle() {
        let mut app = factory_app();
        app.engine.task_store
            .create("stuck".into(), "x".into(), None, Vec::<String>::new())
            .unwrap();
        // Simulate a prior factory turn that claimed t1 but ended without
        // completing it: in_progress + owned by jfc-factory, yet the leader is
        // now idle (turn_started_at None, no live agents).
        app.engine.task_store
            .update(
                "t1",
                jfc_session::TaskPatch {
                    status: Some(jfc_session::TaskStatus::InProgress),
                    owner: Some("jfc-factory".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let (tx, mut rx) = mpsc::channel::<EngineEvent>(16);

        maybe_continue_task_factory(&mut app.engine, &tx).await;

        // The reaper resets the stuck task, then the claim re-picks it and
        // commits a fresh turn — instead of stalling forever.
        assert_eq!(submit_count(&mut rx), 1, "stuck task re-continued");
        assert!(app.engine.turn_started_at.is_some());
        let t1 = app.engine.task_store.get("t1").unwrap();
        assert_eq!(t1.status, jfc_session::TaskStatus::InProgress);
        assert_eq!(t1.owner.as_deref(), Some("jfc-factory"));
    }
}
