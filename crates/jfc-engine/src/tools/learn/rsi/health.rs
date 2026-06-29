use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RsiDefinitionHealth {
    pub(super) status: RsiHealthStatus,
    pub(super) reasons: Vec<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RsiHealthStatus {
    Healthy,
    RollbackRecommended,
}

impl RsiHealthStatus {
    pub(super) const fn slug(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::RollbackRecommended => "rollback_recommended",
        }
    }
}

pub(super) fn assess_definition(
    record: &jfc_knowledge::DefinitionRecord,
    rsi: &Value,
) -> RsiDefinitionHealth {
    let mut reasons = Vec::new();
    if pointer_str(rsi, "/status") != Some("active") {
        reasons.push("metadata_not_active");
    }
    if pointer_str(rsi, "/control/activation_status") != Some("active") {
        reasons.push("control_not_active");
    }
    if pointer_str(rsi, "/control/trust") != Some("verified") {
        reasons.push("trust_not_verified");
    }
    if pointer_bool(rsi, "/thinking/raw_stored") {
        reasons.push("raw_thinking_stored");
    }
    let fixtures_run = pointer_u64(rsi, "/eval/fixtures_run").unwrap_or_default();
    let fixtures_passed = pointer_u64(rsi, "/eval/fixtures_passed").unwrap_or_default();
    if fixtures_run == 0 || fixtures_run != fixtures_passed {
        reasons.push("fixture_gate_regressed");
    }
    if !pointer_bool(rsi, "/eval/research/verified")
        || pointer_u64(rsi, "/eval/research/checks_run").unwrap_or_default() == 0
    {
        reasons.push("research_gate_missing");
    }
    match pointer_str(rsi, "/rollback/action") {
        Some("restore_prior_definition" | "deactivate_active_definition") => {}
        _ => reasons.push("rollback_path_missing"),
    }
    if record
        .source_hash
        .as_deref()
        .is_none_or(|hash| hash != content_hash(&record.body))
    {
        reasons.push("body_hash_mismatch");
    }
    let status = if reasons.is_empty() {
        RsiHealthStatus::Healthy
    } else {
        RsiHealthStatus::RollbackRecommended
    };
    RsiDefinitionHealth { status, reasons }
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

fn pointer_u64(value: &Value, pointer: &str) -> Option<u64> {
    value.pointer(pointer).and_then(Value::as_u64)
}

fn content_hash(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn assess_definition_recommends_rollback_when_active_evidence_regresses_robust() {
        let record = jfc_knowledge::DefinitionRecord {
            id: "id".to_owned(),
            kind: "tool_definition".to_owned(),
            scope: "project".to_owned(),
            project_key: Some("proj".to_owned()),
            namespace: None,
            name: "Edit".to_owned(),
            title: None,
            description: None,
            body: "current body".to_owned(),
            metadata_json: "{}".to_owned(),
            source_path: Some("rsi:definition:candidate".to_owned()),
            source_hash: Some("stale".to_owned()),
            status: "active".to_owned(),
            version: 1,
            created_by: "test".to_owned(),
            created_at_ms: 0,
            updated_at_ms: 0,
            superseded_by: None,
        };
        let rsi = serde_json::json!({
            "status": "active",
            "control": {
                "activation_status": "active",
                "trust": "verified",
            },
            "thinking": {
                "raw_stored": true,
            },
            "eval": {
                "fixtures_run": 1,
                "fixtures_passed": 0,
                "research": {
                    "verified": false,
                    "checks_run": 0,
                },
            },
            "rollback": {
                "action": "restore_prior_definition",
            },
        });

        let health = assess_definition(&record, &rsi);

        assert_eq!(health.status, RsiHealthStatus::RollbackRecommended);
        assert!(health.reasons.contains(&"raw_thinking_stored"));
        assert!(health.reasons.contains(&"fixture_gate_regressed"));
        assert!(health.reasons.contains(&"research_gate_missing"));
        assert!(health.reasons.contains(&"body_hash_mismatch"));
    }
}
