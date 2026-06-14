//! JS sandbox via boa_engine — executes workflow scripts with host-bound
//! agent()/parallel()/pipeline()/phase()/log() functions.
//!
//! ## Async model
//!
//! boa's `Context` and its promise futures are `!Send`, so the entire script
//! runs on a dedicated OS thread driving a tokio `LocalSet`. Host functions
//! that need to do real async work (`agent()`) return a `JsPromise` built from
//! a future via [`JsPromise::from_future`]. That future bridges to the
//! orchestrator running on the main tokio runtime: it sends an [`AgentRequest`]
//! over an mpsc channel and awaits a oneshot reply carrying the agent's text
//! (a `Send` `String`). The orchestrator owns the `!Send`-unfriendly provider
//! handles, so nothing `!Send` ever crosses the thread boundary.
//!
//! `parallel([...])` and `pipeline(items, ...stages)` are implemented in a JS
//! prelude prepended to the user script; they compose `agent()` promises with
//! `Promise.all`, so true concurrency falls out of multiple in-flight futures
//! all waiting on their oneshot replies while the orchestrator services them
//! through the shared semaphore.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use boa_engine::{
    Context, JsArgs, JsError, JsNativeError, JsResult, JsValue, NativeFunction, Source,
    job::{FutureJob, JobQueue, NativeJob},
    js_string,
    object::builtins::JsPromise,
    property::Attribute,
};
use tokio::sync::{mpsc, oneshot};

/// A request from the script's `workflow()` call to the orchestrator for
/// nested sub-workflow execution.
#[derive(Debug)]
pub struct SubWorkflowRequest {
    /// Name of the workflow to run (registry name).
    pub name: String,
    /// Args JSON to pass to the sub-workflow.
    pub args: serde_json::Value,
    /// Reply channel — the orchestrator sends the sub-workflow's result value
    /// (or an error string) back here.
    pub reply: oneshot::Sender<Result<serde_json::Value, String>>,
}

/// A request from the script's `agent()` call to the orchestrator.
#[derive(Debug)]
pub struct AgentRequest {
    /// Monotonic index assigned by the engine (1-based).
    pub index: u32,
    /// The agent prompt.
    pub prompt: String,
    /// Display label (defaults to a prompt prefix).
    pub label: String,
    /// Optional phase title this agent belongs to.
    pub phase: Option<String>,
    /// Optional model override.
    pub model: Option<String>,
    /// Optional structured-output JSON schema.
    pub schema: Option<serde_json::Value>,
    /// Optional custom agent type (subagent_type).
    pub agent_type: Option<String>,
    /// Optional isolation ("worktree").
    pub isolation: Option<String>,
    /// Reply channel — the orchestrator sends the agent's final text (or an
    /// error message) back here.
    pub reply: oneshot::Sender<Result<String, String>>,
}

/// A progress signal from the script to the orchestrator (phase()/log()).
#[derive(Debug, Clone)]
pub enum ProgressSignal {
    Phase(String),
    Log(String),
}

/// Outcome of running a workflow script.
#[derive(Debug)]
pub struct EngineOutcome {
    /// The script's return value, serialized to JSON.
    pub result: serde_json::Value,
    /// Total agent() calls dispatched.
    pub agent_count: u32,
    /// Any uncaught error from the script.
    pub error: Option<String>,
}

/// JS prelude: defines parallel(), pipeline(), and the phase/log/agent/workflow
/// wrappers on top of the native host bindings (`__agent`, `__phase`,
/// `__log`, `__workflow`). Kept minimal and deterministic.
const JS_PRELUDE: &str = r#"
globalThis.agent = function(prompt, opts) {
    return __agent(prompt, opts || {});
};
globalThis.workflow = function(name, args) {
    return __workflow(name, args || {});
};
globalThis.phase = function(title) {
    __phase(title);
    return title;
};
globalThis.log = function(message) {
    __log(String(message));
};
globalThis.parallel = function(thunks) {
    if (!Array.isArray(thunks)) {
        throw new TypeError("parallel() expects an array of thunks");
    }
    return Promise.all(thunks.map(function(t) {
        try {
            return Promise.resolve(t()).catch(function() { return null; });
        } catch (e) {
            return Promise.resolve(null);
        }
    }));
};
globalThis.pipeline = function(items) {
    var stages = Array.prototype.slice.call(arguments, 1);
    if (!Array.isArray(items)) {
        throw new TypeError("pipeline() expects an array as its first argument");
    }
    return Promise.all(items.map(function(item, index) {
        var chain = Promise.resolve(item);
        stages.forEach(function(stage) {
            chain = chain.then(function(prev) {
                if (prev === null) return null;
                return stage(prev, item, index);
            }).catch(function() { return null; });
        });
        return chain;
    }));
};
"#;

