//! Shared types for the prompt-rewrite pipeline.
//!
//! Enums over strings, per the workspace style. The pipeline is a sequence of
//! [`PromptStage`]s that mutate a borrowed [`RewriteContext`]; the public result
//! is a [`RewriteDecision`].

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::Result;

/// The legitimate goal category a prompt is asking about. Taxonomy-grounded,
/// mirroring Llama-Guard-style prompt classification (arXiv:2312.06674).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalCategory {
    Coding,
    Research,
    PolicyAnalysis,
    Cyber,
    Bio,
    Violence,
    Sexual,
    SelfHarm,
    Other,
}

impl GoalCategory {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coding => "coding",
            Self::Research => "research",
            Self::PolicyAnalysis => "policy_analysis",
            Self::Cyber => "cyber",
            Self::Bio => "bio",
            Self::Violence => "violence",
            Self::Sexual => "sexual",
            Self::SelfHarm => "self_harm",
            Self::Other => "other",
        }
    }
}

/// A specific risk signal raised about a prompt. These annotate *why* a prompt
/// was escalated or refused, and what the rewriter should strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RiskFlag {
    /// "bypass / get around / evade / jailbreak" style meta-safety wording.
    EvasionPhrasing,
    /// Anger, slang, or emotional charge that adds no semantic content.
    EmotionalCharge,
    /// Salient sensitive keyword likely driving a keyword-shortcut refusal
    /// (the EVOREFUSE / arXiv:2505.23473 over-weighting failure mode).
    SensitiveKeyword,
    /// Request is ambiguous between a benign and a harmful reading.
    AmbiguousIntent,
    /// Credential theft / phishing / unauthorized-access intent.
    CredentialTheft,
    /// Weapons / CBRN / explosives.
    Weapons,
    /// Child-safety violation.
    ChildSafety,
    /// Unauthorized exploitation of third-party systems.
    UnauthorizedExploitation,
}

impl RiskFlag {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::EvasionPhrasing => "evasion_phrasing",
            Self::EmotionalCharge => "emotional_charge",
            Self::SensitiveKeyword => "sensitive_keyword",
            Self::AmbiguousIntent => "ambiguous_intent",
            Self::CredentialTheft => "credential_theft",
            Self::Weapons => "weapons",
            Self::ChildSafety => "child_safety",
            Self::UnauthorizedExploitation => "unauthorized_exploitation",
        }
    }
}

/// Stage-0 screener triage. The cheap, always-on gate that keeps the common
/// case free (cascade pattern, Constitutional Classifiers++ arXiv:2601.04603).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenVerdict {
    /// Pass through untouched — no LLM cost.
    ClearlyBenign,
    /// Escalate to the intent/risk classifier.
    NeedsReview,
    /// Short-circuit to a refusal.
    ClearlyDisallowed,
}

/// Stage-2 policy-gate verdict over the classifier's reading of intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GateVerdict {
    /// Benign goal; an optional clarity rewrite may still help.
    Allowed,
    /// Benign-but-ambiguous; rewrite and inject scope constraints.
    Ambiguous,
    /// Genuinely harmful goal; refuse — never preserve it.
    Disallowed,
}

/// Structured reading of a prompt produced by the intent/risk classifier
/// (Stage 1). Mirrors Llama-Guard's {category, decision, score} shape plus the
/// trigger terms the rewriter needs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentAssessment {
    pub goal_category: GoalCategory,
    pub verdict: GateVerdict,
    pub risk_flags: Vec<RiskFlag>,
    /// Specific spans/words that likely drove the safety reaction.
    pub trigger_terms: Vec<String>,
    /// Classifier confidence in [0, 1].
    pub confidence: f64,
}

impl IntentAssessment {
    /// A benign, high-confidence assessment with no risk flags — the default
    /// when the cheap screener already cleared the prompt.
    pub fn benign(goal_category: GoalCategory) -> Self {
        Self {
            goal_category,
            verdict: GateVerdict::Allowed,
            risk_flags: Vec::new(),
            trigger_terms: Vec::new(),
            confidence: 1.0,
        }
    }
}

/// A semantic-preserving rewrite plus the rationale to show the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rewrite {
    /// One-line restatement of the legitimate objective.
    pub original_intent: String,
    pub risk_flags: Vec<RiskFlag>,
    /// The rewritten, scope-bounded prompt.
    pub text: String,
    /// Human-readable explanation of what changed and why.
    pub rationale: String,
}

/// The public result of running the pipeline over one prompt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum RewriteDecision {
    /// The prompt is fine as-is; send it unchanged.
    Pass,
    /// A rewrite is proposed. The frontend MUST surface it and let the user
    /// accept/reject/edit — never silently substitute it.
    Rewritten(Rewrite),
    /// The goal is disallowed; do not send. `reason` is user-facing.
    Refused {
        reason: String,
        flags: Vec<RiskFlag>,
    },
}

impl RewriteDecision {
    pub fn is_pass(&self) -> bool {
        matches!(self, Self::Pass)
    }
    pub fn is_refused(&self) -> bool {
        matches!(self, Self::Refused { .. })
    }
    pub fn rewrite(&self) -> Option<&Rewrite> {
        match self {
            Self::Rewritten(r) => Some(r),
            _ => None,
        }
    }
}

/// Abstraction over the local/advisor LLM the rewriter stages call. Keeps
/// `jfc-audit` standalone (no `jfc-provider` dependency) and trivially mockable
/// in tests — the same pattern as [`crate::dispatcher::BountyRunner`].
#[async_trait]
pub trait RewriteModel: Send + Sync {
    /// Complete a single `system` + `user` instruction, returning raw text.
    async fn complete(&self, system: &str, user: &str) -> Result<String>;
}

