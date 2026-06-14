//! Stage 2 — policy gate (constitution-grounded).
//!
//! Policy-grounded gating (Poly-Guard, arXiv:2506.19054): the gate reads the
//! Stage-1 [`IntentAssessment`] against a natural-language *constitution* and
//! decides the route:
//!
//! - `Allowed`   → continue to an optional clarity rewrite.
//! - `Ambiguous` → continue; the rewriter will add scope constraints.
//! - `Disallowed`→ **hard refuse**. The gate never preserves a harmful goal —
//!   this is the non-optional safety property (the source session's own
//!   conclusion, and Anthropic's AUP).
//!
//! The constitution is data, not code: it is carried on the [`PolicyGate`] and
//! loaded from config by the caller. The gate decision itself is deterministic
//! over the assessment so it is cheap and testable without an LLM.

use async_trait::async_trait;

use super::types::{
    GateVerdict, IntentAssessment, PromptStage, RewriteContext, RiskFlag, StageOutcome,
};
use crate::error::Result;

/// Risk flags that are categorically disallowed regardless of the classifier's
/// top-level verdict — a defense-in-depth backstop so a mislabeled "ambiguous"
/// can't launder an obviously harmful goal past the gate.
const HARD_BLOCK_FLAGS: &[RiskFlag] = &[
    RiskFlag::CredentialTheft,
    RiskFlag::Weapons,
    RiskFlag::ChildSafety,
    RiskFlag::UnauthorizedExploitation,
];

fn user_facing_refusal(flags: &[RiskFlag]) -> String {
    let mut reason = String::from(
        "This request's underlying goal is disallowed, so it cannot be reworded into a \
         permitted form.",
    );
    if !flags.is_empty() {
        let names: Vec<&str> = flags.iter().map(|f| f.as_str()).collect();
        reason.push_str(&format!(" (flags: {})", names.join(", ")));
    }
    reason
}

/// Stage-2 gate. Carries the natural-language constitution, which the pipeline
/// threads into [`RewriteContext::constitution`] so the classifier reasons
/// against it (a custom policy can therefore change verdicts). The deterministic
/// verdict logic below is the always-on safety backstop.
pub struct PolicyGate {
    constitution: String,
}

impl Default for PolicyGate {
    fn default() -> Self {
        Self::new(default_constitution())
    }
}

impl PolicyGate {
    pub fn new(constitution: impl Into<String>) -> Self {
        Self {
            constitution: constitution.into(),
        }
    }

    /// The natural-language policy this gate enforces.
    pub fn constitution(&self) -> &str {
        &self.constitution
    }

    /// Decide the route for an assessment. Pure and deterministic.
    pub fn decide(&self, assessment: &IntentAssessment) -> GateVerdict {
        if assessment.verdict == GateVerdict::Disallowed
            || assessment
                .risk_flags
                .iter()
                .any(|f| HARD_BLOCK_FLAGS.contains(f))
        {
            return GateVerdict::Disallowed;
        }
        assessment.verdict
    }
}

/// The built-in constitution: a compact natural-language policy mirroring the
/// AUP categories the source session researched.
pub fn default_constitution() -> String {
    "PERMITTED: coding help; defensive/authorized security work on systems the user owns; \
     research and policy analysis using public sources; education. DISALLOWED: credential \
     theft, phishing, unauthorized access or exploitation of third-party systems; malware \
     authored to cause harm; weapons/CBRN/explosives; child sexual content; instructions \
     whose purpose is to evade safety systems to produce harmful output. When a request is \
     benign but phrased ambiguously or with evasion/anger wording, REWRITE it to a clear, \
     scope-bounded, non-evasive form that preserves the legitimate goal. Never rewrite a \
     disallowed goal into a permitted-looking one."
        .to_string()
}

