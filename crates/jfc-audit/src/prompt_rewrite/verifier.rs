//! Stage 4 — intent-preservation verifier.
//!
//! Gates every proposed rewrite before it can be shown to the user. Intent
//! preservation is BOTH the quality property and the safety property
//! (arXiv:2503.16789, arXiv:2511.19009): a rewrite that changes the goal gives
//! the wrong answer AND could launder a harmful intent. The verifier asks the
//! model two yes/no questions:
//!
//! 1. Does the rewrite still ask for the same legitimate thing? (entailment)
//! 2. Did the rewrite introduce any new harmful meaning?
//!
//! A rewrite is emitted only if (1) holds and (2) does not. Otherwise the
//! original prompt passes through unchanged (annotated). A parse failure is
//! treated as "not verified" — fail-safe toward keeping the original.

use async_trait::async_trait;
use serde::Deserialize;

use super::types::{PromptStage, RewriteContext, StageOutcome};
use crate::error::Result;

const SYSTEM: &str = "You verify a proposed rewrite of a user prompt. Compare the ORIGINAL \
and the REWRITE. Output ONLY a JSON object: {\"intent_preserved\": boolean, \
\"introduced_harm\": boolean, \"confidence\": number 0..1, \"reason\": string}. \
intent_preserved is true only if the rewrite asks for the same legitimate goal as the \
original. introduced_harm is true if the rewrite adds any harmful meaning the original did \
not have. confidence is how sure you are that the rewrite preserves intent without adding \
harm.";

#[derive(Debug, Deserialize)]
struct RawVerdict {
    intent_preserved: Option<bool>,
    introduced_harm: Option<bool>,
    confidence: Option<f64>,
}

/// Whether a verifier response approves the rewrite at acceptance threshold τ.
/// Fail-safe: missing/garbled fields count as not-preserved / harm-introduced.
/// An elicited confidence below τ is rejected even when the booleans pass, so
/// τ tunes precision/recall (Confidence Elicitation, arXiv:2306.13063).
pub fn verdict_approves(text: &str, threshold: f64) -> bool {
    let Some(json) = super::classifier_json(text) else {
        return false;
    };
    let Ok(raw) = serde_json::from_str::<RawVerdict>(json) else {
        return false;
    };
    let preserved = raw.intent_preserved.unwrap_or(false);
    let harmless = !raw.introduced_harm.unwrap_or(true);
    // Confidence defaults to 1.0 when the model omits it, so a model that
    // doesn't emit the field still works against the booleans alone.
    let confident = raw.confidence.unwrap_or(1.0).clamp(0.0, 1.0) >= threshold;
    preserved && harmless && confident
}

/// The Stage-4 [`PromptStage`].
pub struct Verifier;

#[async_trait]
impl PromptStage for Verifier {
    fn name(&self) -> &'static str {
        "verifier"
    }

    async fn run(&self, ctx: &mut RewriteContext<'_>) -> Result<StageOutcome> {
        // No proposal (clarity rewrite declined / parse failed) → pass original.
        let Some(proposal) = ctx.proposal.clone() else {
            return Ok(StageOutcome::Pass);
        };
        let user = format!("ORIGINAL:\n{}\n\nREWRITE:\n{}", ctx.original, proposal.text);
        let raw = ctx.model.complete(SYSTEM, &user).await?;
        if verdict_approves(&raw, ctx.threshold) {
            tracing::debug!(
                target: "jfc::prompt_rewrite",
                stage = "verifier",
                threshold = ctx.threshold,
                "rewrite ACCEPTED (intent preserved, no harm introduced)"
            );
            // Keep the proposal in the context; the pipeline emits it.
            Ok(StageOutcome::Continue)
        } else {
            tracing::debug!(
                target: "jfc::prompt_rewrite",
                stage = "verifier",
                threshold = ctx.threshold,
                "rewrite REJECTED (failed intent/harm/confidence check) — keeping original"
            );
            // Reject the rewrite; fall back to the original prompt.
            ctx.proposal = None;
            Ok(StageOutcome::Pass)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{Rewrite, RewriteModel};
    use super::*;

    const TAU: f64 = super::super::types::DEFAULT_THRESHOLD;

    #[test]
    fn approves_preserved_and_harmless() {
        assert!(verdict_approves(
            r#"{"intent_preserved":true,"introduced_harm":false,"reason":"ok"}"#,
            TAU
        ));
    }

    #[test]
    fn rejects_intent_change() {
        assert!(!verdict_approves(
            r#"{"intent_preserved":false,"introduced_harm":false}"#,
            TAU
        ));
    }

    #[test]
    fn rejects_introduced_harm() {
        assert!(!verdict_approves(
            r#"{"intent_preserved":true,"introduced_harm":true}"#,
            TAU
        ));
    }

    #[test]
    fn parse_failure_rejects() {
        assert!(!verdict_approves("the model rambled with no json", TAU));
        assert!(!verdict_approves(r#"{"intent_preserved":true}"#, TAU)); // missing harm → assume harm
    }

    #[test]
    fn low_confidence_rejected_at_high_threshold() {
        let v = r#"{"intent_preserved":true,"introduced_harm":false,"confidence":0.4}"#;
        assert!(verdict_approves(v, 0.3)); // τ below confidence → accept
        assert!(!verdict_approves(v, 0.9)); // τ above confidence → reject
    }

    fn proposal() -> Rewrite {
        Rewrite {
            original_intent: "i".into(),
            risk_flags: vec![],
            text: "rewritten text".into(),
            rationale: "r".into(),
        }
    }

    fn ctx_with<'a>(
        model: &'a dyn RewriteModel,
        ex: &'a [Rewrite],
        proposal: Option<Rewrite>,
    ) -> RewriteContext<'a> {
        let mut ctx = RewriteContext::new("original prompt", model, ex);
        ctx.proposal = proposal;
        ctx
    }

    struct Canned(&'static str);
    #[async_trait]
    impl RewriteModel for Canned {
        async fn complete(&self, _: &str, _: &str) -> Result<String> {
            Ok(self.0.to_string())
        }
    }

    #[tokio::test]
    async fn no_proposal_passes_original() {
        let model = Canned("");
        let ex: Vec<Rewrite> = Vec::new();
        let mut ctx = ctx_with(&model, &ex, None);
        assert_eq!(Verifier.run(&mut ctx).await.unwrap(), StageOutcome::Pass);
        assert!(ctx.proposal.is_none());
    }

    #[tokio::test]
    async fn approved_rewrite_is_kept() {
        let model = Canned(r#"{"intent_preserved":true,"introduced_harm":false}"#);
        let ex: Vec<Rewrite> = Vec::new();
        let mut ctx = ctx_with(&model, &ex, Some(proposal()));
        assert_eq!(
            Verifier.run(&mut ctx).await.unwrap(),
            StageOutcome::Continue
        );
        assert!(ctx.proposal.is_some());
    }

    #[tokio::test]
    async fn rejected_rewrite_falls_back_to_original() {
        let model = Canned(r#"{"intent_preserved":false,"introduced_harm":false}"#);
        let ex: Vec<Rewrite> = Vec::new();
        let mut ctx = ctx_with(&model, &ex, Some(proposal()));
        assert_eq!(Verifier.run(&mut ctx).await.unwrap(), StageOutcome::Pass);
        assert!(ctx.proposal.is_none());
    }
}