/// A custom job queue that drives both microtask `NativeJob`s and async
/// `FutureJob`s. Futures are polled via the surrounding `LocalSet`.
#[derive(Default)]
struct WorkflowJobQueue {
    jobs: RefCell<VecDeque<NativeJob>>,
    futures: RefCell<VecDeque<FutureJob>>,
}

impl JobQueue for WorkflowJobQueue {
    fn enqueue_promise_job(&self, job: NativeJob, _context: &mut Context) {
        self.jobs.borrow_mut().push_back(job);
    }

    fn enqueue_future_job(&self, future: FutureJob, _context: &mut Context) {
        self.futures.borrow_mut().push_back(future);
    }

    fn run_jobs(&self, context: &mut Context) {
        // Synchronous fallback: drain microtasks. Futures can't be driven
        // here (no executor), so this is only correct when there are none.
        loop {
            let job = self.jobs.borrow_mut().pop_front();
            match job {
                Some(j) => {
                    let _ = j.call(context);
                }
                None => break,
            }
        }
    }

    fn run_jobs_async<'a, 'ctx, 'fut>(
        &'a self,
        context: &'ctx mut Context,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'fut>>
    where
        'a: 'fut,
        'ctx: 'fut,
    {
        Box::pin(async move {
            loop {
                // Drain all ready microtask jobs first.
                loop {
                    let job = self.jobs.borrow_mut().pop_front();
                    match job {
                        Some(j) => {
                            let _ = j.call(context);
                        }
                        None => break,
                    }
                }

                // Take all pending futures and await them concurrently. Each
                // resolves to a NativeJob that settles its promise; run those
                // jobs, which may enqueue further microtasks/futures.
                let pending: Vec<FutureJob> = self.futures.borrow_mut().drain(..).collect();
                if pending.is_empty() {
                    // Nothing left to drive.
                    if self.jobs.borrow().is_empty() {
                        break;
                    }
                    continue;
                }

                let results = futures::future::join_all(pending).await;
                for native in results {
                    let _ = native.call(context);
                }
            }
        })
    }
}

