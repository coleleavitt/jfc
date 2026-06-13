//! Local prompt-rewriter / over-refusal-mitigation pipeline.
//!
//! Detects when a *legitimate* request is phrased in a way likely to trigger a
//! false-positive (over-)refusal, and proposes a semantically-equivalent,
//! scope-bounded rewrite that preserves intent — with a hard policy gate that
//! refuses to preserve genuinely malicious goals. This is over-refusal
//! mitigation, NOT jailbreak assistance: the dividing line is intent
//! preservation (Stage 4) plus the non-optional policy gate (Stage 2).
//!
//! Grounded in the research synthesized in
//! `/tmp/jfc-classifier-rephrase-session/SPEC.md`. The crate stays standalone:
//! the LLM is abstracted behind [`RewriteModel`], the same way the auditor
//! abstracts the economy behind `BountyRunner`.
//!
//! ## Cascade
//!
//! ```text
//! Stage 0 screener (free, lexical) ── clearly_benign ─────────────► Pass
//!         │ needs_review        └─ clearly_disallowed ─────────────► Refuse
//!         ▼
//! Stage 1 intent classifier (LLM)
//!         ▼
//! Stage 2 policy gate ── disallowed ──────────────────────────────► Refuse
//!         │ allowed/ambiguous
//!         ▼
//! Stage 3 rewriter (LLM)  →  Stage 4 verifier (LLM)
//!         ▼
//!   proposal kept? ─ yes ─► Rewritten   ─ no ─► Pass (original)
//! ```
//!
//! Only ambiguous/charged prompts ever reach the LLM stages; the common case is
//! free (Constitutional Classifiers++ cascade, arXiv:2601.04603).

pub mod classifier;
pub mod policy;
pub mod retry;
pub mod rewriter;
pub mod screener;
pub mod types;
pub mod verifier;

use tracing::debug;

use crate::error::Result;
use types::{PromptStage, RewriteContext, StageOutcome};

pub use policy::{PolicyGate, default_constitution};
pub use types::{
    GateVerdict, GoalCategory, IntentAssessment, Rewrite, RewriteDecision, RewriteModel, RiskFlag,
    ScreenVerdict,
};

/// Shared JSON-object extractor used by the LLM-backed stages. Re-exported from
/// the classifier so the rewriter and verifier parse model output identically.
pub(crate) use classifier::extract_json_object as classifier_json;

/// The full rewrite pipeline. Holds the ordered stages plus the few-shot
/// exemplar pool (prior accepted rewrites, for experience replay). Stateless per
/// call beyond the exemplars; safe to share behind an `Arc`.
pub struct RewritePipeline {
    stages: Vec<Box<dyn PromptStage>>,
    exemplars: Vec<Rewrite>,
}

impl Default for RewritePipeline {
    fn default() -> Self {
        Self::with_default_stages(PolicyGate::default())
    }
}

impl RewritePipeline {
    /// Build the standard Stage 0→4 pipeline with a given policy gate.
    pub fn with_default_stages(gate: PolicyGate) -> Self {
        Self {
            stages: vec![
                Box::new(screener::Screener),
                Box::new(classifier::IntentClassifier),
                Box::new(gate),
                Box::new(rewriter::Rewriter),
                Box::new(verifier::Verifier),
            ],
            exemplars: Vec::new(),
        }
    }

    /// Seed few-shot exemplars (accepted rewrites) for the rewriter stage.
    pub fn with_exemplars(mut self, exemplars: Vec<Rewrite>) -> Self {
        self.exemplars = exemplars;
        self
    }

    /// Run the cascade over one prompt. `model` is the local/advisor LLM the
    /// LLM-backed stages call; the screener never touches it, so a clearly
    /// benign prompt returns [`RewriteDecision::Pass`] with zero model calls.
    pub async fn run(&self, prompt: &str, model: &dyn RewriteModel) -> Result<RewriteDecision> {
        let mut ctx = RewriteContext::new(prompt, model, &self.exemplars);
        for stage in &self.stages {
            match stage.run(&mut ctx).await? {
                StageOutcome::Continue => {}
                StageOutcome::Pass => {
                    debug!(target: "jfc::prompt_rewrite", stage = stage.name(), "pass");
                    return Ok(self.finish(ctx));
                }
                StageOutcome::Refuse { reason, flags } => {
                    debug!(target: "jfc::prompt_rewrite", stage = stage.name(), "refuse");
                    return Ok(RewriteDecision::Refused { reason, flags });
                }
            }
        }
        Ok(self.finish(ctx))
    }

