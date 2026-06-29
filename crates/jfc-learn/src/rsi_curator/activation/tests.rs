use super::{RsiDefinitionRef, promote_rsi_definition, rollback_rsi_definition};
use crate::rsi_curator::ApplyToStore;
use crate::rsi_curator::{
    CandidateKind, CandidateStatus, RsiCurator, RsiCuratorConfig, RsiOutcome, RsiPromotionPolicy,
    RsiToolStep, RsiTrace,
};
use serde_json::json;

fn recovered_tool_trace() -> RsiTrace {
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::Succeeded);
    trace.tool_steps = vec![
        RsiToolStep::new("Edit", false),
        RsiToolStep::new("Read", true),
        RsiToolStep::new("Edit", true),
    ];
    trace.verifications = vec![crate::rsi_curator::RsiVerification::new("cargo test", true)];
    trace
}

fn existing_tool_definition() -> jfc_knowledge::NewDefinition {
    jfc_knowledge::NewDefinition {
        kind: "tool_definition".to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some("proj".to_owned()),
        namespace: None,
        name: "Edit".to_owned(),
        title: Some("Edit".to_owned()),
        description: Some("old".to_owned()),
        body: "old body".to_owned(),
        metadata_json: "{}".to_owned(),
        source_path: Some("rust:tool:Edit".to_owned()),
        source_hash: Some("oldhash".to_owned()),
        status: jfc_knowledge::DefinitionStatus::Active,
        created_by: "test".to_owned(),
    }
}

fn fixture_only_candidate_definition() -> jfc_knowledge::NewDefinition {
    jfc_knowledge::NewDefinition {
        kind: "tool_definition".to_owned(),
        scope: jfc_knowledge::DefinitionScope::Project,
        project_key: Some("proj".to_owned()),
        namespace: None,
        name: "rsi-tool_definition_patch-legacy".to_owned(),
        title: Some("Legacy candidate".to_owned()),
        description: Some("fixture-only candidate".to_owned()),
        body: "verify current path before retrying Edit".to_owned(),
        metadata_json: serde_json::to_string(&json!({
            "rsi": {
                "candidate_id": "legacy",
                "candidate_kind": "tool_definition_patch",
                "target": {
                    "kind": "tool_definition",
                    "name": "Edit",
                },
                "status": "candidate",
                "eval": {
                    "passed": true,
                    "score": 0.9,
                    "reason": "legacy fixture-only pass",
                    "fixtures_run": 1,
                    "fixtures_passed": 1,
                },
                "thinking": {
                    "raw_stored": false,
                },
                "control": {
                    "trust": "verified",
                    "activation_status": "candidate",
                },
            }
        }))
        .unwrap(),
        source_path: Some("rsi:definition:legacy".to_owned()),
        source_hash: Some("legacy".to_owned()),
        status: jfc_knowledge::DefinitionStatus::Candidate,
        created_by: "test".to_owned(),
    }
}

#[tokio::test]
async fn promote_verified_candidate_activates_target_with_rollback_snapshot_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    store
        .upsert_definition(&existing_tool_definition())
        .await
        .unwrap();
    let curator = RsiCurator::new(RsiCuratorConfig::default(), RsiPromotionPolicy::default());
    let report = curator.run(&[recovered_tool_trace()]).unwrap();
    report.apply_to_store(&store, "proj").await.unwrap();
    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.kind == CandidateKind::ToolDefinitionPatch)
        .unwrap();

    let promoted = promote_rsi_definition(
        &store,
        "proj",
        &RsiDefinitionRef::new("tool_definition", candidate.definition_name()),
    )
    .await
    .unwrap();

    assert_eq!(promoted.name, "Edit");
    assert_eq!(promoted.status, CandidateStatus::Active.slug());
    let active = tool_definition(&store, "Edit").await;
    assert_eq!(active.body, candidate.body);
    assert_eq!(active.status, CandidateStatus::Active.slug());
    let metadata: serde_json::Value = serde_json::from_str(&active.metadata_json).unwrap();
    assert_eq!(metadata["rsi"]["control"]["activation_status"], "active");
    assert_eq!(metadata["rsi"]["rollback"]["snapshot"]["body"], "old body");
}

#[tokio::test]
async fn rollback_promoted_candidate_restores_prior_definition_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    store
        .upsert_definition(&existing_tool_definition())
        .await
        .unwrap();
    let curator = RsiCurator::new(RsiCuratorConfig::default(), RsiPromotionPolicy::default());
    let report = curator.run(&[recovered_tool_trace()]).unwrap();
    report.apply_to_store(&store, "proj").await.unwrap();
    let candidate = report
        .candidates
        .iter()
        .find(|candidate| candidate.kind == CandidateKind::ToolDefinitionPatch)
        .unwrap();
    promote_rsi_definition(
        &store,
        "proj",
        &RsiDefinitionRef::new("tool_definition", candidate.definition_name()),
    )
    .await
    .unwrap();

    let rolled_back = rollback_rsi_definition(
        &store,
        "proj",
        &RsiDefinitionRef::new("tool_definition", "Edit"),
    )
    .await
    .unwrap();

    assert_eq!(rolled_back.name, "Edit");
    assert_eq!(rolled_back.status, CandidateStatus::Active.slug());
    let active = tool_definition(&store, "Edit").await;
    assert_eq!(active.body, "old body");
    assert_eq!(active.source_path.as_deref(), Some("rust:tool:Edit"));
}

#[tokio::test]
async fn promote_rejects_fixture_only_candidate_without_research_gate_robust() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    store
        .upsert_definition(&fixture_only_candidate_definition())
        .await
        .unwrap();

    let err = promote_rsi_definition(
        &store,
        "proj",
        &RsiDefinitionRef::new("tool_definition", "rsi-tool_definition_patch-legacy"),
    )
    .await
    .unwrap_err();

    assert!(err.to_string().contains("research-gate evidence"));
}

async fn tool_definition(
    store: &jfc_knowledge::KnowledgeStore,
    name: &str,
) -> jfc_knowledge::DefinitionRecord {
    store
        .get_definition_by_name(
            "tool_definition",
            jfc_knowledge::DefinitionScope::Project,
            Some("proj"),
            None,
            name,
        )
        .await
        .unwrap()
        .unwrap()
}
