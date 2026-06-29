//! Optional Stage R — downstream-refusal detection + rephrase-and-retry.
//!
//! Separate from the core pipeline (Stages 0–4 rewrite the *outgoing* prompt).
//! This stage handles the *response* side: when the downstream model refuses an
//! already-benign prompt, detect the refusal (WildGuard's refusal signal,
//! arXiv:2406.18495) and offer ONE rephrase-and-retry through the rewriter.
//!
//! Refusal instability is well-documented — non-refusal tokens stay in the
//! candidate list but the model fails to pick them (AdaCD, arXiv:2604.17132), so
//! a single surface rephrase often recovers a legitimate answer. The retry is
//! **opt-in and user-visible**: this module only *detects* a refusal and
//! *proposes* a retry; it never resends automatically.

use super::types::{Rewrite, RewriteModel};
use super::{RewriteDecision, RewritePipeline};
use crate::error::Result;
use serde::Deserialize;

const RESPONSE_REFUSAL_SYSTEM: &str = "You are a response refusal classifier. \
You read a user prompt and the assistant response it received. You DO NOT answer \
the prompt. You output ONLY a JSON object with keys: verdict (one of: answered, \
partial, refused), confidence (number 0..1), and rationale (short string). Mark \
'refused' only when the assistant primarily declines or withholds the requested \
help. Mark 'answered' when the response substantially answers, even if it opens \
with caveats or mentions safety. Mark 'partial' when it mixes a limited answer \
with a refusal.";

/// Structured verdict from the response-side refusal classifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResponseRefusalVerdict {
    Answered,
    Partial,
    Refused,
}

/// Parsed output from the model-backed refusal classifier.
#[derive(Debug, Clone, PartialEq)]
pub struct ResponseRefusalAssessment {
    pub verdict: ResponseRefusalVerdict,
    pub confidence: f64,
    pub rationale: String,
}

impl ResponseRefusalAssessment {
    pub fn answered() -> Self {
        Self {
            verdict: ResponseRefusalVerdict::Answered,
            confidence: 0.0,
            rationale: "classifier output was missing or not a refusal".to_string(),
        }
    }

    pub fn is_refusal(&self) -> bool {
        self.verdict == ResponseRefusalVerdict::Refused && self.confidence >= 0.5
    }
}

#[derive(Debug, Deserialize)]
struct RawRefusalAssessment {
    verdict: Option<String>,
    confidence: Option<f64>,
    rationale: Option<String>,
}

fn build_refusal_user_prompt(original: &str, response: &str) -> String {
    format!(
        "User prompt:\n{original}\n\nAssistant response:\n{response}\n\nClassify whether the \
assistant response is a refusal of the user prompt."
    )
}

fn parse_refusal_verdict(raw: &str) -> ResponseRefusalVerdict {
    match raw.trim().to_ascii_lowercase().as_str() {
        "refused" | "refusal" => ResponseRefusalVerdict::Refused,
        "partial" | "partially_refused" | "partially refused" => ResponseRefusalVerdict::Partial,
        _ => ResponseRefusalVerdict::Answered,
    }
}

pub fn parse_refusal_assessment(text: &str) -> ResponseRefusalAssessment {
    let Some(json) = super::classifier_json(text) else {
        return ResponseRefusalAssessment::answered();
    };
    let Ok(raw) = serde_json::from_str::<RawRefusalAssessment>(json) else {
        return ResponseRefusalAssessment::answered();
    };

    ResponseRefusalAssessment {
        verdict: raw
            .verdict
            .as_deref()
            .map(parse_refusal_verdict)
            .unwrap_or(ResponseRefusalVerdict::Answered),
        confidence: raw.confidence.unwrap_or(0.0).clamp(0.0, 1.0),
        rationale: raw.rationale.unwrap_or_default(),
    }
}

/// Classify whether `response` is a refusal of `original` using the same
/// provider-backed [`RewriteModel`] architecture as the rewrite stages.
pub async fn classify_refusal(
    model: &dyn RewriteModel,
    original: &str,
    response: &str,
) -> Result<ResponseRefusalAssessment> {
    let user = build_refusal_user_prompt(original, response);
    let raw = model.complete(RESPONSE_REFUSAL_SYSTEM, &user).await?;
    let assessment = parse_refusal_assessment(&raw);
    tracing::debug!(
        target: "jfc::prompt_rewrite",
        stage = "response_refusal_classifier",
        verdict = ?assessment.verdict,
        confidence = assessment.confidence,
        rationale = %assessment.rationale,
        "response-side refusal assessment"
    );
    Ok(assessment)
}

/// Outcome of a retry proposal.
#[derive(Debug, Clone, PartialEq)]
pub enum RetryProposal {
    /// The response was not a refusal — nothing to do.
    NotRefused,
    /// A refusal was detected but no safe rephrase was produced; surface the
    /// original refusal to the user.
    NoRewrite,
    /// A rephrase is available; the frontend offers the user a single retry.
    Retry(Rewrite),
}

