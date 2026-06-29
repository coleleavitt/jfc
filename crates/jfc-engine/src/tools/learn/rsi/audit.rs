use serde_json::Value;

mod dashboard;

use super::health;

const DEFINITION_KINDS: &[&str] = &[
    "skill",
    "system_prompt",
    "tool_definition",
    "harness_patch",
    "context_playbook",
    "budget_policy",
    "reasoning_policy",
];

pub async fn render_rsi_audit(
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    status: Option<&str>,
    limit: Option<u64>,
) -> jfc_knowledge::Result<String> {
    let limit = limit.unwrap_or(20).clamp(1, 100) as usize;
    let statuses = statuses_for(status);
    let mut out = String::new();
    out.push_str(&format!(
        "RSI audit for project `{project_key}` status={} limit={limit}\n",
        status.unwrap_or("candidate")
    ));
    out.push_str("\nDefinitions:\n");
    let mut definition_count = 0usize;
    for status in &statuses {
        for kind in DEFINITION_KINDS {
            let records = store
                .list_definitions_for_project_status(kind, project_key, status, limit)
                .await?;
            for record in records {
                if !is_rsi_definition(&record) {
                    continue;
                }
                definition_count += 1;
                render_definition(&mut out, &record);
            }
        }
    }
    if definition_count == 0 {
        out.push_str("- none\n");
    }
    dashboard::render_experiment_dashboard(&mut out, store, project_key).await?;
    dashboard::render_experiment_loop(&mut out, store, project_key).await?;
    dashboard::render_experiment_job(&mut out, store, project_key).await?;
    dashboard::render_experiment_loop_state(&mut out, store, project_key).await?;
    render_memory_events(&mut out, store, project_key, &statuses, limit).await?;
    Ok(out)
}

fn statuses_for(status: Option<&str>) -> Vec<&'static str> {
    match status.unwrap_or("candidate") {
        "all" => vec!["candidate", "active", "rejected", "superseded"],
        "active" => vec!["active"],
        "rejected" => vec!["rejected"],
        "superseded" => vec!["superseded"],
        _ => vec!["candidate"],
    }
}

fn is_rsi_definition(record: &jfc_knowledge::DefinitionRecord) -> bool {
    record
        .source_path
        .as_deref()
        .is_some_and(|path| path.starts_with("rsi:definition:"))
        || serde_json::from_str::<Value>(&record.metadata_json)
            .ok()
            .and_then(|metadata| metadata.get("rsi").cloned())
            .is_some()
}

fn render_definition(out: &mut String, record: &jfc_knowledge::DefinitionRecord) {
    let metadata = serde_json::from_str::<Value>(&record.metadata_json).unwrap_or(Value::Null);
    let rsi = metadata.get("rsi").unwrap_or(&Value::Null);
    let target_kind = pointer_str(rsi, "/target/kind").unwrap_or(record.kind.as_str());
    let target_name = pointer_str(rsi, "/target/name").unwrap_or(record.name.as_str());
    let profile = pointer_str(rsi, "/eval/research/profile").unwrap_or("unknown");
    let research_ok = pointer_bool(rsi, "/eval/research/verified");
    let score = pointer_f64(rsi, "/eval/score").unwrap_or_default();
    let fixtures_run = pointer_u64(rsi, "/eval/fixtures_run").unwrap_or_default();
    let fixtures_passed = pointer_u64(rsi, "/eval/fixtures_passed").unwrap_or_default();
    let capability = pointer_str(rsi, "/control/capability").unwrap_or("unknown");
    let trust = pointer_str(rsi, "/control/trust").unwrap_or("unknown");
    let approval = pointer_bool(rsi, "/control/approval_required");
    let thinking_source = pointer_str(rsi, "/thinking/source").unwrap_or("unknown");
    let raw_stored = pointer_bool(rsi, "/thinking/raw_stored");
    let rollback = pointer_str(rsi, "/rollback/action").unwrap_or("unknown");
    out.push_str(&format!(
        "- {}/{} [{}] target={}/{} score={score:.2} fixtures={fixtures_passed}/{fixtures_run} research={profile} verified={research_ok} control={capability}/{trust} approval_required={approval} thinking={thinking_source} raw_stored={raw_stored} rollback={rollback}\n",
        record.kind, record.name, record.status, target_kind, target_name
    ));
    if record.status == "candidate" {
        out.push_str(&format!(
            "  promote: learn_rsi_promote kind={} name={}\n",
            record.kind, record.name
        ));
    }
    if record.status == "active" {
        let health = health::assess_definition(record, rsi);
        out.push_str(&format!("  health={}", health.status.slug()));
        if !health.reasons.is_empty() {
            out.push_str(" reasons=");
            out.push_str(&health.reasons.join(","));
        }
        out.push('\n');
        out.push_str(&format!(
            "  rollback: learn_rsi_rollback kind={} name={}\n",
            record.kind, record.name
        ));
    }
}

async fn render_memory_events(
    out: &mut String,
    store: &jfc_knowledge::KnowledgeStore,
    project_key: &str,
    statuses: &[&str],
    limit: usize,
) -> jfc_knowledge::Result<()> {
    out.push_str("\nMemory-rule learning events:\n");
    let mut count = 0usize;
    for status in statuses {
        let events = store.list_learning_events(Some(status), limit * 4).await?;
        for event in events {
            let evidence =
                serde_json::from_str::<Value>(&event.verifier_evidence).unwrap_or(Value::Null);
            if !matches!(
                pointer_str(&evidence, "/candidate_kind"),
                Some("memory_rule")
            ) {
                continue;
            }
            let event_project = pointer_str(&evidence, "/project_key");
            if let Some(event_project) = event_project
                && event_project != project_key
            {
                continue;
            }
            count += 1;
            render_memory_event(out, &event, &evidence, event_project);
            if count >= limit {
                break;
            }
        }
        if count >= limit {
            break;
        }
    }
    if count == 0 {
        out.push_str("- none\n");
    }
    Ok(())
}

fn render_memory_event(
    out: &mut String,
    event: &jfc_knowledge::LearningEventRow,
    evidence: &Value,
    project: Option<&str>,
) {
    let score = pointer_f64(evidence, "/eval/score").unwrap_or_default();
    let profile = pointer_str(evidence, "/eval/research/profile").unwrap_or("unknown");
    let research_ok = pointer_bool(evidence, "/eval/research/verified");
    let trust = pointer_str(evidence, "/control/trust").unwrap_or("unknown");
    out.push_str(&format!(
        "- {} [{}] project={} recurrence={} score={score:.2} research={profile} verified={research_ok} trust={trust} note=memory rules activate via the curator, not learn_rsi_promote\n",
        event.id,
        event.status,
        project.unwrap_or("legacy-ledger"),
        event.recurrence_count
    ));
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
