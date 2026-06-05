//! t213 — jfc-plan end-to-end cross-session test.
//!
//! Walks a project tempdir through three "sessions" (each session is a
//! fresh `PlanStore` opened against the same directory, simulating a new
//! process). Asserts plans survive across session boundaries, plan
//! advancement persists, and PlanDreamer + PlanRecall can operate on the
//! reloaded store.
//!
//! All LLM-facing pieces use an in-process `MockProvider` that returns
//! deterministic canned responses for the select / synthesize passes, so
//! the test runs hermetically in CI without network access.

use std::path::Path;
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use jfc_provider::{
    CompletionResponse, EventStream, ModelId, ModelInfo, Provider, ProviderContent,
    ProviderMessage, StreamConvention, StreamOptions, TokenUsage,
};
use jfc_session::{TaskPatch, TaskStatus, TaskStore};
use jfc_ui::plan::{PlanStatus, PlanStore};
use jfc_ui::plan_dreamer::PlanDreamer;
use jfc_ui::plan_recall;
use serde_json::{Value, json};
use tempfile::TempDir;

// ─── Mock provider ──────────────────────────────────────────────────────────

/// Provider stub that returns a sequence of canned tool-call responses.
/// Tracks every call so tests can assert on prompt content.
struct MockProvider {
    responses: StdMutex<Vec<String>>,
    calls: StdMutex<Vec<String>>,
}

impl MockProvider {
    fn with_responses<I: IntoIterator<Item = String>>(items: I) -> Arc<Self> {
        Arc::new(Self {
            responses: StdMutex::new(items.into_iter().collect()),
            calls: StdMutex::new(Vec::new()),
        })
    }

    fn call_count(&self) -> usize {
        self.calls.lock().unwrap().len()
    }
}

impl jfc_provider::seal::Sealed for MockProvider {}

#[async_trait]
impl Provider for MockProvider {
    fn name(&self) -> &str {
        "mock-plan-recall"
    }

    fn available_models(&self) -> Vec<ModelInfo> {
        Vec::new()
    }

    fn stream_convention(&self) -> StreamConvention {
        StreamConvention::AnthropicNative
    }

    async fn stream(
        &self,
        _: Vec<ProviderMessage>,
        _: &StreamOptions,
    ) -> anyhow::Result<EventStream> {
        anyhow::bail!("MockProvider: stream not supported in plan_e2e tests");
    }

    async fn complete(
        &self,
        messages: Vec<ProviderMessage>,
        _: &StreamOptions,
    ) -> anyhow::Result<CompletionResponse> {
        // Record the last user message text so we can assert what was sent.
        let last = messages
            .last()
            .and_then(|m| m.content.first())
            .and_then(|c| match c {
                ProviderContent::Text(t) => Some(t.clone()),
                _ => None,
            })
            .unwrap_or_default();
        self.calls.lock().unwrap().push(last);

        // Pop the next canned response. If we've exhausted the queue, repeat
        // the last one — recall pipelines may call `complete` a variable
        // number of times depending on caching state.
        let mut q = self.responses.lock().unwrap();
        let next = if q.len() > 1 {
            q.remove(0)
        } else {
            q.first().cloned().unwrap_or_else(|| "{}".to_owned())
        };

        Ok(CompletionResponse {
            content: next,
            usage: TokenUsage::default(),
        })
    }
}

// ─── Helpers ────────────────────────────────────────────────────────────────

/// Open a PlanStore against the project tempdir as if a fresh process is
/// starting. `open_project(Some(root))` writes/reads under `<root>/.jfc/plans`.
fn open_store(project_root: &Path) -> Arc<PlanStore> {
    PlanStore::open_project(Some(project_root)).expect("open_project should succeed")
}

/// Complete a task and notify the store. Returns whether the plan flipped to
/// `Done` as a result.
fn complete_task_and_notify(
    store: &PlanStore,
    task_store: &TaskStore,
    task_id: &str,
    summary: &str,
) {
    task_store
        .update(
            task_id,
            TaskPatch {
                status: Some(TaskStatus::Completed),
                ..Default::default()
            },
        )
        .expect("task update should succeed");
    store
        .on_task_done(task_id, summary, task_store)
        .expect("on_task_done should succeed");
}

// ─── The test ───────────────────────────────────────────────────────────────