/// Run a workflow script to completion on the current thread. Must be called
/// inside a tokio `LocalSet` (the futures driven here are `!Send`).
///
/// `agent_tx` carries [`AgentRequest`]s to the orchestrator; `progress_tx`
/// carries phase/log signals; `sub_workflow_tx` carries [`SubWorkflowRequest`]s
/// for nested sub-workflow invocations. `args_json` is exposed to the script
/// as the global `args`. `token_budget` is exposed as the global `budget`
/// informational object.
pub async fn run_script(
    script_body: &str,
    args_json: serde_json::Value,
    agent_tx: mpsc::UnboundedSender<AgentRequest>,
    progress_tx: mpsc::UnboundedSender<ProgressSignal>,
    sub_workflow_tx: mpsc::UnboundedSender<SubWorkflowRequest>,
    token_budget: Option<u64>,
) -> EngineOutcome {
    let queue = Rc::new(WorkflowJobQueue::default());
    let mut context = match Context::builder().job_queue(queue).build() {
        Ok(c) => c,
        Err(e) => {
            return EngineOutcome {
                result: serde_json::Value::Null,
                agent_count: 0,
                error: Some(format!("failed to build JS context: {e}")),
            };
        }
    };

    // Shared agent counter (engine-side index assignment).
    let counter = Rc::new(RefCell::new(0u32));

    // ── __agent(prompt, opts) → Promise<string> ─────────────────────────
    {
        let agent_tx = agent_tx.clone();
        let counter = counter.clone();
        let agent_fn =
            move |_this: &JsValue, args: &[JsValue], ctx: &mut Context| -> JsResult<JsValue> {
                let prompt = args
                    .get_or_undefined(0)
                    .to_string(ctx)?
                    .to_std_string_escaped();
                let opts = args.get_or_undefined(1).clone();
                let (label, phase, model, schema, agent_type, isolation) =
                    parse_agent_opts(&opts, ctx, &prompt);

                *counter.borrow_mut() += 1;
                let index = *counter.borrow();

                let (reply_tx, reply_rx) = oneshot::channel();
                let req = AgentRequest {
                    index,
                    prompt,
                    label,
                    phase,
                    model,
                    schema,
                    agent_type,
                    isolation,
                    reply: reply_tx,
                };

                // If the orchestrator hung up, reject immediately.
                if agent_tx.send(req).is_err() {
                    return Ok(JsPromise::from_future(
                        async move {
                            Err(JsNativeError::error()
                                .with_message("workflow orchestrator unavailable")
                                .into())
                        },
                        ctx,
                    )
                    .into());
                }

                // Bridge the oneshot reply into a JS promise.
                let fut = async move {
                    match reply_rx.await {
                        Ok(Ok(text)) => Ok(JsValue::from(js_string!(text))),
                        Ok(Err(e)) => Err(JsNativeError::error().with_message(e).into()),
                        Err(_) => Err(JsNativeError::error()
                            .with_message("agent reply channel closed")
                            .into()),
                    }
                };
                Ok(JsPromise::from_future(fut, ctx).into())
            };
        // SAFETY: the closure captures only Send channels + Rc counters that
        // live on this thread; no GC-traced values are captured.
        let native = unsafe { NativeFunction::from_closure(agent_fn) };
        context
            .register_global_callable(js_string!("__agent"), 2, native)
            .expect("register __agent");
    }

    // ── __phase(title) ──────────────────────────────────────────────────
    {
        let progress_tx = progress_tx.clone();
        let phase_fn =
            move |_this: &JsValue, args: &[JsValue], ctx: &mut Context| -> JsResult<JsValue> {
                let title = args
                    .get_or_undefined(0)
                    .to_string(ctx)?
                    .to_std_string_escaped();
                let _ = progress_tx.send(ProgressSignal::Phase(title));
                Ok(JsValue::undefined())
            };
        let native = unsafe { NativeFunction::from_closure(phase_fn) };
        context
            .register_global_callable(js_string!("__phase"), 1, native)
            .expect("register __phase");
    }

    // ── __log(message) ──────────────────────────────────────────────────
    {
        let progress_tx = progress_tx.clone();
        let log_fn =
            move |_this: &JsValue, args: &[JsValue], ctx: &mut Context| -> JsResult<JsValue> {
                let msg = args
                    .get_or_undefined(0)
                    .to_string(ctx)?
                    .to_std_string_escaped();
                let _ = progress_tx.send(ProgressSignal::Log(msg));
                Ok(JsValue::undefined())
            };
        let native = unsafe { NativeFunction::from_closure(log_fn) };
        context
            .register_global_callable(js_string!("__log"), 1, native)
            .expect("register __log");
    }

    // ── __workflow(name, args) → Promise<value> ─────────────────────────
    {
        let sub_workflow_tx = sub_workflow_tx.clone();
        let workflow_fn =
            move |_this: &JsValue, args: &[JsValue], ctx: &mut Context| -> JsResult<JsValue> {
                let name = args
                    .get_or_undefined(0)
                    .to_string(ctx)?
                    .to_std_string_escaped();
                let args_js = args.get_or_undefined(1).clone();
                let args_val = args_js.to_json(ctx).unwrap_or(serde_json::Value::Null);

                let (reply_tx, reply_rx) = oneshot::channel();
                let req = SubWorkflowRequest {
                    name,
                    args: args_val,
                    reply: reply_tx,
                };

                if sub_workflow_tx.send(req).is_err() {
                    return Ok(JsPromise::from_future(
                        async move {
                            Err(JsNativeError::error()
                                .with_message("workflow orchestrator unavailable")
                                .into())
                        },
                        ctx,
                    )
                    .into());
                }

                let fut = async move {
                    match reply_rx.await {
                        Ok(Ok(val)) => {
                            // We can't convert serde_json::Value to JsValue here because
                            // Context is not Send. We return it serialized and let boa parse it.
                            Ok(JsValue::from(js_string!(val.to_string())))
                        }
                        Ok(Err(e)) => Err(JsNativeError::error().with_message(e).into()),
                        Err(_) => Err(JsNativeError::error()
                            .with_message("sub-workflow reply channel closed")
                            .into()),
                    }
                };
                Ok(JsPromise::from_future(fut, ctx).into())
            };
        let native = unsafe { NativeFunction::from_closure(workflow_fn) };
        context
            .register_global_callable(js_string!("__workflow"), 2, native)
            .expect("register __workflow");
    }

    // ── inject `args` global ────────────────────────────────────────────
    {
        let args_val = json_to_js(&args_json, &mut context);
        context
            .register_global_property(js_string!("args"), args_val, Attribute::all())
            .expect("register args");
    }

    // ── inject `budget` global (informational snapshot) ─────────────────
    {
        let total = token_budget.unwrap_or(0);
        let budget_json = serde_json::json!({
            "totalTokens": total,
            "spentTokens": 0u64,
            "remainingTokens": total,
        });
        let budget_val = json_to_js(&budget_json, &mut context);
        context
            .register_global_property(js_string!("budget"), budget_val, Attribute::all())
            .expect("register budget");
    }

    // ── run the prelude ─────────────────────────────────────────────────
    if let Err(e) = context.eval(Source::from_bytes(JS_PRELUDE)) {
        return EngineOutcome {
            result: serde_json::Value::Null,
            agent_count: *counter.borrow(),
            error: Some(format!("prelude error: {}", fmt_js_error(&e, &mut context))),
        };
    }

    // ── wrap the user script in an async IIFE so top-level await works,
    //    and capture its returned value ──────────────────────────────────
    let wrapped = format!("globalThis.__wf_result = (async () => {{\n{script_body}\n}})();");

    let eval_result = context.eval(Source::from_bytes(wrapped.as_bytes()));
    if let Err(e) = eval_result {
        return EngineOutcome {
            result: serde_json::Value::Null,
            agent_count: *counter.borrow(),
            error: Some(format!("script error: {}", fmt_js_error(&e, &mut context))),
        };
    }

    // ── drive the job queue (futures + microtasks) to completion ────────
    context.run_jobs_async().await;

    // ── extract the resolved result ─────────────────────────────────────
    let result_promise = context
        .global_object()
        .get(js_string!("__wf_result"), &mut context)
        .ok();

    let result_value = match result_promise {
        Some(v) if v.is_object() => {
            // It's a promise; read its settled state.
            match JsPromise::from_object(v.as_object().unwrap().clone()) {
                Ok(p) => match p.state() {
                    boa_engine::builtins::promise::PromiseState::Fulfilled(v) => {
                        js_to_json(&v, &mut context)
                    }
                    boa_engine::builtins::promise::PromiseState::Rejected(e) => {
                        return EngineOutcome {
                            result: serde_json::Value::Null,
                            agent_count: *counter.borrow(),
                            error: Some(format!(
                                "workflow rejected: {}",
                                e.to_string(&mut context)
                                    .map(|s| s.to_std_string_escaped())
                                    .unwrap_or_else(|_| "<unprintable>".into())
                            )),
                        };
                    }
                    boa_engine::builtins::promise::PromiseState::Pending => serde_json::Value::Null,
                },
                Err(_) => js_to_json(&v, &mut context),
            }
        }
        Some(v) => js_to_json(&v, &mut context),
        None => serde_json::Value::Null,
    };

    EngineOutcome {
        result: result_value,
        agent_count: *counter.borrow(),
        error: None,
    }
}