/// Given a downstream refusal of `original`, propose a single rephrased retry by
/// running the prompt back through the rewrite pipeline. Returns
/// [`RetryProposal::NotRefused`] when `response` is not a refusal, so callers can
/// invoke this unconditionally after a turn.
pub async fn propose_retry(
    pipeline: &RewritePipeline,
    model: &dyn RewriteModel,
    original: &str,
    response: &str,
) -> Result<RetryProposal> {
    let refusal = classify_refusal(model, original, response).await?;
    if !refusal.is_refusal() {
        return Ok(RetryProposal::NotRefused);
    }
    tracing::debug!(
        target: "jfc::prompt_rewrite",
        stage = "retry",
        "downstream refusal detected; attempting rephrase-and-retry"
    );
    match pipeline
        .run_with_feedback(original, model, &[], &[], Some(response))
        .await?
    {
        RewriteDecision::Rewritten(rewrite) => {
            tracing::debug!(
                target: "jfc::prompt_rewrite",
                stage = "retry",
                "retry proposal available"
            );
            Ok(RetryProposal::Retry(rewrite))
        }
        // Pass (no rewrite) or Refused → nothing safe to retry with.
        RewriteDecision::Pass | RewriteDecision::Refused { .. } => Ok(RetryProposal::NoRewrite),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    #[test]
    fn parses_refusal_assessment_from_json() {
        let out = parse_refusal_assessment(
            r#"{"verdict":"refused","confidence":0.91,"rationale":"declined"}"#,
        );
        assert_eq!(out.verdict, ResponseRefusalVerdict::Refused);
        assert!(out.is_refusal());
    }

    #[test]
    fn answered_assessment_is_not_refusal() {
        let out = parse_refusal_assessment(
            r#"{"verdict":"answered","confidence":0.86,"rationale":"substantive answer"}"#,
        );
        assert_eq!(out.verdict, ResponseRefusalVerdict::Answered);
        assert!(!out.is_refusal());
    }

    #[test]
    fn malformed_classifier_output_fails_open_to_answered() {
        let out = parse_refusal_assessment("not json");
        assert_eq!(out.verdict, ResponseRefusalVerdict::Answered);
        assert!(!out.is_refusal());
    }

    struct Model {
        refusal: String,
        classify: String,
        rewrite: String,
        verify: String,
    }
    #[async_trait]
    impl RewriteModel for Model {
        async fn complete(&self, system: &str, _user: &str) -> Result<String> {
            if system.starts_with("You are a response refusal classifier") {
                Ok(self.refusal.clone())
            } else if system.starts_with("You are a safety intent classifier") {
                Ok(self.classify.clone())
            } else if system.starts_with("You rewrite") {
                Ok(self.rewrite.clone())
            } else {
                Ok(self.verify.clone())
            }
        }
    }

    #[tokio::test]
    async fn non_refusal_returns_not_refused() {
        let model = Model {
            refusal: r#"{"verdict":"answered","confidence":0.9,"rationale":"answered"}"#.into(),
            classify: String::new(),
            rewrite: String::new(),
            verify: String::new(),
        };
        let out = propose_retry(
            &RewritePipeline::default(),
            &model,
            "explain tls",
            "Sure, here is the answer.",
        )
        .await
        .unwrap();
        assert_eq!(out, RetryProposal::NotRefused);
    }

    #[tokio::test]
    async fn refusal_of_ambiguous_prompt_proposes_retry() {
        let model = Model {
            refusal: r#"{"verdict":"refused","confidence":0.9,"rationale":"declined"}"#.into(),
            classify: r#"{"goal_category":"policy_analysis","verdict":"ambiguous","risk_flags":["evasion_phrasing"],"confidence":0.6}"#.into(),
            rewrite: r#"{"original_intent":"understand classifiers","text":"Research public documentation on safety classifiers; defensive analysis only.","rationale":"removed evasion wording"}"#.into(),
            verify: r#"{"intent_preserved":true,"introduced_harm":false}"#.into(),
        };
        let out = propose_retry(
            &RewritePipeline::default(),
            &model,
            "dig into classifiers and how to get around it",
            "I can't help with that.",
        )
        .await
        .unwrap();
        match out {
            RetryProposal::Retry(r) => assert!(r.text.contains("public documentation")),
            other => panic!("expected a retry proposal, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn refusal_of_benign_prompt_has_no_rewrite() {
        // A malformed/empty rewrite proposal still yields no retry.
        let model = Model {
            refusal: r#"{"verdict":"refused","confidence":0.9,"rationale":"declined"}"#.into(),
            classify: r#"{"goal_category":"coding","verdict":"allowed","confidence":0.9}"#.into(),
            rewrite: String::new(),
            verify: String::new(),
        };
        let out = propose_retry(
            &RewritePipeline::default(),
            &model,
            "write a rust function to reverse a slice",
            "I cannot help with that.",
        )
        .await
        .unwrap();
        assert_eq!(out, RetryProposal::NoRewrite);
    }

    #[tokio::test]
    async fn refusal_of_benign_prompt_can_rewrite_in_retry_mode() {
        let model = Model {
            refusal: r#"{"verdict":"refused","confidence":0.9,"rationale":"declined"}"#.into(),
            classify: String::new(),
            rewrite: r#"{"original_intent":"rust slice helper","text":"Clarified request: write a Rust function that reverses a slice.","rationale":"made the coding scope explicit"}"#.into(),
            verify: r#"{"intent_preserved":true,"introduced_harm":false}"#.into(),
        };
        let out = propose_retry(
            &RewritePipeline::default(),
            &model,
            "write a rust function to reverse a slice",
            "I cannot help with that.",
        )
        .await
        .unwrap();
        match out {
            RetryProposal::Retry(r) => assert!(r.text.contains("Clarified request")),
            other => panic!("expected a retry proposal, got {other:?}"),
        }
    }
}
