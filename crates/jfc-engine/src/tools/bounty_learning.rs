use std::path::Path;

use super::economy::AppliedSolution;

pub(crate) struct BountyLearningContext<'a> {
    pub cwd: &'a Path,
    pub bounty_id: &'a str,
    pub mode: &'a str,
    pub task_id: Option<&'a str>,
}

pub(crate) async fn persist_bounty_learning(
    ctx: BountyLearningContext<'_>,
    outcome: &jfc_economy::reporting::CycleOutcome,
    written: &AppliedSolution,
) -> Option<jfc_learn::StoreApplyReport> {
    let trace = bounty_trace_from_outcome(&ctx, outcome, written);
    let report = bounty_curator_report(&trace)?;
    let project_key = jfc_knowledge::project_key(ctx.cwd);
    let result = async {
        let store = jfc_knowledge::KnowledgeStore::open_default().await?;
        report.apply_to_store(&store, &project_key).await
    }
    .await;
    match result {
        Ok(applied) => Some(applied),
        Err(error) => {
            tracing::warn!(
                target: "jfc::rsi::bounty",
                bounty_id = %ctx.bounty_id,
                error = %error,
                "failed to persist bounty learning"
            );
            None
        }
    }
}

fn bounty_curator_report(trace: &jfc_learn::RsiTrace) -> Option<jfc_learn::RsiCuratorReport> {
    let curator = jfc_learn::RsiCurator::new(
        jfc_learn::RsiCuratorConfig::default(),
        jfc_learn::RsiPromotionPolicy::default(),
    );
    match curator.run(std::slice::from_ref(trace)) {
        Ok(report) if !report.is_empty() => Some(report),
        Ok(_) => None,
        Err(error) => {
            tracing::warn!(
                target: "jfc::rsi::bounty",
                session_id = %trace.session_id,
                error = %error,
                "failed to curate bounty learning trace"
            );
            None
        }
    }
}

fn bounty_trace_from_outcome(
    ctx: &BountyLearningContext<'_>,
    outcome: &jfc_economy::reporting::CycleOutcome,
    written: &AppliedSolution,
) -> jfc_learn::RsiTrace {
    let mut trace =
        jfc_learn::RsiTrace::new(format!("project:{}", jfc_knowledge::project_key(ctx.cwd)));
    trace.turn_id = Some(format!("bounty:{}", ctx.bounty_id));
    trace.tool_steps = vec![
        jfc_learn::RsiToolStep::new(ctx.mode, true),
        jfc_learn::RsiToolStep::new("bounty.solve", outcome.evidence.solution_count > 0),
        jfc_learn::RsiToolStep::new("bounty.validate", outcome.evidence.validator_count > 0),
        jfc_learn::RsiToolStep::new("bounty.settle", outcome.settlement.winner.is_some()),
        jfc_learn::RsiToolStep::new("apply_winning_solution", !written.files.is_empty()),
    ];
    trace.agent_fanouts = vec![jfc_learn::RsiAgentFanout::new(
        "bounty",
        outcome.evidence.solver_count.max(1),
        outcome.settlement.winner.is_some(),
    )];
    trace.selections = vec![jfc_learn::RsiSelectionEvent::new(
        "bounty",
        outcome
            .settlement
            .winner
            .as_ref()
            .map(|winner| winner.label().to_owned()),
        Some(outcome.evidence.solution_count.max(1)),
    )];
    if let Some(solution) = &outcome.winning_solution {
        if let Some(compiles) = solution.compiles {
            trace.verifications.push(jfc_learn::RsiVerification::new(
                "bounty compile check",
                compiles,
            ));
        }
        if let Some(tests_pass) = solution.tests_pass {
            trace.verifications.push(jfc_learn::RsiVerification::new(
                "bounty test check",
                tests_pass,
            ));
        }
    }
    if let Some(task_id) = ctx.task_id {
        trace.retrieval_steps.push(jfc_learn::RsiRetrievalStep::new(
            format!("task:{task_id}"),
            "task_store",
            1,
        ));
    }
    trace.outcome = Some(if outcome.settlement.winner.is_some() {
        jfc_learn::RsiOutcome::Succeeded
    } else {
        jfc_learn::RsiOutcome::Failed
    });
    trace
}

#[cfg(test)]
mod tests {
    use super::*;
    use jfc_economy::types::{AgentId, Settlement, Solution};

    #[test]
    fn bounty_trace_distills_market_outcome_without_private_cot_normal() {
        let winner = AgentId::from_label("solver_a");
        let outcome = jfc_economy::reporting::CycleOutcome {
            settlement: Settlement {
                bounty_id: "bounty_1".to_owned(),
                winner: Some(winner.clone()),
                payouts: vec![(winner.clone(), 500)],
                trust_updates: vec![(winner.clone(), 5)],
                total_cost: 500,
            },
            winning_solution: Some(Solution {
                agent_id: winner,
                bounty_id: "bounty_1".to_owned(),
                patch: "diff --git a/x b/x".to_owned(),
                explanation: "private-ish solver explanation must not be stored".to_owned(),
                self_assessment: 0.9,
                tokens_consumed: 123,
                compiles: Some(true),
                tests_pass: Some(true),
                suspicious: false,
            }),
            evidence: jfc_economy::reporting::CycleEvidence {
                solver_count: 3,
                solution_count: 2,
                validator_count: 2,
                no_flaw_found: 2,
                ..Default::default()
            },
        };
        let written = AppliedSolution {
            files: vec!["src/lib.rs".into()],
            summary: "wrote one file".to_owned(),
        };
        let ctx = BountyLearningContext {
            cwd: Path::new("/tmp/jfc-bounty-learning"),
            bounty_id: "bounty_1",
            mode: "run_bounty",
            task_id: Some("task_1"),
        };

        let trace = bounty_trace_from_outcome(&ctx, &outcome, &written);
        let report = bounty_curator_report(&trace).expect("curator should emit candidates");

        assert_eq!(trace.thinking_blocks.len(), 0);
        assert_eq!(trace.thinking_tokens, 0);
        assert_eq!(trace.agent_fanouts[0].count, 3);
        assert_eq!(trace.selections[0].selected_from, Some(2));
        assert!(report.candidates.iter().all(|candidate| {
            !candidate.body.contains("diff --git") && !candidate.body.contains("private-ish")
        }));
    }
}