/// Parse the `opts` object of an `agent()` call into typed fields.
fn parse_agent_opts(
    opts: &JsValue,
    ctx: &mut Context,
    prompt: &str,
) -> (
    String,
    Option<String>,
    Option<String>,
    Option<serde_json::Value>,
    Option<String>,
    Option<String>,
) {
    let default_label: String = prompt
        .chars()
        .take(60)
        .collect::<String>()
        .replace('\n', " ");
    if !opts.is_object() {
        return (default_label, None, None, None, None, None);
    }
    let get_str = |ctx: &mut Context, key: &str| -> Option<String> {
        opts.as_object()
            .and_then(|o| o.get(js_string!(key), ctx).ok())
            .filter(|v| !v.is_undefined() && !v.is_null())
            .and_then(|v| v.to_string(ctx).ok())
            .map(|s| s.to_std_string_escaped())
    };
    let label = get_str(ctx, "label").unwrap_or(default_label);
    let phase = get_str(ctx, "phase");
    let model = get_str(ctx, "model");
    let agent_type = get_str(ctx, "agentType");
    let isolation = get_str(ctx, "isolation");
    let schema = opts
        .as_object()
        .and_then(|o| o.get(js_string!("schema"), ctx).ok())
        .filter(|v| v.is_object())
        .map(|v| js_to_json(&v, ctx));
    (label, phase, model, schema, agent_type, isolation)
}

