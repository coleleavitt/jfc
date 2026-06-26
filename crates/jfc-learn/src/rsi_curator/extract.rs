use serde_json::Value;

use super::{
    RsiAgentFanout, RsiCuratorConfig, RsiCuratorJob, RsiOutcome, RsiPromotionPolicy,
    RsiRetrievalStep, RsiSelectionEvent, RsiToolStep, RsiTrace, RsiVerification,
};
use crate::error::LearnError;

pub fn trace_from_messages(
    session_id: impl Into<String>,
    messages: &[jfc_knowledge::SessionMessage],
) -> RsiTrace {
    let mut trace = RsiTrace::new(session_id);
    for message in messages {
        let Some(meta) = message
            .meta
            .as_deref()
            .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        else {
            inspect_plain_message(message, &mut trace);
            continue;
        };
        if trace.model.is_none() {
            trace.model = meta
                .get("model_name")
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
        if trace.thinking_tokens == 0 {
            trace.thinking_tokens = meta
                .get("usage")
                .and_then(|usage| usage.get("thinking_tokens"))
                .and_then(Value::as_u64)
                .unwrap_or(0);
        }
        let Some(parts) = meta.get("parts").and_then(Value::as_array) else {
            inspect_plain_message(message, &mut trace);
            continue;
        };
        for part in parts {
            inspect_part(&message.role, part, &mut trace);
        }
    }
    if trace.thinking_tokens == 0 {
        trace.thinking_tokens = trace
            .thinking_blocks
            .iter()
            .map(|block| (block.len() as u64).div_ceil(4))
            .sum();
    }
    trace.outcome = Some(infer_outcome(&trace));
    trace
}

pub async fn load_trace_from_store(
    store: &jfc_knowledge::KnowledgeStore,
    session_id: &str,
) -> Result<RsiTrace, LearnError> {
    let messages = store.load_transcript(session_id).await?;
    let mut trace = trace_from_messages(session_id, &messages);
    augment_trace_from_store(store, session_id, &mut trace).await?;
    trace.outcome = Some(infer_outcome(&trace));
    Ok(trace)
}

pub async fn load_recent_traces_from_store(
    store: &jfc_knowledge::KnowledgeStore,
    cwd: Option<&str>,
    limit: usize,
) -> Result<Vec<RsiTrace>, LearnError> {
    let sessions = store.list_sessions(cwd, limit).await?;
    let mut traces = Vec::new();
    for session in sessions {
        let trace = load_trace_from_store(store, &session.id).await?;
        if !trace.tool_steps.is_empty()
            || !trace.thinking_blocks.is_empty()
            || !trace.retrieval_steps.is_empty()
            || !trace.agent_fanouts.is_empty()
            || !trace.selections.is_empty()
            || trace.user_correction.is_some()
        {
            traces.push(trace);
        }
    }
    Ok(traces)
}

pub async fn build_recent_rsi_job(
    store: &jfc_knowledge::KnowledgeStore,
    cwd: Option<&str>,
    limit: usize,
    config: RsiCuratorConfig,
    promotion_policy: RsiPromotionPolicy,
) -> Result<Option<RsiCuratorJob>, LearnError> {
    let project_key = cwd.map(|cwd| jfc_knowledge::project_key(std::path::Path::new(cwd)));
    if let Some(project_key) = &project_key {
        let decision =
            super::experiment_loop_due_decision(store, project_key, super::current_time_ms())
                .await?;
        if !decision.due {
            return Ok(None);
        }
    }
    let mut traces = load_recent_traces_from_store(store, cwd, limit).await?;
    if let Some(project_key) = &project_key
        && let Some(trace) = load_project_activity_trace_from_store(store, project_key).await?
    {
        traces.push(trace);
    }
    if traces.is_empty() {
        return Ok(None);
    }
    Ok(Some(RsiCuratorJob {
        traces,
        config,
        promotion_policy,
        project_key,
        sandbox_enforcement: None,
        worker: None,
    }))
}

async fn load_project_activity_trace_from_store(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
) -> Result<Option<RsiTrace>, LearnError> {
    let session_id = format!("project:{project_key}");
    let mut trace = RsiTrace::new(session_id.clone());
    for event in store.list_agent_events(&session_id, 500).await? {
        inspect_agent_event(&event.kind, &event.content, &mut trace);
    }
    for artifact in store
        .list_session_artifacts(&session_id, "bounty", 100)
        .await?
    {
        inspect_bounty_value(&artifact.value_json, &mut trace);
    }
    if trace.agent_fanouts.is_empty()
        && trace.selections.is_empty()
        && trace.retrieval_steps.is_empty()
        && trace.tool_steps.is_empty()
    {
        return Ok(None);
    }
    trace.outcome = Some(infer_outcome(&trace));
    Ok(Some(trace))
}

async fn augment_trace_from_store(
    store: &jfc_knowledge::KnowledgeStore,
    session_id: &str,
    trace: &mut RsiTrace,
) -> Result<(), LearnError> {
    let tool_runs = store.list_session_tool_runs(session_id).await?;
    for row in tool_runs.iter().skip(trace.tool_steps.len()) {
        let (step, verification) = trace_from_tool_run(row);
        trace.tool_steps.push(step);
        if let Some(verification) = verification {
            trace.verifications.push(verification);
        }
    }

    for event in store.list_session_retrieval_events(session_id).await? {
        trace.retrieval_steps.push(RsiRetrievalStep::new(
            event.query,
            event.source,
            event.result_count,
        ));
    }

    for event in store.list_agent_events(session_id, 500).await? {
        inspect_agent_event(&event.kind, &event.content, trace);
    }
    Ok(())
}

fn inspect_plain_message(message: &jfc_knowledge::SessionMessage, trace: &mut RsiTrace) {
    if message.role == "user" && is_correction(&message.content) {
        trace.user_correction = Some(message.content.clone());
    }
}

fn inspect_part(role: &str, part: &Value, trace: &mut RsiTrace) {
    let part_type = part.get("type").and_then(Value::as_str).unwrap_or_default();
    match part_type {
        "reasoning" => {
            if let Some(content) = part.get("content").and_then(Value::as_str) {
                trace.thinking_blocks.push(content.to_owned());
            }
        }
        "tool" => inspect_tool_part(part, trace),
        "text" if role == "user" => {
            if let Some(content) = part.get("content").and_then(Value::as_str)
                && is_correction(content)
            {
                trace.user_correction = Some(content.to_owned());
            }
        }
        "text"
        | "reasoning_signature"
        | "task_status"
        | "compact_boundary"
        | "advisor"
        | "redacted_thinking" => {}
        _ => {}
    }
}

fn inspect_tool_part(part: &Value, trace: &mut RsiTrace) {
    let name = part
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("tool")
        .to_owned();
    let status = part
        .get("status")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let success = status_success(status);
    if let Some(command) = command_from_value(part)
        && is_verification_command(&command)
    {
        trace
            .verifications
            .push(RsiVerification::new(command, success));
    }
    trace.tool_steps.push(RsiToolStep::new(name, success));
}

fn trace_from_tool_run(
    row: &jfc_knowledge::SessionToolRunRow,
) -> (RsiToolStep, Option<RsiVerification>) {
    let success = status_success(&row.status);
    let verification = row
        .input_json
        .as_deref()
        .and_then(command_from_input_json)
        .filter(|command| is_verification_command(command))
        .map(|command| RsiVerification::new(command, success));
    (RsiToolStep::new(row.kind.clone(), success), verification)
}

fn inspect_agent_event(kind: &str, content: &str, trace: &mut RsiTrace) {
    if kind.starts_with("bounty.") {
        inspect_bounty_value(content, trace);
        return;
    }
    let success = match kind {
        "agent.completed" => Some(true),
        "agent.failed" => Some(false),
        _ => None,
    };
    if let Some(succeeded) = success {
        trace
            .agent_fanouts
            .push(RsiAgentFanout::new("agent", 1, succeeded));
    }
}

fn inspect_bounty_value(raw: &str, trace: &mut RsiTrace) {
    let Ok(value) = serde_json::from_str::<Value>(raw) else {
        return;
    };
    let kind = value
        .get("kind")
        .and_then(Value::as_str)
        .or_else(|| {
            value
                .get("event_kind")
                .and_then(Value::as_str)
                .map(|kind| kind.strip_prefix("bounty.").unwrap_or(kind))
        })
        .unwrap_or_default();
    let payload = value.get("payload").unwrap_or(&value);
    match kind {
        "dispatch_started" => {
            let count = payload
                .get("n_solvers")
                .and_then(Value::as_u64)
                .unwrap_or(1)
                .max(1);
            trace
                .agent_fanouts
                .push(RsiAgentFanout::new("bounty", count, true));
        }
        "settled" => {
            let winner = payload
                .get("winner")
                .and_then(Value::as_str)
                .map(str::to_owned);
            let selected_from = payload.get("payouts").and_then(Value::as_u64);
            trace
                .selections
                .push(RsiSelectionEvent::new("bounty", winner, selected_from));
        }
        "failed" => {
            trace
                .agent_fanouts
                .push(RsiAgentFanout::new("bounty", 1, false));
        }
        _ => {}
    }
}

fn infer_outcome(trace: &RsiTrace) -> RsiOutcome {
    if trace.user_correction.is_some() {
        return RsiOutcome::UserCorrected;
    }
    if trace
        .verifications
        .iter()
        .any(|verification| !verification.passed)
    {
        return RsiOutcome::Failed;
    }
    if trace
        .verifications
        .iter()
        .any(|verification| verification.passed)
    {
        return RsiOutcome::Succeeded;
    }
    if trace
        .selections
        .iter()
        .any(|selection| selection.winner.is_some())
    {
        return RsiOutcome::Succeeded;
    }
    if trace.agent_fanouts.iter().any(|fanout| !fanout.succeeded) {
        return RsiOutcome::Failed;
    }
    if !trace.tool_steps.is_empty() && trace.tool_steps.iter().all(|step| step.success) {
        return RsiOutcome::Succeeded;
    }
    RsiOutcome::Failed
}

fn status_success(status: &str) -> bool {
    let normalized = status.to_ascii_lowercase();
    matches!(
        normalized.as_str(),
        "complete" | "completed" | "success" | "succeeded" | "ok"
    )
}

fn command_from_input_json(raw: &str) -> Option<String> {
    serde_json::from_str::<Value>(raw)
        .ok()
        .and_then(|value| command_from_value(&value))
}

fn command_from_value(value: &Value) -> Option<String> {
    if let Some(command) = value.get("command").and_then(Value::as_str) {
        return Some(command.to_owned());
    }
    if let Some(command) = value.get("cmd").and_then(Value::as_str) {
        return Some(command.to_owned());
    }
    if let Some(input) = value.get("input")
        && let Some(command) = command_from_value(input)
    {
        return Some(command);
    }
    if let Some(args) = value.get("args")
        && let Some(command) = command_from_value(args)
    {
        return Some(command);
    }
    None
}

fn is_verification_command(command: &str) -> bool {
    let normalized = command.to_ascii_lowercase();
    [
        "cargo test",
        "cargo build",
        "cargo clippy",
        "make ",
        "npm test",
        "bun test",
    ]
    .iter()
    .any(|needle| normalized.contains(needle))
}

fn is_correction(text: &str) -> bool {
    let normalized = text.trim_start().to_ascii_lowercase();
    [
        "no,",
        "no ",
        "actually",
        "that's wrong",
        "thats wrong",
        "incorrect",
        "instead",
        "stop",
    ]
    .iter()
    .any(|cue| normalized.starts_with(cue) || normalized.contains(cue))
}

#[cfg(test)]
mod tests;
