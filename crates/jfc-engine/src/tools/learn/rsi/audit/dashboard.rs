use serde_json::Value;

pub(super) async fn render_experiment_dashboard(
    out: &mut String,
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
) -> jfc_knowledge::Result<()> {
    out.push_str("\nExperiment dashboard:\n");
    let dashboards = store
        .list_definitions_for_project_status("rsi_experiment_dashboard", project_key, "active", 1)
        .await?;
    let Some(record) = dashboards.first() else {
        out.push_str("- none\n");
        return Ok(());
    };
    let metadata = serde_json::from_str::<Value>(&record.metadata_json).unwrap_or(Value::Null);
    let dashboard = metadata
        .pointer("/rsi/experiment_dashboard")
        .unwrap_or(&Value::Null);
    let trace_count = pointer_u64(dashboard, "/trace_count").unwrap_or_default();
    let best = pointer_f64(dashboard, "/metrics/best_score").unwrap_or_default();
    let latest = pointer_f64(dashboard, "/metrics/latest_score").unwrap_or_default();
    let plateau = pointer_str(dashboard, "/plateau/status").unwrap_or("unknown");
    let hidden = if pointer_bool(dashboard, "/hidden_validation/required") {
        "required"
    } else {
        "missing"
    };
    let anti_cheat = pointer_str(dashboard, "/anti_cheat/status").unwrap_or("unknown");
    let sandbox = if pointer_bool(dashboard, "/sandbox/network_blocked") {
        "network_blocked"
    } else {
        "network_allowed"
    };
    let egress = pointer_str(dashboard, "/sandbox/egress_policy").unwrap_or("unknown");
    let next = pointer_str(dashboard, "/next_action").unwrap_or("unknown");
    let tokens = pointer_u64(dashboard, "/cost/estimated_tokens").unwrap_or_default();
    out.push_str(&format!(
        "- traces={trace_count} best_score={best:.2} latest_score={latest:.2} plateau={plateau} hidden_validation={hidden} anti_cheat={anti_cheat} sandbox={sandbox} egress={egress} next_action={next} cost_tokens={tokens}\n"
    ));
    Ok(())
}

pub(super) async fn render_experiment_loop(
    out: &mut String,
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
) -> jfc_knowledge::Result<()> {
    out.push_str("\nExperiment loop:\n");
    let plans = store
        .list_definitions_for_project_status("rsi_experiment_loop", project_key, "active", 1)
        .await?;
    let Some(record) = plans.first() else {
        out.push_str("- none\n");
        return Ok(());
    };
    let metadata = serde_json::from_str::<Value>(&record.metadata_json).unwrap_or(Value::Null);
    let plan = metadata
        .pointer("/rsi/experiment_loop")
        .unwrap_or(&Value::Null);
    let phase = pointer_str(plan, "/phase").unwrap_or("unknown");
    let timeout = pointer_u64(plan, "/timeout_seconds").unwrap_or_default();
    let commit = pointer_bool(plan, "/commit_required");
    let holdout = pointer_str(plan, "/validation/holdout_name").unwrap_or("unknown");
    let sandbox = if pointer_bool(plan, "/sandbox/network_blocked") {
        "network_blocked"
    } else {
        "network_allowed"
    };
    let egress = pointer_str(plan, "/sandbox/egress_policy").unwrap_or("unknown");
    let tokens = pointer_u64(plan, "/cost/max_next_iteration_tokens").unwrap_or_default();
    let branch = pointer_str(plan, "/branch_strategy").unwrap_or("unknown");
    out.push_str(&format!(
        "- phase={phase} timeout_seconds={timeout} commit_required={commit} holdout={holdout} sandbox={sandbox} egress={egress} max_tokens={tokens} branch_strategy={branch}\n"
    ));
    Ok(())
}

