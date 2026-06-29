use super::*;

fn corrected_trace() -> RsiTrace {
    let mut trace = RsiTrace::new("s1");
    trace.outcome = Some(RsiOutcome::UserCorrected);
    trace.user_correction = Some("actually verify with cargo test".to_owned());
    trace.thinking_blocks = vec!["raw private reasoning".to_owned()];
    trace
}

#[test]
fn default_policy_keeps_passed_changes_as_candidates_normal() {
    let curator = RsiCurator::new(RsiCuratorConfig::default(), RsiPromotionPolicy::default());

    let report = curator.run(&[corrected_trace()]).unwrap();

    assert!(!report.is_empty());
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.status == CandidateStatus::Candidate)
    );
}

#[test]
fn auto_activate_policy_promotes_verified_candidates_normal() {
    let curator = RsiCurator::new(
        RsiCuratorConfig::default(),
        RsiPromotionPolicy::auto_activate_verified(),
    );

    let report = curator.run(&[corrected_trace()]).unwrap();

    assert!(!report.is_empty());
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| candidate.status == CandidateStatus::Active)
    );
}

#[test]
fn evidence_policy_promotes_recurring_candidate_normal() {
    let curator = RsiCurator::new(
        RsiCuratorConfig::default(),
        RsiPromotionPolicy::evidence_or_approval(2, 70),
    );
    let mut second = corrected_trace();
    second.session_id = "s2".to_owned();

    let report = curator.run(&[corrected_trace(), second]).unwrap();

    assert!(report.candidates.iter().any(|candidate| {
        candidate.recurrence_count == 2 && candidate.status == CandidateStatus::Active
    }));
}

#[test]
fn experiment_dashboard_detects_plateau_and_branches_out_normal() {
    let traces = [
        succeeded_trace("s1", 1_000),
        succeeded_trace("s2", 1_100),
        succeeded_trace("s3", 1_200),
        succeeded_trace("s4", 1_300),
    ];

    let dashboard = build_experiment_dashboard(&traces);

    assert_eq!(dashboard.trace_count, 4);
    assert_eq!(dashboard.plateau.status, RsiPlateauStatus::Plateaued);
    assert_eq!(dashboard.next_action, RsiExperimentAction::BranchOut);
    assert!(dashboard.hidden_validation.required);
    assert_eq!(dashboard.anti_cheat.status, RsiAntiCheatStatus::Protected);
    assert!(dashboard.sandbox.network_blocked);
    assert_eq!(dashboard.sandbox.egress_policy, "deny_by_default");
    assert!(dashboard.cost.estimated_tokens > 0);
}

#[test]
fn experiment_loop_plan_turns_plateau_into_sandboxed_branch_iteration_normal() {
    let traces = [
        succeeded_trace("s1", 1_000),
        succeeded_trace("s2", 1_100),
        succeeded_trace("s3", 1_200),
        succeeded_trace("s4", 1_300),
    ];
    let dashboard = build_experiment_dashboard(&traces);

    let plan = build_experiment_loop_plan(&dashboard);

    assert_eq!(plan.phase, RsiExperimentPhase::Branch);
    assert_eq!(plan.timeout_seconds, 300);
    assert!(plan.commit_required);
    assert_eq!(plan.validation.holdout_name, "hidden-validation-holdout");
    assert!(plan.validation.hidden_holdout_required);
    assert!(
        plan.anti_cheat
            .protected_targets
            .contains(&"score_function")
    );
    assert!(plan.sandbox.network_blocked);
    assert_eq!(plan.sandbox.egress_policy, "deny_by_default");
    assert!(plan.cost.max_next_iteration_tokens > 0);
}

fn succeeded_trace(session: &str, thinking_tokens: u64) -> RsiTrace {
    let mut trace = RsiTrace::new(session);
    trace.outcome = Some(RsiOutcome::Succeeded);
    trace.thinking_tokens = thinking_tokens;
    trace.tool_steps = vec![RsiToolStep::new("Bash", true)];
    trace.verifications = vec![RsiVerification::new("hidden cargo test", true)];
    trace
}
