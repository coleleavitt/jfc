use super::candidate::{CandidateChange, CandidateKind, ThinkingSource};
use super::fixtures::RsiRegressionFixture;
use super::gate::{RsiEvalProfile, RsiResearchCheck, RsiResearchRef};
use super::trace::RsiTrace;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ResearchGateReport {
    pub profile: RsiEvalProfile,
    pub checks: Vec<RsiResearchCheck>,
    pub lineage: Vec<RsiResearchRef>,
}

impl ResearchGateReport {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|check| check.passed)
    }

    pub fn failed_check_names(&self) -> Vec<&'static str> {
        self.checks
            .iter()
            .filter(|check| !check.passed)
            .map(|check| check.name)
            .collect()
    }
}

pub(super) fn assess_research_gate(
    candidate: &CandidateChange,
    fixtures: &[RsiRegressionFixture],
    trace: &RsiTrace,
) -> ResearchGateReport {
    let mut checks = vec![
        RsiResearchCheck::new(
            "private_cot_distilled_not_stored",
            keeps_private_cot_private(candidate, trace),
        ),
        RsiResearchCheck::new("durable_target_bound", has_durable_target(candidate)),
        RsiResearchCheck::new("deterministic_eval_fixture", !fixtures.is_empty()),
        RsiResearchCheck::new(
            "controlled_activation_path",
            has_controlled_activation_path(candidate),
        ),
    ];
    checks.push(RsiResearchCheck::new(
        "kind_specific_improvement_contract",
        kind_contract_holds(candidate),
    ));
    checks.push(RsiResearchCheck::new(
        "private_reasoning_requires_observable_support",
        private_reasoning_has_observable_support(candidate),
    ));

    ResearchGateReport {
        profile: profile_for(candidate.kind.clone()),
        checks,
        lineage: lineage_for(candidate.kind.clone()),
    }
}

fn keeps_private_cot_private(candidate: &CandidateChange, trace: &RsiTrace) -> bool {
    !candidate.thinking.raw_stored
        && !candidate.body.contains("<thinking")
        && !candidate.body.contains("</thinking")
        && trace
            .thinking_blocks
            .iter()
            .all(|block| block.is_empty() || !candidate.body.contains(block))
}

fn private_reasoning_has_observable_support(candidate: &CandidateChange) -> bool {
    candidate.thinking.source != ThinkingSource::PrivateReasoningDerived
        || candidate.thinking.has_observable_support()
}

fn has_durable_target(candidate: &CandidateChange) -> bool {
    !candidate.target.kind.trim().is_empty() && !candidate.target.name.trim().is_empty()
}

fn has_controlled_activation_path(candidate: &CandidateChange) -> bool {
    candidate.kind == CandidateKind::MemoryRule || candidate.kind.definition_kind().is_some()
}

fn kind_contract_holds(candidate: &CandidateChange) -> bool {
    let body = candidate.body.to_ascii_lowercase();
    match candidate.kind {
        CandidateKind::MemoryRule => body.contains("verify") || body.contains("correction"),
        CandidateKind::SkillDraft => {
            body.contains("created-by: rsi-curator") && body.contains("verify")
        }
        CandidateKind::SystemPromptPatch => body.contains("correction") && body.contains("verify"),
        CandidateKind::ToolDefinitionPatch => {
            body.contains(&candidate.target.name.to_ascii_lowercase()) && body.contains("verify")
        }
        CandidateKind::HarnessPatch => {
            body.contains("harness proposal")
                && body.contains("validation")
                && body.contains("rollback")
        }
        CandidateKind::ContextPlaybookPatch => {
            body.contains("retrieval") && body.contains("validation")
        }
        CandidateKind::BudgetPolicy => candidate.budget.is_some() && mentions_budget_control(&body),
        CandidateKind::ReasoningPolicy => {
            body.contains("reasoning")
                && body.contains("observable verification")
                && body.contains("never copy private reasoning")
                && body.contains("self-consistency")
                && body.contains("distill")
        }
    }
}