pub(super) async fn render_experiment_job(
    out: &mut String,
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
) -> jfc_knowledge::Result<()> {
    out.push_str("\nExperiment job:\n");
    let jobs = store
        .list_definitions_for_project_status("rsi_experiment_job", project_key, "active", 1)
        .await?;
    let Some(record) = jobs.first() else {
        out.push_str("- none\n");
        return Ok(());
    };
    let metadata = serde_json::from_str::<Value>(&record.metadata_json).unwrap_or(Value::Null);
    let job = metadata
        .pointer("/rsi/experiment_job")
        .unwrap_or(&Value::Null);
    let name = pointer_str(job, "/name").unwrap_or("unknown");
    let phase = pointer_str(job, "/phase").unwrap_or("unknown");
    let cadence = pointer_u64(job, "/schedule/cadence_seconds").unwrap_or_default();
    let max_iterations = pointer_u64(job, "/schedule/max_iterations_per_cycle").unwrap_or_default();
    let preflight = pointer_str(job, "/preflight/status").unwrap_or("unknown");
    let reasons = pointer_array(job, "/preflight/reasons").unwrap_or_else(|| "none".to_owned());
    let holdout = pointer_str(job, "/hidden_validation/holdout_name").unwrap_or("unknown");
    let passed = pointer_u64(job, "/hidden_validation/observed_passed").unwrap_or_default();
    let total = pointer_u64(job, "/hidden_validation/observed_total").unwrap_or_default();
    let metric_guard = pointer_bool(job, "/hidden_validation/reject_metric_mutation");
    let sandbox = if pointer_bool(job, "/sandbox/network_blocked") {
        "network_blocked"
    } else {
        "network_allowed"
    };
    let enforcement = pointer_str(job, "/sandbox/status").unwrap_or("unknown");
    let execution_mode = pointer_str(job, "/sandbox/execution_mode").unwrap_or("unknown");
    let kernel_enforced = pointer_bool(job, "/sandbox/kernel_enforced");
    let sandbox_backend = pointer_str(job, "/sandbox/kernel_backend").unwrap_or("unknown");
    let external = pointer_str(job, "/external_worker_sandbox/status").unwrap_or("unknown");
    let external_backend =
        pointer_str(job, "/external_worker_sandbox/kernel_backend").unwrap_or("unknown");
    let external_egress = pointer_bool(job, "/external_worker_sandbox/egress_isolated");
    let egress = pointer_str(job, "/sandbox/egress_policy").unwrap_or("unknown");
    let fresh = pointer_bool(job, "/sandbox/require_fresh_worktree");
    let tokens = pointer_u64(job, "/cost/max_tokens").unwrap_or_default();
    out.push_str(&format!(
        "- name={name} phase={phase} cadence_seconds={cadence} max_iterations_per_cycle={max_iterations} preflight={preflight} reasons={reasons} holdout={holdout} hidden_validation={passed}/{total} reject_metric_mutation={metric_guard} sandbox_enforcement={enforcement} execution_mode={execution_mode} kernel_enforced={kernel_enforced} sandbox_backend={sandbox_backend} external_worker_sandbox={external} external_backend={external_backend} external_egress_isolated={external_egress} sandbox={sandbox} egress={egress} fresh_worktree={fresh} max_tokens={tokens}\n"
    ));
    Ok(())
}

pub(super) async fn render_experiment_loop_state(
    out: &mut String,
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
) -> jfc_knowledge::Result<()> {
    out.push_str("\nExperiment loop state:\n");
    let states = store
        .list_definitions_for_project_status(
            jfc_learn::RSI_LOOP_STATE_KIND,
            project_key,
            "active",
            1,
        )
        .await?;
    let Some(record) = states.first() else {
        out.push_str("- none\n");
        return Ok(());
    };
    let metadata = serde_json::from_str::<Value>(&record.metadata_json).unwrap_or(Value::Null);
    let state = metadata
        .pointer("/rsi/experiment_loop_state")
        .unwrap_or(&Value::Null);
    let runs = pointer_u64(state, "/run_count").unwrap_or_default();
    let last = pointer_u64(state, "/last_run_at_ms").unwrap_or_default();
    let next = pointer_u64(state, "/next_due_at_ms").unwrap_or_default();
    let cadence = pointer_u64(state, "/cadence_seconds").unwrap_or_default();
    let phase = pointer_str(state, "/phase").unwrap_or("unknown");
    let preflight = pointer_str(state, "/preflight_status").unwrap_or("unknown");
    let actions = pointer_u64(state, "/candidate_actions").unwrap_or_default();
    let traces = pointer_u64(state, "/traces_scored").unwrap_or_default();
    let candidates = pointer_u64(state, "/candidates_seen").unwrap_or_default();
    let tokens = pointer_u64(state, "/total_estimated_tokens").unwrap_or_default();
    out.push_str(&format!(
        "- runs={runs} last_run_at_ms={last} next_due_at_ms={next} cadence_seconds={cadence} phase={phase} preflight={preflight} candidate_actions={actions} traces={traces} candidates={candidates} cost_tokens={tokens}\n"
    ));
    Ok(())
}

fn pointer_str<'a>(value: &'a Value, pointer: &str) -> Option<&'a str> {
    value.pointer(pointer).and_then(Value::as_str)
}

fn pointer_bool(value: &Value, pointer: &str) -> bool {
    value
        .pointer(pointer)
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn pointer_f64(value: &Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer).and_then(Value::as_f64)
}

fn pointer_u64(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(Value::as_u64)
}

fn pointer_array(value: &Value, pointer: &str) -> Option<String> {
    let array = value.pointer(pointer)?.as_array()?;
    if array.is_empty() {
        return Some("none".to_owned());
    }
    Some(
        array
            .iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join("+"),
    )
}
