use crate::rsi_curator::ApplyToStore;
use crate::rsi_curator::{
    CandidateChange, CandidateEval, CandidateKind, CandidateStatus, CandidateTarget,
    ExperienceGraph, RsiCurator, RsiCuratorConfig, RsiCuratorReport, RsiOutcome,
    RsiPromotionPolicy, RsiTrace, ThinkingProvenance,
};

mod experiment;

#[tokio::test]
async fn active_tool_patch_without_fixture_evidence_is_quarantined_robust() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    let candidate = CandidateChange {
        id: "1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef".to_owned(),
        kind: CandidateKind::ToolDefinitionPatch,
        target: CandidateTarget {
            kind: "tool_definition".to_owned(),
            name: "Edit".to_owned(),
        },
        title: "Tool patch".to_owned(),
        body: "verify current path before retrying Edit".to_owned(),
        evidence: "session=s1".to_owned(),
        source_session_id: "s1".to_owned(),
        source_turn_id: None,
        score: 0.92,
        recurrence_count: 1,
        eval: CandidateEval::pass(0.92, "passed"),
        status: CandidateStatus::Active,
        budget: None,
        thinking: ThinkingProvenance::from_trace(&RsiTrace::new("s1")),
    };
    let report = RsiCuratorReport {
        traces_scored: 1,
        candidates: vec![candidate],
        experience_graph: ExperienceGraph::default(),
        experiment_dashboard: Default::default(),
        experiment_loop: Default::default(),
        experiment_job: Default::default(),
    };

    let applied = report.apply_to_store(&store, "proj").await.unwrap();

    assert_eq!(applied.definitions, 1);
    let active_slot = store
        .get_definition_by_name(
            "tool_definition",
            jfc_knowledge::DefinitionScope::Project,
            Some("proj"),
            None,
            "Edit",
        )
        .await
        .unwrap();
    assert!(active_slot.is_none());
    let quarantined = store
        .get_definition_by_name(
            "tool_definition",
            jfc_knowledge::DefinitionScope::Project,
            Some("proj"),
            None,
            "rsi-tool_definition_patch-1234567890ab",
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(quarantined.status, CandidateStatus::Candidate.slug());
    let metadata: serde_json::Value = serde_json::from_str(&quarantined.metadata_json).unwrap();
    assert_eq!(metadata["rsi"]["control"]["trust"], "candidate");
    assert_eq!(metadata["rsi"]["control"]["activation_status"], "candidate");
    let learning = store
        .list_learning_events(Some(CandidateStatus::Candidate.slug()), 5)
        .await
        .unwrap();
    assert_eq!(learning.len(), 1);
    let evidence: serde_json::Value = serde_json::from_str(&learning[0].verifier_evidence).unwrap();
    assert_eq!(evidence["project_key"], "proj");
    assert_eq!(evidence["control"]["activation_status"], "candidate");
}

#[tokio::test]
async fn approved_memory_rule_promotes_to_knowledge_record_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    let policy = RsiPromotionPolicy::auto_activate_verified();
    let curator = RsiCurator::new(RsiCuratorConfig::default(), policy);
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::UserCorrected);
    trace.user_correction = Some("actually inspect the current file".to_owned());
    trace.verifications = vec![crate::rsi_curator::RsiVerification::new(
        "hidden cargo test",
        true,
    )];
    let report = curator.run(&[trace]).unwrap();

    let applied = report.apply_to_store(&store, "proj").await.unwrap();

    assert!(applied.memories > 0);
    let hits = store
        .recall(
            "correction",
            &jfc_knowledge::RecallFilter {
                project_key: Some("proj"),
                limit: 5,
            },
        )
        .await
        .unwrap();
    assert!(!hits.is_empty());
}

#[tokio::test]
async fn context_playbook_persists_with_experience_graph_metadata_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    let curator = RsiCurator::new(RsiCuratorConfig::default(), RsiPromotionPolicy::default());
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::UserCorrected);
    trace.user_correction = Some("actually inspect the current file".to_owned());
    trace.verifications = vec![crate::rsi_curator::RsiVerification::new("cargo test", true)];
    let report = curator.run(&[trace]).unwrap();

    let applied = report.apply_to_store(&store, "proj").await.unwrap();

    assert!(applied.definitions > 0);
    let playbook_change = report
        .candidates
        .iter()
        .find(|candidate| candidate.kind == crate::rsi_curator::CandidateKind::ContextPlaybookPatch)
        .unwrap();
    let playbook = store
        .get_definition_by_name(
            "context_playbook",
            jfc_knowledge::DefinitionScope::Project,
            Some("proj"),
            None,
            &playbook_change.definition_name(),
        )
        .await
        .unwrap()
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&playbook.metadata_json).unwrap();
    assert_eq!(
        metadata["rsi"]["experience_graph"]["candidate_node"],
        format!("candidate:{}", playbook_change.id)
    );
    assert!(
        metadata["rsi"]["experience_graph"]["nodes"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert!(
        metadata["rsi"]["experience_graph"]["edges"]
            .as_u64()
            .unwrap()
            > 0
    );
}

#[tokio::test]
async fn reasoning_policy_persists_private_cot_distillation_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    let curator = RsiCurator::new(RsiCuratorConfig::default(), RsiPromotionPolicy::default());
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::Succeeded);
    trace.thinking_tokens = 1_200;
    trace.thinking_blocks = vec!["raw hidden reasoning".to_owned()];
    trace.verifications = vec![crate::rsi_curator::RsiVerification::new("cargo test", true)];
    let report = curator.run(&[trace]).unwrap();

    let applied = report.apply_to_store(&store, "proj").await.unwrap();

    assert!(applied.definitions > 0);
    let policy_change = report
        .candidates
        .iter()
        .find(|candidate| candidate.kind == CandidateKind::ReasoningPolicy)
        .unwrap();
    let policy = store
        .get_definition_by_name(
            "reasoning_policy",
            jfc_knowledge::DefinitionScope::Project,
            Some("proj"),
            None,
            &policy_change.definition_name(),
        )
        .await
        .unwrap()
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&policy.metadata_json).unwrap();
    assert_eq!(
        metadata["rsi"]["eval"]["research"]["profile"],
        "reasoning_process"
    );
    assert_eq!(metadata["rsi"]["thinking"]["raw_stored"], false);
    assert_eq!(metadata["rsi"]["control"]["capability"], "reasoning_write");
    assert!(!policy.body.contains("raw hidden reasoning"));
}
