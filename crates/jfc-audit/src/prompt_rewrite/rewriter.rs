//! Stage 3 — semantic-preserving rewriter.
//!
//! For allowed/ambiguous prompts, restate the legitimate objective in clear,
//! scope-bounded, non-evasive language (Conversational prompt-rewriting,
//! arXiv:2503.16789): strip anger, slang, "bypass/get around/leeway" framing,
//! and irrelevant salient keywords; add explicit scope constraints ("public
//! sources only", "authorized/defensive only", "no evasion"). Prior accepted
//! rewrites are supplied as few-shot exemplars (experience replay).
//!
//! The model returns a JSON object; we parse it defensively and fall back to a
//! null proposal (the pipeline then passes the prompt through unchanged) rather
//! than emit a malformed rewrite.

use async_trait::async_trait;
use serde::Deserialize;

use super::types::{IntentAssessment, PromptStage, Rewrite, RewriteContext, StageOutcome};
use crate::error::Result;

const SYSTEM: &str = "You rewrite a user's prompt to a clear, scope-bounded, non-evasive form \
that PRESERVES the user's legitimate goal. Remove anger, slang, and evasion wording \
('bypass', 'get around', 'leeway'); remove sensitive keywords that aren't essential to the \
goal; add explicit scope constraints such as 'use public sources only', 'defensive and \
authorized analysis only', and 'do not provide evasion tactics'. Do NOT change what the \
user is actually trying to accomplish. If the goal is harmful you must refuse rather than \
launder it. Output ONLY a JSON object: {\"original_intent\": string, \"text\": string, \
\"rationale\": string}.";

#[derive(Debug, Deserialize)]
struct RawRewrite {
    original_intent: Option<String>,
    text: Option<String>,
    rationale: Option<String>,
}

fn build_user_prompt(ctx: &RewriteContext<'_>, assessment: &IntentAssessment) -> String {
    let mut s = String::new();
    if !ctx.exemplars.is_empty() {
        s.push_str("Examples of good rewrites:\n");
        for ex in ctx.exemplars.iter().take(3) {
            s.push_str(&format!(
                "- ORIGINAL_INTENT: {}\n  REWRITE: {}\n",
                ex.original_intent, ex.text
            ));
        }
        s.push('\n');
    }
    if !assessment.trigger_terms.is_empty() {
        s.push_str(&format!(
            "Terms that triggered a safety reaction (strip if non-essential): {}\n",
            assessment.trigger_terms.join(", ")
        ));
    }
    s.push_str(&format!(
        "Goal category: {}\n",
        assessment.goal_category.as_str()
    ));
    s.push_str("Prompt to rewrite:\n");
    s.push_str(ctx.original);
    s
}

/// Parse the model's rewrite JSON into a [`Rewrite`], carrying the assessment's
/// risk flags. Returns `None` on parse failure or an empty rewrite body.
pub fn parse_rewrite(text: &str, assessment: &IntentAssessment) -> Option<Rewrite> {
    let json = super::classifier_json(text)?;
    let raw: RawRewrite = serde_json::from_str(json).ok()?;
    let body = raw.text.unwrap_or_default();
    if body.trim().is_empty() {
        return None;
    }
    Some(Rewrite {
        original_intent: raw.original_intent.unwrap_or_default(),
        risk_flags: assessment.risk_flags.clone(),
        text: body,
        rationale: raw.rationale.unwrap_or_default(),
    })
}

/// The Stage-3 [`PromptStage`].
pub struct Rewriter;

#[async_trait]
impl PromptStage for Rewriter {
    fn name(&self) -> &'static str {
        "rewriter"
    }

    async fn run(&self, ctx: &mut RewriteContext<'_>) -> Result<StageOutcome> {
        let assessment = ctx
            .assessment
            .clone()
            .unwrap_or_else(|| IntentAssessment::benign(super::types::GoalCategory::Other));
        let user = build_user_prompt(ctx, &assessment);
        let raw = ctx.model.complete(SYSTEM, &user).await?;
        ctx.proposal = parse_rewrite(&raw, &assessment);
        tracing::debug!(
            target: "jfc::prompt_rewrite",
            stage = "rewriter",
            proposed = ctx.proposal.is_some(),
            "rewrite proposal generated"
        );
        // Continue regardless: the verifier (Stage 4) decides whether to emit
        // the proposal or pass the original through.
        Ok(StageOutcome::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{GateVerdict, GoalCategory, RewriteModel, RiskFlag, ScreenVerdict};
    use super::*;

    fn assessment() -> IntentAssessment {
        IntentAssessment {
            goal_category: GoalCategory::PolicyAnalysis,
            verdict: GateVerdict::Ambiguous,
            risk_flags: vec![RiskFlag::EvasionPhrasing],
            trigger_terms: vec!["get around".into()],
            confidence: 0.7,
        }
    }

    #[test]
    fn parses_rewrite_and_carries_flags() {
        let rw = parse_rewrite(
            r#"{"original_intent":"understand classifiers","text":"Research public docs on safety classifiers; no evasion tactics.","rationale":"removed 'get around'"}"#,
            &assessment(),
        )
        .unwrap();
        assert!(rw.text.contains("public docs"));
        assert_eq!(rw.original_intent, "understand classifiers");
        assert_eq!(rw.risk_flags, vec![RiskFlag::EvasionPhrasing]);
    }

    #[test]
    fn empty_rewrite_body_yields_none() {
        assert!(parse_rewrite(r#"{"text":""}"#, &assessment()).is_none());
        assert!(parse_rewrite("not json", &assessment()).is_none());
    }

    #[tokio::test]
    async fn stage_sets_proposal_with_intent_and_rationale() {
        struct M;
        #[async_trait]
        impl RewriteModel for M {
            async fn complete(&self, _: &str, _: &str) -> Result<String> {
                Ok(r#"{"original_intent":"learn classifiers","text":"Research public sources on safety classifiers, defensive analysis only.","rationale":"stripped evasion wording"}"#.into())
            }
        }
        let model = M;
        let ex: Vec<Rewrite> = Vec::new();
        let mut ctx =
            RewriteContext::new("dig into classifiers and how to get around it", &model, &ex);
        ctx.screen = Some(ScreenVerdict::NeedsReview);
        ctx.assessment = Some(assessment());
        let outcome = Rewriter.run(&mut ctx).await.unwrap();
        assert_eq!(outcome, StageOutcome::Continue);
        let p = ctx.proposal.as_ref().unwrap();
        assert!(!p.original_intent.is_empty());
        assert!(!p.rationale.is_empty());
        assert!(p.text.contains("public sources"));
    }

    #[tokio::test]
    async fn exemplars_appear_in_prompt() {
        let ex = vec![Rewrite {
            original_intent: "prior".into(),
            risk_flags: vec![],
            text: "prior rewrite text".into(),
            rationale: "r".into(),
        }];
        let model_check = {
            struct M;
            #[async_trait]
            impl RewriteModel for M {
                async fn complete(&self, _: &str, user: &str) -> Result<String> {
                    assert!(user.contains("prior rewrite text"));
                    Ok(r#"{"text":"ok"}"#.into())
                }
            }
            M
        };
        let mut ctx = RewriteContext::new("p", &model_check, &ex);
        ctx.assessment = Some(assessment());
        Rewriter.run(&mut ctx).await.unwrap();
    }
}
