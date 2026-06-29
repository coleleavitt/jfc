use crate::rsi_curator::ApplyToStore;
use crate::rsi_curator::{
    RsiCurator, RsiCuratorConfig, RsiOutcome, RsiPromotionPolicy, RsiToolStep, RsiTrace,
};

fn recovered_tool_trace() -> RsiTrace {
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::Succeeded);
    trace.tool_steps = vec![
        RsiToolStep::new("Edit", false),
        RsiToolStep::new("Read", true),
        RsiToolStep::new("Edit", true),
    ];
    trace.verifications = vec![crate::rsi_curator::RsiVerification::new("cargo test", true)];
    trace.thinking_tokens = 2_000;
    trace.thinking_blocks = vec!["raw private thinking never written to metadata".to_owned()];
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

#[tokio::test]
async fn candidate_definition_keeps_prior_rollback_metadata_normal() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    store
        .upsert_definition(&existing_tool_definition())
        .await
        .unwrap();
    let curator = RsiCurator::new(RsiCuratorConfig::default(), RsiPromotionPolicy::default());
    let report = curator.run(&[recovered_tool_trace()]).unwrap();

    let applied = report.apply_to_store(&store, "proj").await.unwrap();

    assert!(applied.learning_events > 0);
    assert!(applied.definitions > 0);
    assert_eq!(applied.experiment_jobs, 1);
    assert_eq!(applied.experiment_loop_states, 1);
    let job = store
        .get_definition_by_name(
            "rsi_experiment_job",
            jfc_knowledge::DefinitionScope::Project,
            Some("proj"),
            None,
            "current",
        )
        .await
        .unwrap()
        .unwrap();
    let job_metadata: serde_json::Value = serde_json::from_str(&job.metadata_json).unwrap();
    assert_eq!(
        job_metadata["rsi"]["experiment_job"]["preflight"]["status"],
        "ready"
    );
    assert_eq!(
        job_metadata["rsi"]["experiment_job"]["sandbox"]["egress_policy"],
        "deny_by_default"
    );
    assert_eq!(
        job_metadata["rsi"]["experiment_job"]["sandbox"]["status"],
        "in_process_only"
    );
    assert_eq!(
        job_metadata["rsi"]["experiment_job"]["sandbox"]["execution_mode"],
        "in_process_curator"
    );
    assert_eq!(
        job_metadata["rsi"]["experiment_job"]["sandbox"]["kernel_enforced"],
        false
    );
    assert_eq!(
        job_metadata["rsi"]["experiment_job"]["external_worker_sandbox"]["status"],
        "blocked"
    );
    assert_eq!(
        job_metadata["rsi"]["experiment_job"]["external_worker_sandbox"]["reasons"][0],
        "kernel_sandbox_receipt_missing"
    );
    let state = crate::rsi_curator::load_experiment_loop_state(&store, "proj")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.run_count, 1);
    assert_eq!(state.preflight_status, "ready");
    let candidate_change = report
        .candidates
        .iter()
        .find(|candidate| candidate.kind == crate::rsi_curator::CandidateKind::ToolDefinitionPatch)
        .unwrap();
    let candidate = store
        .get_definition_by_name(
            "tool_definition",
            jfc_knowledge::DefinitionScope::Project,
            Some("proj"),
            None,
            &candidate_change.definition_name(),
        )
        .await
        .unwrap()
        .unwrap();
    let metadata: serde_json::Value = serde_json::from_str(&candidate.metadata_json).unwrap();
    assert_eq!(metadata["rsi"]["prior"]["version"], 1);
    assert_eq!(
        metadata["rsi"]["thinking"]["source"],
        "private_reasoning_derived"
    );
    assert_eq!(metadata["rsi"]["thinking"]["raw_stored"], false);
    assert_eq!(
        metadata["rsi"]["control"]["capability"],
        "tool_definition_write"
    );
    assert_eq!(metadata["rsi"]["control"]["trust"], "verified");
    assert_eq!(metadata["rsi"]["control"]["activation_status"], "candidate");
    assert_eq!(metadata["rsi"]["control"]["approval_required"], true);
    assert_eq!(
        metadata["rsi"]["eval"]["research"]["profile"],
        "tool_definition_control"
    );
    assert_eq!(metadata["rsi"]["eval"]["research"]["verified"], true);
    assert!(
        metadata["rsi"]["eval"]["research"]["lineage"]
            .as_array()
            .unwrap()
            .iter()
            .any(|reference| reference["paper_id"] == "2601.08012")
    );
    assert_eq!(
        metadata["rsi"]["rollback"]["action"],
        "restore_prior_definition"
    );
    assert_eq!(
        candidate.status,
        crate::rsi_curator::CandidateStatus::Candidate.slug()
    );
}

#[tokio::test]
async fn blocked_experiment_job_persists_state_without_candidate_mutation_robust() {
    let store = jfc_knowledge::KnowledgeStore::open_in_memory()
        .await
        .unwrap();
    let curator = RsiCurator::new(RsiCuratorConfig::default(), RsiPromotionPolicy::default());
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::UserCorrected);
    trace.user_correction = Some("actually verify first".to_owned());
    trace.thinking_tokens = 1_000;
    let report = curator.run(&[trace]).unwrap();

    let applied = report.apply_to_store(&store, "proj").await.unwrap();

    assert_eq!(applied.learning_events, 0);
    assert_eq!(applied.definitions, 0);
    assert_eq!(applied.experiment_jobs, 1);
    assert_eq!(applied.experiment_loop_states, 1);
    let state = crate::rsi_curator::load_experiment_loop_state(&store, "proj")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state.preflight_status, "blocked");
    assert_eq!(state.candidate_actions, 0);
    assert!(state.candidates_seen > 0);
}