/// Default intent-preservation acceptance threshold τ. The verifier accepts a
/// rewrite only when its confidence ≥ this; tunable via config (`threshold`).
pub const DEFAULT_THRESHOLD: f64 = 0.5;

/// Mutable state threaded through the stages for one prompt.
pub struct RewriteContext<'a> {
    /// The original user prompt (never mutated).
    pub original: &'a str,
    /// The local model the stages call.
    pub model: &'a dyn RewriteModel,
    /// Stage-0 triage, set by the screener.
    pub screen: Option<ScreenVerdict>,
    /// Stage-1 assessment, set by the classifier.
    pub assessment: Option<IntentAssessment>,
    /// Stage-2 gate verdict.
    pub gate: Option<GateVerdict>,
    /// Stage-3 proposed rewrite.
    pub proposal: Option<Rewrite>,
    /// Few-shot exemplars of prior accepted rewrites (experience replay).
    pub exemplars: &'a [Rewrite],
    /// Preceding conversation turns (oldest→newest), so a prompt benign in
    /// isolation but harmful in context (and vice-versa) is judged correctly
    /// (Defensive M2S arXiv:2601.00454, MTMCS-Bench arXiv:2601.06757). Empty
    /// when the caller supplies no history.
    pub history: &'a [String],
    /// The live natural-language policy the classifier/gate reason against. Lets
    /// a config-supplied constitution actually change verdicts.
    pub constitution: &'a str,
    /// Intent-preservation acceptance threshold τ in [0, 1].
    pub threshold: f64,
}

impl<'a> RewriteContext<'a> {
    pub fn new(original: &'a str, model: &'a dyn RewriteModel, exemplars: &'a [Rewrite]) -> Self {
        Self {
            original,
            model,
            screen: None,
            assessment: None,
            gate: None,
            proposal: None,
            exemplars,
            history: &[],
            constitution: "",
            threshold: DEFAULT_THRESHOLD,
        }
    }

    /// Attach preceding conversation turns for context-aware judging.
    pub fn with_history(mut self, history: &'a [String]) -> Self {
        self.history = history;
        self
    }

    /// Use a specific constitution (config override) instead of the empty default.
    pub fn with_constitution(mut self, constitution: &'a str) -> Self {
        self.constitution = constitution;
        self
    }

    /// Set the intent-preservation acceptance threshold τ.
    pub fn with_threshold(mut self, threshold: f64) -> Self {
        self.threshold = threshold.clamp(0.0, 1.0);
        self
    }

    /// Render the recent conversation context as a compact block for an LLM
    /// stage prompt, or an empty string when there is no history.
    pub fn history_block(&self) -> String {
        if self.history.is_empty() {
            return String::new();
        }
        let recent: Vec<&String> = self.history.iter().rev().take(6).rev().collect();
        let mut s = String::from("Preceding conversation (oldest→newest):\n");
        for turn in recent {
            s.push_str("- ");
            s.push_str(turn);
            s.push('\n');
        }
        s
    }
}

/// What a stage decided about whether the pipeline should keep going.
#[derive(Debug, Clone, PartialEq)]
pub enum StageOutcome {
    /// Continue to the next stage.
    Continue,
    /// Short-circuit: the prompt is fine, send unchanged.
    Pass,
    /// Short-circuit with a refusal.
    Refuse {
        reason: String,
        flags: Vec<RiskFlag>,
    },
}

/// One stage of the rewrite pipeline. Mirrors the [`crate::Guard`]-style shape:
/// each stage owns one focused analysis; the pipeline owns orchestration.
#[async_trait]
pub trait PromptStage: Send + Sync {
    fn name(&self) -> &'static str;
    async fn run(&self, ctx: &mut RewriteContext<'_>) -> Result<StageOutcome>;
}

#[cfg(test)]
mod tests {
    use super::*;

    struct NullModel;
    #[async_trait]
    impl RewriteModel for NullModel {
        async fn complete(&self, _system: &str, _user: &str) -> Result<String> {
            Ok(String::new())
        }
    }

    #[test]
    fn context_constructs_with_empty_state() {
        let model = NullModel;
        let exemplars: Vec<Rewrite> = Vec::new();
        let ctx = RewriteContext::new("hello world", &model, &exemplars);
        assert_eq!(ctx.original, "hello world");
        assert!(ctx.screen.is_none());
        assert!(ctx.assessment.is_none());
        assert!(ctx.gate.is_none());
        assert!(ctx.proposal.is_none());
    }

    #[test]
    fn decision_accessors_normal() {
        assert!(RewriteDecision::Pass.is_pass());
        let refused = RewriteDecision::Refused {
            reason: "no".into(),
            flags: vec![RiskFlag::Weapons],
        };
        assert!(refused.is_refused());
        assert!(refused.rewrite().is_none());
        let rw = RewriteDecision::Rewritten(Rewrite {
            original_intent: "i".into(),
            risk_flags: vec![],
            text: "t".into(),
            rationale: "r".into(),
        });
        assert!(rw.rewrite().is_some());
    }

    #[test]
    fn benign_assessment_has_no_flags() {
        let a = IntentAssessment::benign(GoalCategory::Coding);
        assert_eq!(a.verdict, GateVerdict::Allowed);
        assert!(a.risk_flags.is_empty());
        assert_eq!(a.confidence, 1.0);
    }
}
