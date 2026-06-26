use super::RsiCuratorConfig;
use super::analysis::ThinkingAnalysis;
use super::budget::{BudgetRecommendation, budget_policy, should_emit_budget};
use super::eval::evaluate_candidate;
use super::fixtures::RsiRegressionFixture;
use super::gate::{CandidateEval, CandidateStatus, RsiPromotionPolicy};
use super::id::candidate_id;
use super::proposals::{
    context_playbook_patch, harness_patch, memory_rule, recovered_tool, skill_draft,
    system_prompt_patch, tool_patch,
};
use super::reasoning::{reasoning_policy, should_emit_reasoning_policy};
use super::trace::{RsiOutcome, RsiTrace, RsiTraceScore};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CandidateKind {
    MemoryRule,
    SkillDraft,
    SystemPromptPatch,
    ToolDefinitionPatch,
    HarnessPatch,
    ContextPlaybookPatch,
    BudgetPolicy,
    ReasoningPolicy,
}

impl CandidateKind {
    pub const fn definition_kind(&self) -> Option<&'static str> {
        match self {
            Self::MemoryRule => None,
            Self::SkillDraft => Some("skill"),
            Self::SystemPromptPatch => Some("system_prompt"),
            Self::ToolDefinitionPatch => Some("tool_definition"),
            Self::HarnessPatch => Some("harness_patch"),
            Self::ContextPlaybookPatch => Some("context_playbook"),
            Self::BudgetPolicy => Some("budget_policy"),
            Self::ReasoningPolicy => Some("reasoning_policy"),
        }
    }

    pub const fn slug(&self) -> &'static str {
        match self {
            Self::MemoryRule => "memory_rule",
            Self::SkillDraft => "skill_draft",
            Self::SystemPromptPatch => "system_prompt_patch",
            Self::ToolDefinitionPatch => "tool_definition_patch",
            Self::HarnessPatch => "harness_patch",
            Self::ContextPlaybookPatch => "context_playbook_patch",
            Self::BudgetPolicy => "budget_policy",
            Self::ReasoningPolicy => "reasoning_policy",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingSource {
    None,
    PrivateReasoningDerived,
}

impl ThinkingSource {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PrivateReasoningDerived => "private_reasoning_derived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingSupport {
    None,
    PrivateOnly,
    ObservableSignals,
}

impl ThinkingSupport {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::PrivateOnly => "private_only",
            Self::ObservableSignals => "observable_signals",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThinkingSelfConsistency {
    Untested,
    SingleSignal,
    CrossChecked,
    ConflictObserved,
}

impl ThinkingSelfConsistency {
    pub const fn slug(self) -> &'static str {
        match self {
            Self::Untested => "untested",
            Self::SingleSignal => "single_signal",
            Self::CrossChecked => "cross_checked",
            Self::ConflictObserved => "conflict_observed",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThinkingProvenance {
    pub source: ThinkingSource,
    pub private_blocks_seen: usize,
    pub thinking_tokens: u64,
    pub raw_stored: bool,
    pub support: ThinkingSupport,
    pub self_consistency: ThinkingSelfConsistency,
    pub observable_support_count: u8,
}

impl ThinkingProvenance {
    pub fn from_trace(trace: &RsiTrace) -> Self {
        let source = if trace.thinking_blocks.is_empty() && trace.thinking_tokens == 0 {
            ThinkingSource::None
        } else {
            ThinkingSource::PrivateReasoningDerived
        };
        let observable_support_count = observable_support_count(trace);
        let support = match (source, observable_support_count) {
            (ThinkingSource::None, _) => ThinkingSupport::None,
            (ThinkingSource::PrivateReasoningDerived, 0) => ThinkingSupport::PrivateOnly,
            (ThinkingSource::PrivateReasoningDerived, _) => ThinkingSupport::ObservableSignals,
        };
        let self_consistency = if trace
            .verifications
            .iter()
            .any(|verification| !verification.passed)
            || matches!(
                trace.outcome,
                Some(RsiOutcome::Failed | RsiOutcome::UserCorrected)
            ) {
            ThinkingSelfConsistency::ConflictObserved
        } else if observable_support_count >= 2 {
            ThinkingSelfConsistency::CrossChecked
        } else if observable_support_count == 1 {
            ThinkingSelfConsistency::SingleSignal
        } else {
            ThinkingSelfConsistency::Untested
        };
        Self {
            source,
            private_blocks_seen: trace.thinking_blocks.len(),
            thinking_tokens: trace.thinking_tokens,
            raw_stored: false,
            support,
            self_consistency,
            observable_support_count,
        }
    }

    pub const fn has_observable_support(self) -> bool {
        self.observable_support_count > 0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateTarget {
    pub kind: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CandidateChange {
    pub id: String,
    pub kind: CandidateKind,
    pub target: CandidateTarget,
    pub title: String,
    pub body: String,
    pub evidence: String,
    pub source_session_id: String,
    pub source_turn_id: Option<String>,
    pub score: f64,
    pub recurrence_count: i64,
    pub eval: CandidateEval,
    pub status: CandidateStatus,
    pub budget: Option<BudgetRecommendation>,
    pub thinking: ThinkingProvenance,
}

impl CandidateChange {
    pub fn evaluate(
        &mut self,
        policy: &RsiPromotionPolicy,
        fixtures: &[RsiRegressionFixture],
        trace: &RsiTrace,
    ) {
        self.eval = evaluate_candidate(self, fixtures, trace);
        self.status = policy.status_for(&self.id, &self.eval, self.recurrence_count);
    }

    pub fn definition_name(&self) -> String {
        if self.status == CandidateStatus::Active {
            return self.target.name.clone();
        }
        format!("rsi-{}-{}", self.kind.slug(), &self.id[..12])
    }

    pub fn absorb_recurrence(&mut self, other: &Self) {
        self.recurrence_count += other.recurrence_count;
        self.score = self.score.max(other.score);
        self.eval.score = self.eval.score.max(other.eval.score);
        if !self.evidence.contains(&other.evidence) {
            self.evidence.push('\n');
            self.evidence.push_str(&other.evidence);
        }
    }

    pub fn refresh_status(&mut self, policy: &RsiPromotionPolicy) {
        self.status = policy.status_for(&self.id, &self.eval, self.recurrence_count);
    }
}

pub fn generate_candidates(
    trace: &RsiTrace,
    score: &RsiTraceScore,
    config: &RsiCuratorConfig,
    analysis: &ThinkingAnalysis,
) -> Vec<CandidateChange> {
    let mut out = Vec::new();
    if trace.user_correction.is_some() || matches!(trace.outcome, Some(RsiOutcome::UserCorrected)) {
        out.push(memory_rule(trace, score, &analysis.lesson));
        out.push(system_prompt_patch(trace, score));
    }
    if let Some(tool) = recovered_tool(&trace.tool_steps) {
        out.push(tool_patch(trace, score, &tool));
        out.push(harness_patch(trace, score, analysis, Some(&tool)));
    } else if trace.user_correction.is_some()
        || matches!(
            trace.outcome,
            Some(RsiOutcome::UserCorrected | RsiOutcome::Failed)
        )
    {
        out.push(harness_patch(trace, score, analysis, None));
    }
    if score.tool_success_rate >= 0.99
        && matches!(trace.outcome, Some(RsiOutcome::Succeeded))
        && trace.tool_steps.len() >= 2
    {
        out.push(skill_draft(trace, score));
    }
    if should_emit_budget(trace, score) {
        out.push(budget_policy(trace, score));
    }
    if should_emit_reasoning_policy(trace) {
        out.push(reasoning_policy(trace, score, analysis));
    }
    if should_emit_playbook(trace) {
        out.push(context_playbook_patch(trace, score, analysis));
    }
    out.retain(|candidate| candidate.score >= config.min_candidate_score);
    out
}

fn should_emit_playbook(trace: &RsiTrace) -> bool {
    trace.user_correction.is_some()
        || !trace.verifications.is_empty()
        || !trace.retrieval_steps.is_empty()
        || !trace.selections.is_empty()
        || trace.agent_fanouts.iter().any(|fanout| fanout.count > 1)
        || trace.tool_steps.iter().any(|step| !step.success)
}

pub(super) fn candidate(
    trace: &RsiTrace,
    kind: CandidateKind,
    target: CandidateTarget,
    title: impl Into<String>,
    body: String,
    score: f64,
    budget: Option<BudgetRecommendation>,
) -> CandidateChange {
    let title = title.into();
    let evidence = trace_evidence(trace);
    let id = candidate_id(kind.slug(), &target.kind, &target.name, &body);
    CandidateChange {
        id,
        kind,
        target,
        title,
        body,
        evidence,
        source_session_id: trace.session_id.clone(),
        source_turn_id: trace.turn_id.clone(),
        score: score.clamp(0.0, 1.0),
        recurrence_count: 1,
        eval: CandidateEval::reject(0.0, "not evaluated"),
        status: CandidateStatus::Candidate,
        budget,
        thinking: ThinkingProvenance::from_trace(trace),
    }
}

fn trace_evidence(trace: &RsiTrace) -> String {
    let thinking = ThinkingProvenance::from_trace(trace);
    format!(
        "session={} turn={} thinking_blocks={} thinking_tokens={} thinking_support={} self_consistency={} observable_support={} tools={} verifications={} retrievals={} fanouts={} selections={} correction={}",
        trace.session_id,
        trace.turn_id.as_deref().unwrap_or(""),
        trace.thinking_blocks.len(),
        trace.thinking_tokens,
        thinking.support.slug(),
        thinking.self_consistency.slug(),
        thinking.observable_support_count,
        trace.tool_steps.len(),
        trace.verifications.len(),
        trace.retrieval_steps.len(),
        trace.agent_fanouts.len(),
        trace.selections.len(),
        trace.user_correction.is_some()
    )
}

fn observable_support_count(trace: &RsiTrace) -> u8 {
    let mut count = 0u8;
    if trace
        .verifications
        .iter()
        .any(|verification| verification.passed)
    {
        count += 1;
    }
    if trace.tool_steps.iter().any(|step| step.success) {
        count += 1;
    }
    if trace
        .retrieval_steps
        .iter()
        .any(|step| step.result_count > 0)
    {
        count += 1;
    }
    if trace.agent_fanouts.iter().any(|fanout| fanout.count > 1)
        && trace
            .selections
            .iter()
            .any(|selection| selection.winner.is_some())
    {
        count += 1;
    }
    if trace.user_correction.is_some() || matches!(trace.outcome, Some(RsiOutcome::UserCorrected)) {
        count += 1;
    }
    count
}

#[cfg(test)]
mod tests;