fn mentions_budget_control(body: &str) -> bool {
    ["budget", "effort", "thinking", "tool"]
        .iter()
        .any(|word| body.contains(word))
}

fn profile_for(kind: CandidateKind) -> RsiEvalProfile {
    match kind {
        CandidateKind::MemoryRule => RsiEvalProfile::ExperienceMemory,
        CandidateKind::SkillDraft => RsiEvalProfile::SkillAcquisition,
        CandidateKind::SystemPromptPatch => RsiEvalProfile::PromptRevision,
        CandidateKind::ToolDefinitionPatch => RsiEvalProfile::ToolDefinitionControl,
        CandidateKind::HarnessPatch => RsiEvalProfile::HarnessSelfImprovement,
        CandidateKind::ContextPlaybookPatch => RsiEvalProfile::ContextPlaybook,
        CandidateKind::BudgetPolicy => RsiEvalProfile::BudgetPolicy,
        CandidateKind::ReasoningPolicy => RsiEvalProfile::ReasoningProcess,
    }
}

fn lineage_for(kind: CandidateKind) -> Vec<RsiResearchRef> {
    let mut refs = vec![
        paper("2312.06942", "ai_control"),
        paper("cs/0309048", "goedel_machine_formal_root"),
        paper("tiling-agents", "tiling_self_trust_formal_root"),
        paper(
            "vingean-reflection",
            "reflection_under_limited_self_knowledge",
        ),
        paper("2510.10232", "self_governance_formal_root"),
    ];
    match kind {
        CandidateKind::MemoryRule => refs.extend([
            paper("2603.20667", "revere_prompt_template_cheatsheet_revision"),
            paper("2605.17721", "experience_graph_to_durable_update"),
            paper("2510.04618", "agent_correction_experience"),
            paper("2510.16079", "experience_driven_evolution"),
        ]),
        CandidateKind::SkillDraft => refs.extend([
            paper("2602.08234", "skill_rl"),
            paper("2510.16079", "experience_driven_evolution"),
            paper("2604.25256", "long_horizon_research_eval"),
        ]),
        CandidateKind::SystemPromptPatch => refs.extend([
            paper("2603.20667", "revere_system_prompt_revision"),
            paper("2510.04618", "agent_correction_experience"),
            paper("2604.25256", "long_horizon_research_eval"),
        ]),
        CandidateKind::ToolDefinitionPatch => refs.extend([
            paper("2601.08012", "verifiably_safe_tool_use"),
            paper("2603.13791", "deceptguard_tool_control"),
            paper("2603.03329", "autoharness"),
            paper("2606.07591", "research_claw_bench"),
        ]),
        CandidateKind::HarnessPatch => refs.extend([
            paper("2603.03329", "autoharness"),
            paper("2606.09498", "self_harness"),
            paper("2603.19461", "hyperagents"),
            paper("2606.12797", "containment_gap"),
            paper("2606.07591", "research_claw_bench"),
        ]),
        CandidateKind::ContextPlaybookPatch => refs.extend([
            paper("2603.20667", "revere_cumulative_cheatsheet"),
            paper("2605.17721", "experience_graph_to_durable_update"),
            paper("2604.25256", "long_horizon_research_eval"),
        ]),
        CandidateKind::BudgetPolicy => refs.extend([
            paper("2603.03329", "autoharness"),
            paper("2606.09498", "self_harness"),
            paper("2606.12797", "containment_gap"),
        ]),
        CandidateKind::ReasoningPolicy => refs.extend([
            paper("2505.05410", "private_reasoning_unfaithfulness_guard"),
            paper("2304.11657", "self_consistency_prompting"),
            paper("2603.20667", "revere_reasoning_revision"),
            paper("2604.25256", "long_horizon_research_eval"),
            paper("2606.07591", "research_claw_bench"),
            paper("2603.13791", "deceptguard_private_reasoning_control"),
        ]),
    }
    refs
}

const fn paper(paper_id: &'static str, role: &'static str) -> RsiResearchRef {
    RsiResearchRef { paper_id, role }
}
