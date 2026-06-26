mod activation;
mod analysis;
mod budget;
mod candidate;
mod control;
mod eval;
mod experience;
mod experiment;
mod extract;
mod fixtures;
mod gate;
mod id;
mod job;
mod loop_plan;
mod loop_state;
mod metadata;
mod proposals;
mod reasoning;
mod research;
mod sandbox_enforcement;
mod store;
mod trace;
mod worker;

pub use activation::{
    RsiActivationAction, RsiActivationReport, RsiDefinitionRef, promote_rsi_definition,
    rollback_rsi_definition,
};
pub use analysis::{ThinkingAnalysis, ThinkingPatternKind, analyze_thinking};
pub use budget::{BudgetRecommendation, ToolVisibilityAction, ToolVisibilityRecommendation};
pub use candidate::{
    CandidateChange, CandidateKind, CandidateTarget, ThinkingProvenance, ThinkingSelfConsistency,
    ThinkingSource, ThinkingSupport, generate_candidates,
};
pub use control::{ControlAssessment, ControlCapability, ControlTrust, assess_control};
pub use eval::evaluate_candidate;
pub use experience::{
    ExperienceEdge, ExperienceEdgeKind, ExperienceGraph, ExperienceNode, ExperienceNodeKind,
    build_experience_graph,
};
pub use experiment::{
    RsiAntiCheatReport, RsiAntiCheatStatus, RsiCostReport, RsiExperimentAction,
    RsiExperimentDashboard, RsiHiddenValidationReport, RsiMetricPoint, RsiPlateauReport,
    RsiPlateauStatus, RsiSandboxReport, build_experiment_dashboard,
};
pub use extract::{
    build_recent_rsi_job, load_recent_traces_from_store, load_trace_from_store, trace_from_messages,
};
pub use fixtures::{RsiRegressionFixture, fixtures_for_candidate};
pub use gate::{
    CandidateEval, CandidateStatus, PromotionMode, RsiEvalProfile, RsiPromotionPolicy,
    RsiResearchCheck, RsiResearchRef,
};
pub use job::{
    RsiExperimentJobSpec, RsiExperimentSchedule, RsiHiddenValidationHarness, RsiJobPreflight,
    RsiJobPreflightStatus, build_experiment_job_spec,
};
pub use loop_plan::{
    RsiExperimentLoopPlan, RsiExperimentPhase, RsiLoopAntiCheatPlan, RsiLoopCostPlan,
    RsiLoopSandboxPlan, RsiLoopValidationPlan, build_experiment_loop_plan,
};
pub use loop_state::{
    RSI_LOOP_STATE_KIND, RSI_LOOP_STATE_NAME, RsiExperimentLoopState, RsiLoopDueDecision,
    build_next_loop_state, current_time_ms, experiment_loop_due_decision,
    load_experiment_loop_state,
};
pub use sandbox_enforcement::{
    RsiSandboxEnforcement, RsiSandboxEnforcementStatus, RsiSandboxExecutionMode,
};
pub use store::StoreApplyReport;
pub use trace::{
    RsiAgentFanout, RsiOutcome, RsiRetrievalStep, RsiSelectionEvent, RsiToolStep, RsiTrace,
    RsiTraceScore, RsiVerification, score_trace,
};
pub use worker::{
    RsiCuratorWorkerConfig, RsiWorkerInput, RsiWorkerOutput, run_rsi_worker_file,
    run_rsi_worker_job,
};

use crate::error::LearnError;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RsiCuratorConfig {
    pub min_candidate_score: f64,
    pub max_candidates_per_trace: usize,
}

impl Default for RsiCuratorConfig {
    fn default() -> Self {
        Self {
            min_candidate_score: 0.55,
            max_candidates_per_trace: 6,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct RsiCuratorReport {
    pub traces_scored: usize,
    pub candidates: Vec<CandidateChange>,
    pub experience_graph: ExperienceGraph,
    pub experiment_dashboard: RsiExperimentDashboard,
    pub experiment_loop: RsiExperimentLoopPlan,
    pub experiment_job: RsiExperimentJobSpec,
}

impl RsiCuratorReport {
    pub fn len(&self) -> usize {
        self.candidates.len()
    }

    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }
}

#[derive(Debug, Clone)]
pub struct RsiCurator {
    config: RsiCuratorConfig,
    policy: RsiPromotionPolicy,
}

#[derive(Debug, Clone)]
pub struct RsiCuratorJob {
    pub traces: Vec<RsiTrace>,
    pub config: RsiCuratorConfig,
    pub promotion_policy: RsiPromotionPolicy,
    pub project_key: Option<String>,
    pub sandbox_enforcement: Option<RsiSandboxEnforcement>,
    pub worker: Option<RsiCuratorWorkerConfig>,
}

impl RsiCurator {
    pub fn new(config: RsiCuratorConfig, policy: RsiPromotionPolicy) -> Self {
        Self { config, policy }
    }

    pub fn run(&self, traces: &[RsiTrace]) -> Result<RsiCuratorReport, LearnError> {
        let mut candidates = Vec::new();
        for trace in traces {
            let score = score_trace(trace);
            let analysis = analyze_thinking(trace, &score);
            let mut generated = generate_candidates(trace, &score, &self.config, &analysis);
            generated.truncate(self.config.max_candidates_per_trace);
            for mut candidate in generated {
                let fixtures = fixtures_for_candidate(trace, &candidate);
                candidate.evaluate(&self.policy, &fixtures, trace);
                if candidate.eval.score >= self.config.min_candidate_score
                    || candidate.status != CandidateStatus::Rejected
                {
                    candidates.push(candidate);
                }
            }
        }
        let candidates = aggregate_candidates(candidates, &self.policy);
        let experience_graph = build_experience_graph(traces, &candidates);
        let experiment_dashboard = build_experiment_dashboard(traces);
        let experiment_loop = build_experiment_loop_plan(&experiment_dashboard);
        let experiment_job = build_experiment_job_spec(&experiment_dashboard, &experiment_loop);
        Ok(RsiCuratorReport {
            traces_scored: traces.len(),
            candidates,
            experience_graph,
            experiment_dashboard,
            experiment_loop,
            experiment_job,
        })
    }
}

fn aggregate_candidates(
    candidates: Vec<CandidateChange>,
    policy: &RsiPromotionPolicy,
) -> Vec<CandidateChange> {
    let mut by_id: BTreeMap<String, CandidateChange> = BTreeMap::new();
    for candidate in candidates {
        by_id
            .entry(candidate.id.clone())
            .and_modify(|existing| existing.absorb_recurrence(&candidate))
            .or_insert(candidate);
    }
    by_id
        .into_values()
        .map(|mut candidate| {
            candidate.refresh_status(policy);
            candidate
        })
        .collect()
}

#[cfg(test)]
mod tests;
