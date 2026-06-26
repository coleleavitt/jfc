use serde_json::json;
use sha2::{Digest, Sha256};

use super::{CandidateChange, CandidateStatus, ControlAssessment};

pub(super) fn definition_metadata(
    candidate: &CandidateChange,
    prior: Option<&jfc_knowledge::DefinitionRecord>,
    graph: &super::ExperienceGraph,
    control: &ControlAssessment,
) -> String {
    serde_json::to_string_pretty(&json!({
        "rsi": {
            "candidate_id": candidate.id,
            "candidate_kind": candidate.kind.slug(),
            "target": {
                "kind": candidate.target.kind,
                "name": candidate.target.name,
            },
            "status": candidate.status.slug(),
            "score": candidate.score,
            "eval": {
                "passed": candidate.eval.passed,
                "score": candidate.eval.score,
                "reason": candidate.eval.reason,
                "fixtures_run": candidate.eval.fixtures_run,
                "fixtures_passed": candidate.eval.fixtures_passed,
                "research": research_metadata(&candidate.eval),
            },
            "budget": candidate.budget.as_ref().map(budget_metadata),
            "thinking": thinking_metadata(candidate.thinking),
            "experience_graph": graph_metadata(candidate, graph),
            "control": control_metadata(control),
            "provenance": {
                "source_session_id": candidate.source_session_id,
                "source_turn_id": candidate.source_turn_id,
                "evidence": candidate.evidence,
            },
            "prior": prior.map(prior_metadata),
            "rollback": rollback_metadata(prior),
        }
    }))
    .unwrap_or_else(|_| "{}".to_owned())
}

pub(super) fn control_metadata(control: &ControlAssessment) -> serde_json::Value {
    json!({
        "capability": control.capability.slug(),
        "trust": control.trust.slug(),
        "activation_status": control.activation_status.slug(),
        "approval_required": control.approval_required,
        "reason": control.reason,
    })
}

pub(super) fn graph_metadata(
    candidate: &CandidateChange,
    graph: &super::ExperienceGraph,
) -> serde_json::Value {
    json!({
        "nodes": graph.nodes.len(),
        "edges": graph.edges.len(),
        "candidate_node": format!("candidate:{}", candidate.id),
        "source_trace_node": format!("trace:{}", candidate.source_session_id),
    })
}

pub(super) fn definition_status(status: CandidateStatus) -> jfc_knowledge::DefinitionStatus {
    match status {
        CandidateStatus::Candidate => jfc_knowledge::DefinitionStatus::Candidate,
        CandidateStatus::Active => jfc_knowledge::DefinitionStatus::Active,
        CandidateStatus::Rejected => jfc_knowledge::DefinitionStatus::Rejected,
    }
}

pub(super) fn content_hash(body: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(body.as_bytes());
    to_hex(&hasher.finalize())
}

fn thinking_metadata(thinking: super::candidate::ThinkingProvenance) -> serde_json::Value {
    json!({
        "source": thinking.source.slug(),
        "private_blocks_seen": thinking.private_blocks_seen,
        "thinking_tokens": thinking.thinking_tokens,
        "raw_stored": thinking.raw_stored,
        "support": thinking.support.slug(),
        "self_consistency": thinking.self_consistency.slug(),
        "observable_support_count": thinking.observable_support_count,
    })
}

pub(super) fn research_metadata(eval: &super::CandidateEval) -> serde_json::Value {
    json!({
        "profile": eval.research_profile.slug(),
        "checks_run": eval.research_checks_run,
        "checks_passed": eval.research_checks_passed,
        "verified": eval.research_verified(),
        "checks": eval.research_checks.iter().map(|check| {
            json!({
                "name": check.name,
                "passed": check.passed,
            })
        }).collect::<Vec<_>>(),
        "lineage": eval.research_lineage.iter().map(|reference| {
            json!({
                "paper_id": reference.paper_id,
                "role": reference.role,
            })
        }).collect::<Vec<_>>(),
    })
}

fn budget_metadata(budget: &super::BudgetRecommendation) -> serde_json::Value {
    json!({
        "model": budget.model,
        "effort": budget.effort,
        "recommendation": budget.recommendation,
        "tool_visibility": budget.tool_visibility.iter().map(|item| {
            json!({
                "tool_name": item.tool_name,
                "action": item.action.slug(),
                "reason": item.reason,
            })
        }).collect::<Vec<_>>(),
    })
}

fn prior_metadata(prior: &jfc_knowledge::DefinitionRecord) -> serde_json::Value {
    json!({
        "id": prior.id,
        "version": prior.version,
        "source_hash": prior.source_hash,
        "source_path": prior.source_path,
    })
}

fn rollback_metadata(prior: Option<&jfc_knowledge::DefinitionRecord>) -> serde_json::Value {
    match prior {
        Some(prior) => json!({
            "action": "restore_prior_definition",
            "definition_id": prior.id,
            "version": prior.version,
        }),
        None => json!({
            "action": "delete_candidate_definition",
        }),
    }
}

fn to_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}