/// Cross-session plan persistence + dreamer + recall happy-path.
///
/// We run the recall block in a tokio runtime so the async provider trait
/// is satisfied. Everything else is synchronous.
#[test]
fn plan_e2e_cross_session_lifecycle() {
    // Hermetic project root that lives for the whole test.
    let tmp = TempDir::new().unwrap();
    let project_root = tmp.path();

    // ── Session 1 ───────────────────────────────────────────────────────
    {
        let store = open_store(project_root);
        let task_store = TaskStore::in_memory();

        store
            .create(
                "Design Auth Flow",
                "Initial design for the new authentication subsystem.\n\n\
                 ## TODOs\n\
                 - [ ] 1. Pick token format\n\
                 - [ ] 2. Wire up session store\n\
                 - [ ] 3. Document threat model\n",
            )
            .expect("create plan");

        // Bump to Active so it's the kind PlanRecall prefers.
        store
            .update(
                "design-auth-flow",
                jfc_ui::plan::PlanPatch {
                    status: Some(PlanStatus::Active),
                    ..Default::default()
                },
            )
            .expect("activate plan");

        // Materialize three tasks.
        let ids = store
            .materialize_tasks("design-auth-flow", &task_store)
            .expect("materialize");
        assert_eq!(ids.len(), 3, "expected 3 tasks materialized, got {:?}", ids);

        let after_materialize = store.get("design-auth-flow").unwrap();
        let last_advanced_before = after_materialize.frontmatter.last_advanced.clone();

        // Complete the first task → plan must advance but stay Active.
        complete_task_and_notify(&store, &task_store, &ids[0], "Picked JWT");

        let after_one = store.get("design-auth-flow").unwrap();
        assert!(
            after_one.body.contains("Picked JWT"),
            "progress log should record 'Picked JWT', body was: {}",
            after_one.body
        );
        assert!(
            after_one.frontmatter.last_advanced.is_some()
                && after_one.frontmatter.last_advanced != last_advanced_before,
            "last_advanced must move forward after first task completion"
        );
        assert_ne!(
            after_one.frontmatter.status,
            PlanStatus::Done,
            "plan must remain Active while two tasks still pending"
        );

        // Run a dreamer cycle. PlanDreamer is currently lease-only + stubs for
        // the LLM-driven passes, so a no-LLM cycle is enough to prove it runs
        // against a real on-disk store without panicking.
        let dreamer = PlanDreamer::new(store.clone());
        let report = dreamer.run_cycle().expect("dreamer cycle");
        assert!(
            !report.tasks_run.is_empty(),
            "dreamer should have run at least one task"
        );
        // The lease file must be removed on the way out.
        assert!(
            !project_root.join(".jfc/plans/.dreamer.lock").exists(),
            "dreamer lock must be released after cycle"
        );
    }

    // ── Session 2: simulate a fresh process ─────────────────────────────
    let (last_advanced_s1, ids_after_reload) = {
        let store = open_store(project_root);

        let plans = store.list(None);
        assert_eq!(
            plans.len(),
            1,
            "expected exactly one persisted plan, got {}",
            plans.len()
        );
        let plan = store.get("design-auth-flow").expect("plan reloaded");
        assert_eq!(plan.frontmatter.title, "Design Auth Flow");
        assert_eq!(plan.frontmatter.status, PlanStatus::Active);
        assert_eq!(
            plan.frontmatter.linked_task_ids.len(),
            3,
            "linked_task_ids must survive process boundary"
        );
        assert!(
            plan.frontmatter.last_advanced.is_some(),
            "last_advanced must survive process boundary"
        );
        assert!(
            plan.body.contains("Picked JWT"),
            "progress log entry from session 1 must persist; body was: {}",
            plan.body
        );

        let last_advanced_s1 = plan.frontmatter.last_advanced.clone();
        let ids = plan.frontmatter.linked_task_ids.clone();

        // Reconstitute a TaskStore that mirrors completion of task 1. We
        // can't share the session-1 TaskStore (in-memory), so we mark task 1
        // done in this fresh task store before notifying the plan store. The
        // tasks themselves don't carry plan-linkage metadata — the plan
        // store is the system of record — so we just need a task store with
        // matching IDs and the right statuses.
        let task_store = TaskStore::in_memory();
        // Re-create tasks with the *same IDs* via direct create — IDs are
        // allocated sequentially, so we just create three tasks in order
        // and trust the allocation. If that ever diverges from the original
        // IDs, the on_task_done lookup will simply find no matching plan,
        // which would assert below.
        let mut new_ids = Vec::with_capacity(3);
        for (i, _) in ids.iter().enumerate() {
            let t = task_store
                .create(
                    format!("placeholder-task-{i}"),
                    String::new(),
                    None,
                    Vec::<String>::new(),
                )
                .expect("create placeholder task");
            new_ids.push(t.id.to_string());
        }
        // Mark the first placeholder as already completed — we don't call
        // on_task_done for it again (it was advanced in session 1).
        task_store
            .update(
                &new_ids[0],
                TaskPatch {
                    status: Some(TaskStatus::Completed),
                    ..Default::default()
                },
            )
            .expect("pre-complete task 0");

        // Update linked_task_ids on the plan to reference these new IDs, so
        // on_task_done finds the plan.
        store
            .update(
                "design-auth-flow",
                jfc_ui::plan::PlanPatch {
                    linked_task_ids: Some(new_ids.clone()),
                    ..Default::default()
                },
            )
            .expect("rewire linked ids");

        // Complete the remaining two tasks one by one — plan must flip to
        // Done only after the *last* one.
        complete_task_and_notify(&store, &task_store, &new_ids[1], "Session store wired");
        let mid = store.get("design-auth-flow").unwrap();
        assert_ne!(
            mid.frontmatter.status,
            PlanStatus::Done,
            "plan must NOT be Done after only 2/3 tasks complete (task 0 was pre-completed in this task store and the plan checks task-store state)"
        );

        complete_task_and_notify(&store, &task_store, &new_ids[2], "Threat model written");

        let after = store.get("design-auth-flow").unwrap();
        assert_eq!(
            after.frontmatter.status,
            PlanStatus::Done,
            "plan must flip to Done once every linked task is completed"
        );
        assert!(after.body.contains("Session store wired"));
        assert!(after.body.contains("Threat model written"));

        (last_advanced_s1, new_ids)
    };

    // ── Session 3: verify no regressions after another reopen ───────────
    {
        let store = open_store(project_root);
        let plans = store.list(None);
        assert_eq!(plans.len(), 1, "still exactly one plan after final reopen");
        let plan = store.get("design-auth-flow").unwrap();
        assert_eq!(
            plan.frontmatter.status,
            PlanStatus::Done,
            "plan must still be Done after session 3 reopen"
        );
        // last_advanced must have moved forward since session 1.
        assert!(
            plan.frontmatter.last_advanced.is_some()
                && plan.frontmatter.last_advanced != last_advanced_s1,
            "last_advanced must reflect session-2 advancements"
        );
        // Sanity: progress log carries every advancement.
        for needle in ["Picked JWT", "Session store wired", "Threat model written"] {
            assert!(
                plan.body.contains(needle),
                "progress log must contain '{needle}', got body: {}",
                plan.body
            );
        }
        // And the task IDs we wired in session 2 are still there.
        assert_eq!(
            plan.frontmatter.linked_task_ids, ids_after_reload,
            "linked task ids must round-trip across sessions"
        );
    }

    // ── PlanRecall: prove a query against the persisted store returns the
    //    plan's content via the two-phase select → synthesize pipeline. ──
    {
        let store = open_store(project_root);
        let plans = store.list(None);
        assert!(!plans.is_empty(), "recall needs a persisted plan");

        // Phase 1 (select) returns the plan slug; phase 2 (synthesize)
        // returns one context item that references the plan.
        let select_resp: Value = json!({
            "selected_plans": ["design-auth-flow"]
        });
        let synth_resp: Value = json!({
            "context_items": [{
                "context": "Auth flow design is Done — JWT chosen, sessions wired, threat model documented.",
                "plan_slug": "design-auth-flow"
            }]
        });
        let provider =
            MockProvider::with_responses([select_resp.to_string(), synth_resp.to_string()]);

        // run_plan_recall has a process-wide cache; clear it so prior tests
        // (or a previous invocation of this one) don't poison the result.
        plan_recall::clear_cache();

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let block_opt = rt.block_on(plan_recall::run_plan_recall(
            "auth flow design",
            &plans,
            provider.clone() as Arc<dyn Provider>,
            ModelId::new("mock-model"),
        ));

        let block =
            block_opt.expect("recall pipeline must return a block when both phases respond");
        assert!(
            block.contains("<system-reminder>"),
            "recall block must wrap output in <system-reminder>, got: {block}"
        );
        assert!(
            block.contains("design-auth-flow"),
            "recall block must mention the plan slug, got: {block}"
        );
        assert!(
            block.contains("Auth flow design"),
            "recall block must surface the synthesized context, got: {block}"
        );
        assert_eq!(
            provider.call_count(),
            2,
            "expected exactly 2 provider calls (select + synthesize)"
        );
    }
}
