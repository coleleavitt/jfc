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

/// Lexical refusal markers. Cheap first pass before spending an LLM judge call;
/// covers the canonical assistant refusal phrasings.
const REFUSAL_MARKERS: &[&str] = &[
    "i can't help with that",
    "i cannot help with that",
    "i can't assist with that",
    "i cannot assist with that",
    "i'm not able to help",
    "i am not able to help",
    "i won't be able to help",
    "i'm unable to provide",
    "i can't provide",
    "i cannot provide",
    "i'm sorry, but i can't",
    "i'm sorry, but i cannot",
    "i can't comply",
    "i cannot comply",
    "as an ai",
];

/// Heuristic refusal detector over a model response. Conservative: the response
/// must *begin* with a refusal phrase, so a substantive answer that merely
/// *mentions* refusing (e.g. explaining how refusals work) is not misclassified.
pub fn looks_like_refusal(response: &str) -> bool {
    let head = response.trim_start().to_lowercase();
    REFUSAL_MARKERS.iter().any(|m| head.starts_with(m))
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
    if !looks_like_refusal(response) {
        return Ok(RetryProposal::NotRefused);
    }
    match pipeline.run(original, model).await? {
        RewriteDecision::Rewritten(rewrite) => Ok(RetryProposal::Retry(rewrite)),
        // Pass (no rewrite) or Refused → nothing safe to retry with.
        RewriteDecision::Pass | RewriteDecision::Refused { .. } => Ok(RetryProposal::NoRewrite),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;

    #[test]
    fn detects_canonical_refusals() {
        assert!(looks_like_refusal("I can't help with that request."));
        assert!(looks_like_refusal("I'm sorry, but I cannot provide that."));
    }

    #[test]
    fn substantive_answer_is_not_a_refusal() {
        assert!(!looks_like_refusal(
            "Sure! Here is how TLS works: the client and server negotiate..."
        ));
        // Mentions refusal deep in an explanation — not a leading refusal.
        assert!(!looks_like_refusal(
            "Safety classifiers decide when a model should respond that i can't help with that."
        ));
    }

    struct Model {
        classify: String,
        rewrite: String,
        verify: String,
    }
    #[async_trait]
    impl RewriteModel for Model {
        async fn complete(&self, system: &str, _user: &str) -> Result<String> {
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
    async fn non_refusal_returns_not_refused() {
        let model = Model {
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
        // Benign prompt short-circuits to Pass → nothing to retry with.
        let model = Model {
            classify: String::new(),
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
}