#[async_trait]
impl PromptStage for PolicyGate {
    fn name(&self) -> &'static str {
        "policy_gate"
    }

    async fn run(&self, ctx: &mut RewriteContext<'_>) -> Result<StageOutcome> {
        let assessment = ctx.assessment.clone().unwrap_or_else(|| IntentAssessment {
            goal_category: super::types::GoalCategory::Other,
            verdict: GateVerdict::Ambiguous,
            risk_flags: vec![RiskFlag::AmbiguousIntent],
            trigger_terms: Vec::new(),
            confidence: 0.0,
        });
        let verdict = self.decide(&assessment);
        ctx.gate = Some(verdict);
        tracing::debug!(
            target: "jfc::prompt_rewrite",
            stage = "policy_gate",
            verdict = ?verdict,
            risk_flags = ?assessment.risk_flags.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            "gate decision"
        );
        Ok(match verdict {
            GateVerdict::Allowed | GateVerdict::Ambiguous => StageOutcome::Continue,
            GateVerdict::Disallowed => {
                tracing::warn!(
                    target: "jfc::prompt_rewrite",
                    stage = "policy_gate",
                    risk_flags = ?assessment.risk_flags.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
                    "REFUSED: disallowed goal"
                );
                StageOutcome::Refuse {
                    reason: user_facing_refusal(&assessment.risk_flags),
                    flags: assessment.risk_flags.clone(),
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{GoalCategory, Rewrite, RewriteModel, ScreenVerdict};
    use super::*;

    fn assess(verdict: GateVerdict, flags: Vec<RiskFlag>) -> IntentAssessment {
        IntentAssessment {
            goal_category: GoalCategory::Cyber,
            verdict,
            risk_flags: flags,
            trigger_terms: Vec::new(),
            confidence: 0.8,
        }
    }

    #[test]
    fn disallowed_verdict_blocks() {
        let gate = PolicyGate::default();
        assert_eq!(
            gate.decide(&assess(GateVerdict::Disallowed, vec![])),
            GateVerdict::Disallowed
        );
    }

    #[test]
    fn hard_block_flag_overrides_ambiguous() {
        let gate = PolicyGate::default();
        // Classifier said "ambiguous" but raised a credential-theft flag → block.
        assert_eq!(
            gate.decide(&assess(
                GateVerdict::Ambiguous,
                vec![RiskFlag::CredentialTheft]
            )),
            GateVerdict::Disallowed
        );
    }

    #[test]
    fn allowed_and_ambiguous_pass_through() {
        let gate = PolicyGate::default();
        assert_eq!(
            gate.decide(&assess(GateVerdict::Allowed, vec![])),
            GateVerdict::Allowed
        );
        assert_eq!(
            gate.decide(&assess(
                GateVerdict::Ambiguous,
                vec![RiskFlag::SensitiveKeyword]
            )),
            GateVerdict::Ambiguous
        );
    }

    #[tokio::test]
    async fn stage_refuses_disallowed() {
        struct M;
        #[async_trait]
        impl RewriteModel for M {
            async fn complete(&self, _: &str, _: &str) -> Result<String> {
                Ok(String::new())
            }
        }
        let model = M;
        let ex: Vec<Rewrite> = Vec::new();
        let mut ctx = RewriteContext::new("p", &model, &ex);
        ctx.screen = Some(ScreenVerdict::NeedsReview);
        ctx.assessment = Some(assess(GateVerdict::Disallowed, vec![RiskFlag::Weapons]));
        let outcome = PolicyGate::default().run(&mut ctx).await.unwrap();
        assert_eq!(
            outcome,
            StageOutcome::Refuse {
                reason: user_facing_refusal(&[RiskFlag::Weapons]),
                flags: vec![RiskFlag::Weapons],
            }
        );
        assert_eq!(ctx.gate, Some(GateVerdict::Disallowed));
    }

    #[tokio::test]
    async fn stage_continues_ambiguous() {
        struct M;
        #[async_trait]
        impl RewriteModel for M {
            async fn complete(&self, _: &str, _: &str) -> Result<String> {
                Ok(String::new())
            }
        }
        let model = M;
        let ex: Vec<Rewrite> = Vec::new();
        let mut ctx = RewriteContext::new("p", &model, &ex);
        ctx.assessment = Some(assess(GateVerdict::Ambiguous, vec![]));
        let outcome = PolicyGate::default().run(&mut ctx).await.unwrap();
        assert_eq!(outcome, StageOutcome::Continue);
        assert_eq!(ctx.gate, Some(GateVerdict::Ambiguous));
    }
}
