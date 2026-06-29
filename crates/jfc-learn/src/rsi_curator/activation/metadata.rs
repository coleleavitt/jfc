use serde_json::{Value, json};

use crate::error::LearnError;

pub(super) fn metadata_value(
    record: &jfc_knowledge::DefinitionRecord,
) -> Result<Value, LearnError> {
    Ok(serde_json::from_str(&record.metadata_json)?)
}

pub(super) fn require_verified_candidate(metadata: &Value) -> Result<(), LearnError> {
    let rsi = require_rsi_metadata(metadata)?;
    let trust = rsi.pointer("/control/trust").and_then(Value::as_str);
    let raw_stored = rsi
        .pointer("/thinking/raw_stored")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    let fixtures_run = rsi
        .pointer("/eval/fixtures_run")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let fixtures_passed = rsi
        .pointer("/eval/fixtures_passed")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let research_verified = rsi
        .pointer("/eval/research/verified")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let research_checks_run = rsi
        .pointer("/eval/research/checks_run")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    if trust == Some("verified")
        && !raw_stored
        && fixtures_run > 0
        && fixtures_run == fixtures_passed
        && research_verified
        && research_checks_run > 0
    {
        return Ok(());
    }
    Err(LearnError::ContractViolation {
        message: "RSI candidate lacks fixture and research-gate evidence for active rollout"
            .to_owned(),
    })
}

pub(super) fn require_rsi_metadata(metadata: &Value) -> Result<&Value, LearnError> {
    metadata
        .get("rsi")
        .ok_or_else(|| LearnError::ContractViolation {
            message: "definition is not an RSI definition".to_owned(),
        })
}

pub(super) fn target_name(metadata: &Value) -> Result<String, LearnError> {
    require_rsi_metadata(metadata)?
        .pointer("/target/name")
        .and_then(Value::as_str)
        .filter(|name| !name.trim().is_empty())
        .map(str::to_owned)
        .ok_or_else(|| LearnError::ContractViolation {
            message: "RSI definition metadata is missing target.name".to_owned(),
        })
}

pub(super) fn mark_promoted_metadata(
    metadata: &mut Value,
    candidate: &jfc_knowledge::DefinitionRecord,
    prior: Option<&jfc_knowledge::DefinitionRecord>,
) -> Result<(), LearnError> {
    let Some(rsi) = metadata.get_mut("rsi").and_then(Value::as_object_mut) else {
        return Err(LearnError::ContractViolation {
            message: "definition is not an RSI definition".to_owned(),
        });
    };
    rsi.insert("status".to_owned(), json!("active"));
    if let Some(control) = rsi.get_mut("control").and_then(Value::as_object_mut) {
        control.insert("activation_status".to_owned(), json!("active"));
        control.insert(
            "reason".to_owned(),
            json!("approved RSI definition promoted to active runtime"),
        );
    }
    rsi.insert(
        "rollout".to_owned(),
        json!({
            "action": "promote_candidate_definition",
            "promoted_from": candidate.name,
            "previous_active": prior.map(snapshot_metadata),
        }),
    );
    rsi.insert("rollback".to_owned(), rollback_metadata(prior));
    Ok(())
}

fn rollback_metadata(prior: Option<&jfc_knowledge::DefinitionRecord>) -> Value {
    match prior {
        Some(prior) => json!({
            "action": "restore_prior_definition",
            "snapshot": snapshot_metadata(prior),
        }),
        None => json!({
            "action": "deactivate_active_definition",
        }),
    }
}

fn snapshot_metadata(record: &jfc_knowledge::DefinitionRecord) -> Value {
    json!({
        "title": record.title,
        "description": record.description,
        "body": record.body,
        "metadata_json": record.metadata_json,
        "source_path": record.source_path,
        "source_hash": record.source_hash,
    })
}

pub(super) fn required_string(value: &Value, key: &str) -> Result<String, LearnError> {
    optional_string(value, key).ok_or_else(|| LearnError::ContractViolation {
        message: format!("rollback snapshot is missing `{key}`"),
    })
}

pub(super) fn optional_string(value: &Value, key: &str) -> Option<String> {
    value.get(key).and_then(Value::as_str).map(str::to_owned)
}
