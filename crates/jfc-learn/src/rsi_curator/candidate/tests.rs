use super::*;
use crate::rsi_curator::trace::{
    RsiAgentFanout, RsiOutcome, RsiRetrievalStep, RsiSelectionEvent, RsiTrace,
};

#[test]
fn candidates_do_not_copy_raw_thinking_normal() {
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::UserCorrected);
    trace.user_correction = Some("actually use cargo test".into());
    trace.thinking_blocks = vec!["raw private chain of thought".into()];
    let score = crate::rsi_curator::score_trace(&trace);

    let analysis = crate::rsi_curator::analyze_thinking(&trace, &score);
    let candidates = generate_candidates(&trace, &score, &RsiCuratorConfig::default(), &analysis);

    assert!(!candidates.is_empty());
    assert!(
        candidates
            .iter()
            .all(|candidate| !candidate.body.contains("raw private chain of thought"))
    );
}

#[test]
fn candidate_id_accumulates_across_sessions_normal() {
    let mut a = RsiTrace::new("s1");
    a.outcome = Some(RsiOutcome::UserCorrected);
    a.user_correction = Some("actually use cargo test".into());
    let mut b = a.clone();
    b.session_id = "s2".to_owned();
    let score = crate::rsi_curator::score_trace(&a);
    let analysis = crate::rsi_curator::analyze_thinking(&a, &score);

    let first = generate_candidates(&a, &score, &RsiCuratorConfig::default(), &analysis);
    let second = generate_candidates(&b, &score, &RsiCuratorConfig::default(), &analysis);

    assert_eq!(first[0].id, second[0].id);
}

#[test]
fn recovered_tool_trace_generates_harness_patch_without_raw_thinking_normal() {
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::Succeeded);
    trace.thinking_blocks = vec!["private chain-of-thought text".into()];
    trace.tool_steps = vec![
        crate::rsi_curator::RsiToolStep::new("Edit", false),
        crate::rsi_curator::RsiToolStep::new("Read", true),
        crate::rsi_curator::RsiToolStep::new("Edit", true),
    ];
    trace.verifications = vec![crate::rsi_curator::RsiVerification::new(
        "cargo test -p jfc-learn",
        true,
    )];
    let score = crate::rsi_curator::score_trace(&trace);
    let analysis = crate::rsi_curator::analyze_thinking(&trace, &score);

    let candidates = generate_candidates(&trace, &score, &RsiCuratorConfig::default(), &analysis);
    let patch = candidates
        .iter()
        .find(|candidate| candidate.kind == crate::rsi_curator::CandidateKind::HarnessPatch)
        .expect("harness patch candidate");

    assert!(patch.body.contains("Weakness Mining"));
    assert!(patch.body.contains("Harness Proposal"));
    assert!(patch.body.contains("Proposal Validation"));
    assert!(patch.body.contains("verify"));
    assert!(!patch.body.contains("private chain-of-thought text"));
}

#[test]
fn thinking_trace_generates_reasoning_policy_without_raw_cot_normal() {
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::Succeeded);
    trace.thinking_tokens = 1_200;
    trace.thinking_blocks = vec!["raw private reasoning should stay hidden".into()];
    trace.tool_steps = vec![crate::rsi_curator::RsiToolStep::new("Bash", true)];
    trace.verifications = vec![crate::rsi_curator::RsiVerification::new(
        "cargo test -p jfc-learn",
        true,
    )];
    let score = crate::rsi_curator::score_trace(&trace);
    let analysis = crate::rsi_curator::analyze_thinking(&trace, &score);

    let candidates = generate_candidates(&trace, &score, &RsiCuratorConfig::default(), &analysis);
    let policy = candidates
        .iter()
        .find(|candidate| candidate.kind == crate::rsi_curator::CandidateKind::ReasoningPolicy)
        .expect("reasoning policy candidate");

    assert_eq!(policy.target.kind, "reasoning_policy");
    assert!(policy.body.contains("Reasoning Process Policy"));
    assert!(policy.body.contains("Reflection Signal"));
    assert!(policy.body.contains("self-consistency"));
    assert!(policy.body.contains("distill"));
    assert!(policy.body.contains("observable verification"));
    assert!(policy.body.contains("never copy private reasoning"));
    assert!(
        !policy
            .body
            .contains("raw private reasoning should stay hidden")
    );
    assert!(
        policy
            .evidence
            .contains("thinking_support=observable_signals")
    );
    assert!(policy.evidence.contains("self_consistency=cross_checked"));
}

#[test]
fn fanout_selection_trace_generates_non_cot_playbook_normal() {
    let mut trace = RsiTrace::new("project:fixture");
    trace.outcome = Some(RsiOutcome::Succeeded);
    trace.thinking_blocks = vec!["private parallel reasoning".into()];
    trace.retrieval_steps = vec![RsiRetrievalStep::new("bounty context", "memory", 2)];
    trace.agent_fanouts = vec![RsiAgentFanout::new("bounty", 3, true)];
    trace.selections = vec![RsiSelectionEvent::new(
        "bounty",
        Some("solver_a".to_owned()),
        Some(3),
    )];
    let score = crate::rsi_curator::score_trace(&trace);
    let analysis = crate::rsi_curator::analyze_thinking(&trace, &score);

    let candidates = generate_candidates(&trace, &score, &RsiCuratorConfig::default(), &analysis);
    let playbook = candidates
        .iter()
        .find(|candidate| candidate.kind == crate::rsi_curator::CandidateKind::ContextPlaybookPatch)
        .expect("context playbook candidate");

    assert!(playbook.body.contains("parallel selection"));
    assert!(playbook.body.contains("retrievals=1"));
    assert!(playbook.body.contains("fanout_agents=3"));
    assert!(playbook.body.contains("selections=1"));
    assert!(playbook.body.contains("observable verification"));
    assert!(!playbook.body.contains("private parallel reasoning"));
}

#[test]
fn private_reasoning_only_policy_is_rejected_by_research_gate_normal() {
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::Succeeded);
    trace.thinking_tokens = 1_200;
    trace.thinking_blocks = vec!["raw private reasoning should not promote".into()];
    let curator = crate::rsi_curator::RsiCurator::new(
        RsiCuratorConfig::default(),
        crate::rsi_curator::RsiPromotionPolicy::auto_activate_verified(),
    );

    let report = curator.run(&[trace]).expect("curator report");
    let policy = report
        .candidates
        .iter()
        .find(|candidate| candidate.kind == crate::rsi_curator::CandidateKind::ReasoningPolicy)
        .expect("reasoning policy candidate");

    assert_eq!(policy.status, crate::rsi_curator::CandidateStatus::Rejected);
    assert!(policy.evidence.contains("thinking_support=private_only"));
    assert!(
        policy
            .eval
            .reason
            .contains("private_reasoning_requires_observable_support")
    );
}