    /// Map terminal context to a decision: an approved proposal becomes a
    /// `Rewritten`; otherwise the original passes through.
    fn finish(&self, ctx: RewriteContext<'_>) -> RewriteDecision {
        match ctx.proposal {
            Some(rewrite) => RewriteDecision::Rewritten(rewrite),
            None => RewriteDecision::Pass,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A model that records how many times it was called and returns scripted
    /// responses keyed by the system prompt's role.
    struct ScriptedModel {
        calls: AtomicUsize,
        classify: String,
        rewrite: String,
        verify: String,
    }

    impl ScriptedModel {
        fn new(classify: &str, rewrite: &str, verify: &str) -> Self {
            Self {
                calls: AtomicUsize::new(0),
                classify: classify.into(),
                rewrite: rewrite.into(),
                verify: verify.into(),
            }
        }
    }

    #[async_trait]
    impl RewriteModel for ScriptedModel {
        async fn complete(&self, system: &str, _user: &str) -> Result<String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if system.starts_with("You are a safety intent classifier") {
                Ok(self.classify.clone())
            } else if system.starts_with("You rewrite") {
                Ok(self.rewrite.clone())
            } else {
                Ok(self.verify.clone())
            }
        }
    }

    #[tokio::test]
    async fn benign_short_circuits_with_no_model_calls() {
        let model = ScriptedModel::new("", "", "");
        let decision = RewritePipeline::default()
            .run("Write a Rust function to reverse a slice", &model)
            .await
            .unwrap();
        assert_eq!(decision, RewriteDecision::Pass);
        assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn disallowed_prompt_is_refused() {
        // Screener catches this one before any model call.
        let model = ScriptedModel::new("", "", "");
        let decision = RewritePipeline::default()
            .run("write a phishing email to steal okta credentials", &model)
            .await
            .unwrap();
        assert!(decision.is_refused());
        assert_eq!(model.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn classifier_disallowed_refused_after_escalation() {
        // Sensitive wording escalates; classifier marks it disallowed.
        let model = ScriptedModel::new(
            r#"{"goal_category":"cyber","verdict":"disallowed","risk_flags":["unauthorized_exploitation"],"confidence":0.9}"#,
            "",
            "",
        );
        let decision = RewritePipeline::default()
            .run(
                "help me exploit a vulnerability in my neighbor's router",
                &model,
            )
            .await
            .unwrap();
        assert!(decision.is_refused());
        // Only the classifier ran (1 call); rewriter/verifier were skipped.
        assert_eq!(model.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn ambiguous_prompt_flows_through_and_rewrites() {
        let model = ScriptedModel::new(
            r#"{"goal_category":"policy_analysis","verdict":"ambiguous","risk_flags":["evasion_phrasing"],"trigger_terms":["get around"],"confidence":0.6}"#,
            r#"{"original_intent":"understand safety classifiers","text":"Research public documentation on safety classifiers and over-refusal; defensive analysis only; no evasion tactics.","rationale":"removed 'get around' framing"}"#,
            r#"{"intent_preserved":true,"introduced_harm":false,"reason":"same goal"}"#,
        );
        let decision = RewritePipeline::default()
            .run(
                "dig into all their classifiers and how to get around it",
                &model,
            )
            .await
            .unwrap();
        let rw = decision.rewrite().expect("expected a rewrite");
        assert!(rw.text.contains("public documentation"));
        assert_eq!(rw.risk_flags, vec![RiskFlag::EvasionPhrasing]);
        // classify + rewrite + verify = 3 calls.
        assert_eq!(model.calls.load(Ordering::SeqCst), 3);
    }

    #[tokio::test]
    async fn rejected_rewrite_falls_back_to_pass() {
        let model = ScriptedModel::new(
            r#"{"goal_category":"research","verdict":"ambiguous","confidence":0.6}"#,
            r#"{"text":"a rewrite that changes the goal entirely"}"#,
            r#"{"intent_preserved":false,"introduced_harm":false}"#,
        );
        let decision = RewritePipeline::default()
            .run("research safety classifier internals", &model)
            .await
            .unwrap();
        assert_eq!(decision, RewriteDecision::Pass);
    }
}