/// Convert a serde_json::Value into a JsValue.
fn json_to_js(value: &serde_json::Value, ctx: &mut Context) -> JsValue {
    match JsValue::from_json(value, ctx) {
        Ok(v) => v,
        Err(_) => JsValue::null(),
    }
}

/// Convert a JsValue into a serde_json::Value (best-effort).
fn js_to_json(value: &JsValue, ctx: &mut Context) -> serde_json::Value {
    value.to_json(ctx).unwrap_or(serde_json::Value::Null)
}

/// Format a JsError for human display.
fn fmt_js_error(e: &JsError, ctx: &mut Context) -> String {
    e.to_opaque(ctx)
        .to_string(ctx)
        .map(|s| s.to_std_string_escaped())
        .unwrap_or_else(|_| format!("{e:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::task::LocalSet;

    /// Drive a script with a mock orchestrator that echoes prompts.
    async fn run_with_echo(script: &str, args: serde_json::Value) -> EngineOutcome {
        let (agent_tx, mut agent_rx) = mpsc::unbounded_channel::<AgentRequest>();
        let (progress_tx, _progress_rx) = mpsc::unbounded_channel::<ProgressSignal>();
        let (sub_wf_tx, _sub_wf_rx) = mpsc::unbounded_channel::<SubWorkflowRequest>();

        // Mock orchestrator: reply to each agent request with "ok:<prompt>".
        let orchestrator = tokio::spawn(async move {
            while let Some(req) = agent_rx.recv().await {
                let _ = req.reply.send(Ok(format!("ok:{}", req.prompt)));
            }
        });

        let local = LocalSet::new();
        let script = script.to_owned();
        let outcome = local
            .run_until(async move {
                run_script(&script, args, agent_tx, progress_tx, sub_wf_tx, None).await
            })
            .await;
        orchestrator.abort();
        outcome
    }

    #[tokio::test]
    async fn single_agent_returns_result_normal() {
        let out = run_with_echo("return await agent('hello');", serde_json::Value::Null).await;
        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result, serde_json::json!("ok:hello"));
        assert_eq!(out.agent_count, 1);
    }

    #[tokio::test]
    async fn parallel_runs_multiple_agents_normal() {
        let out = run_with_echo(
            "return await parallel([() => agent('a'), () => agent('b'), () => agent('c')]);",
            serde_json::Value::Null,
        )
        .await;
        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result, serde_json::json!(["ok:a", "ok:b", "ok:c"]));
        assert_eq!(out.agent_count, 3);
    }

    #[tokio::test]
    async fn args_global_is_exposed_normal() {
        let out = run_with_echo(
            "return await agent('q:' + args.question);",
            serde_json::json!({ "question": "why" }),
        )
        .await;
        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result, serde_json::json!("ok:q:why"));
    }

    #[tokio::test]
    async fn pipeline_chains_stages_normal() {
        // Each item passes through one stage that calls agent().
        let out = run_with_echo(
            "return await pipeline([1, 2], (n) => agent('n' + n));",
            serde_json::Value::Null,
        )
        .await;
        assert!(out.error.is_none(), "error: {:?}", out.error);
        assert_eq!(out.result, serde_json::json!(["ok:n1", "ok:n2"]));
    }

    #[tokio::test]
    async fn script_error_is_captured_robust() {
        let out = run_with_echo("throw new Error('boom');", serde_json::Value::Null).await;
        assert!(out.error.is_some());
        assert!(out.error.unwrap().contains("boom"));
    }
}
