//! Content-aware self-critique over the assistant's OWN reasoning + output.
//!
//! The existing RSI miner is observable-event driven (tool errors, user
//! corrections) and the thinking analysis ([`crate::rsi_curator::analyze_thinking`])
//! reads only *metrics* — token counts, outcome — never the reasoning content.
//! This module is the missing *source*: it looks at the actual chain-of-thought
//! + visible output + outcome of a past turn and emits structured improvement
//! proposals targeting reasoning quality and output technique, not just
//! failures. This is the "Claude improves Claude" critique.
//!
//! The model call is injected (like the verifier): callers pass a
//! [`CritiqueJudge`]. [`HeuristicJudge`] ships as the always-on, model-free
//! baseline so the loop produces value with zero token cost; an LLM-backed judge
//! augments it. Proposals reuse the existing [`CandidateKind`] taxonomy so they
//! flow into the same candidate → gate → apply pipeline.

use crate::rsi_curator::CandidateKind;

/// One past turn reduced to what a critic needs. Built from the persisted
/// transcript: `reasoning` comes from the CoT read-back primitive
/// (`jfc_engine::session_message_reasoning`), the rest from the trace.
#[derive(Debug, Clone, Default)]
pub struct TurnSample {
    pub session_id: String,
    pub seq: i64,
    /// The assistant's chain-of-thought for this turn, if any.
    pub reasoning: Option<String>,
    /// The assistant's visible output text.
    pub output: String,
    /// A user correction immediately followed this turn (soft-failure signal).
    pub followed_by_correction: bool,
    /// The turn ended with a tool failure that was never recovered.
    pub had_unrecovered_error: bool,
    /// Size of the reasoning, for "deliberated a lot but still failed" signals.
    pub thinking_chars: usize,
}

/// A single improvement the critic proposes for FUTURE turns. Maps directly onto
/// a [`CandidateKind`] so it can become a `CandidateChange` downstream.
#[derive(Debug, Clone, PartialEq)]
pub struct ImprovementProposal {
    pub kind: CandidateKind,
    pub title: String,
    /// The actionable policy/lesson to apply going forward.
    pub body: String,
    /// Why — the evidence excerpt from the critiqued turn.
    pub evidence: String,
    pub source_session_id: String,
    pub source_seq: i64,
    /// Critic confidence in `0.0..=1.0`.
    pub confidence: f64,
}

/// Where a failed turn went wrong — the shared failure-localization taxonomy.
///
/// The Fan-talk "diagnose the misconception" move: a wrong answer isn't just
/// wrong, it's wrong *at a step* and *for a reason*. The same vocabulary tags
/// self-critique proposals AND eval error-pattern signatures (jfc-knowledge's
/// `record_eval_error_signature`), so "what kind of mistake" is one language
/// across the whole self-improvement loop — which makes error *distributions*,
/// not just pass-rates, comparable across variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailureKind {
    /// Misread / didn't observe the ground truth (assumed instead of checking).
    Perception,
    /// Had the facts but drew the wrong inference / plan.
    Reasoning,
    /// Lacked a fact it needed and didn't go acquire it.
    KnowledgeGap,
    /// Reached a result but reported it without confirming.
    Verification,
    /// A failure that doesn't localize to the above.
    Other,
}

impl FailureKind {
    /// Stable snake_case token used as an eval error-pattern `signature` and in
    /// persisted critique records. Must stay in lockstep with the buckets the
    /// eval harness groups on.
    pub fn signature(self) -> &'static str {
        match self {
            FailureKind::Perception => "perception",
            FailureKind::Reasoning => "reasoning",
            FailureKind::KnowledgeGap => "knowledge_gap",
            FailureKind::Verification => "verification",
            FailureKind::Other => "other",
        }
    }
}

/// Localize the dominant failure of a turn, or `None` when there's no failure
/// signal. Mirrors the [`HeuristicJudge`] rules: a hedge that preceded a
/// correction is a *perception* miss (assumed instead of observing); a
/// premature done/verified claim over an unresolved error is a *verification*
/// miss; heavy deliberation that still failed is a *reasoning* miss; any other
/// unrecovered error / correction is `Other` (didn't localize).
pub fn classify_failure(s: &TurnSample) -> Option<FailureKind> {
    let reasoning_lc = s.reasoning.as_deref().unwrap_or("").to_lowercase();
    let output_lc = s.output.to_lowercase();
    if s.followed_by_correction && HEDGES.iter().any(|h| reasoning_lc.contains(h)) {
        return Some(FailureKind::Perception);
    }
    if s.had_unrecovered_error && DONE_CLAIMS.iter().any(|c| output_lc.contains(c)) {
        return Some(FailureKind::Verification);
    }
    if s.had_unrecovered_error && s.thinking_chars > 4_000 {
        return Some(FailureKind::Reasoning);
    }
    if s.had_unrecovered_error || s.followed_by_correction {
        return Some(FailureKind::Other);
    }
    None
}

/// Pluggable critic. The engine can inject an LLM-backed implementation;
/// [`HeuristicJudge`] is the deterministic baseline.
pub trait CritiqueJudge {
    fn critique(&self, sample: &TurnSample) -> Vec<ImprovementProposal>;
}

/// Deterministic, model-free critic. Catches the high-signal reasoning/output
/// anti-patterns detectable without an LLM, so the loop always produces
/// something; an LLM judge augments rather than replaces it.
pub struct HeuristicJudge;

/// Reasoning hedges that, when they precede a correction, signal an unverified
/// assumption the agent should have checked.
const HEDGES: &[&str] = &[
    "i assume",
    "assuming",
    "probably",
    "should be",
    "i think it",
    "likely ",
    "presumably",
    "i'll guess",
];

/// Output phrases that claim completion/verification.
const DONE_CLAIMS: &[&str] = &[
    "done",
    "fixed",
    "verified",
    "tested",
    "all passing",
    "works now",
];

impl CritiqueJudge for HeuristicJudge {
    fn critique(&self, s: &TurnSample) -> Vec<ImprovementProposal> {
        let mut out = Vec::new();
        let reasoning = s.reasoning.as_deref().unwrap_or("");
        let reasoning_lc = reasoning.to_lowercase();
        let output_lc = s.output.to_lowercase();

        // 1) Unverified assumption that preceded a correction → reasoning policy.
        if s.followed_by_correction && HEDGES.iter().any(|h| reasoning_lc.contains(h)) {
            out.push(ImprovementProposal {
                kind: CandidateKind::ReasoningPolicy,
                title: "Verify cheap-to-check assumptions before acting".to_owned(),
                body: "When the reasoning hedges ('assume' / 'probably' / 'should be') on a \
                       fact that is cheap to confirm, confirm it (read the file, run the \
                       command) before acting — this turn assumed and was then corrected."
                    .to_owned(),
                evidence: excerpt(reasoning),
                source_session_id: s.session_id.clone(),
                source_seq: s.seq,
                confidence: 0.6,
            });
        }

        // 2) Heavy deliberation that still hit an unrecovered error → front-load
        //    an observable check (complements analyze_thinking's metric rule).
        if s.had_unrecovered_error && s.thinking_chars > 4_000 {
            out.push(ImprovementProposal {
                kind: CandidateKind::ReasoningPolicy,
                title: "Front-load an observable check on hard turns".to_owned(),
                body: "Long deliberation preceded an unrecovered failure; insert an early \
                       observable check (read / grep / build) to ground the plan before \
                       spending more reasoning budget."
                    .to_owned(),
                evidence: format!(
                    "{} chars of reasoning, ended in an unrecovered error",
                    s.thinking_chars
                ),
                source_session_id: s.session_id.clone(),
                source_seq: s.seq,
                confidence: 0.55,
            });
        }

        // 3) Output claimed done/verified while the turn still had an unresolved
        //    error → output-technique patch (the "verify before reporting" rule).
        if s.had_unrecovered_error && DONE_CLAIMS.iter().any(|c| output_lc.contains(c)) {
            out.push(ImprovementProposal {
                kind: CandidateKind::SystemPromptPatch,
                title: "Don't claim done/verified without successful evidence".to_owned(),
                body: "The output reported completion/verification while the turn had an \
                       unresolved error. Only report done/verified after a check actually \
                       succeeded."
                    .to_owned(),
                evidence: excerpt(&s.output),
                source_session_id: s.session_id.clone(),
                source_seq: s.seq,
                confidence: 0.65,
            });
        }

        out
    }
}

/// Run a judge over a batch of turns, returning all proposals.
pub fn critique_turns(
    judge: &dyn CritiqueJudge,
    samples: &[TurnSample],
) -> Vec<ImprovementProposal> {
    samples.iter().flat_map(|s| judge.critique(s)).collect()
}

/// First non-empty line, capped, for evidence quotes.
fn excerpt(text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("")
        .chars()
        .take(160)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> TurnSample {
        TurnSample {
            session_id: "ses_1".to_owned(),
            seq: 3,
            ..Default::default()
        }
    }

    #[test]
    fn flags_unverified_assumption_before_correction_regression() {
        let s = TurnSample {
            reasoning: Some("I assume the config key is camelCase here.".to_owned()),
            followed_by_correction: true,
            ..sample()
        };
        let props = HeuristicJudge.critique(&s);
        assert!(
            props
                .iter()
                .any(|p| p.kind == CandidateKind::ReasoningPolicy && p.title.contains("Verify")),
            "expected a verify-assumptions reasoning policy, got {props:?}"
        );
    }

    #[test]
    fn no_proposal_when_assumption_was_correct_normal() {
        // Same hedge, but no correction followed → don't propose.
        let s = TurnSample {
            reasoning: Some("I assume the config key is camelCase here.".to_owned()),
            followed_by_correction: false,
            ..sample()
        };
        assert!(HeuristicJudge.critique(&s).is_empty());
    }

    #[test]
    fn classify_failure_localizes_buckets_normal() {
        let perception = TurnSample {
            reasoning: Some("I assume the key is camelCase.".to_owned()),
            followed_by_correction: true,
            ..sample()
        };
        assert_eq!(classify_failure(&perception), Some(FailureKind::Perception));

        let verification = TurnSample {
            output: "Done, tests pass.".to_owned(),
            had_unrecovered_error: true,
            ..sample()
        };
        assert_eq!(
            classify_failure(&verification),
            Some(FailureKind::Verification)
        );

        let reasoning = TurnSample {
            thinking_chars: 9_000,
            had_unrecovered_error: true,
            ..sample()
        };
        assert_eq!(classify_failure(&reasoning), Some(FailureKind::Reasoning));

        // A clean turn has no failure signal.
        assert_eq!(classify_failure(&sample()), None);
        assert_eq!(FailureKind::Reasoning.signature(), "reasoning");
    }

    #[test]
    fn flags_premature_done_claim_with_unresolved_error_regression() {
        let s = TurnSample {
            output: "Done — the build is passing now.".to_owned(),
            had_unrecovered_error: true,
            ..sample()
        };
        let props = HeuristicJudge.critique(&s);
        assert!(
            props
                .iter()
                .any(|p| p.kind == CandidateKind::SystemPromptPatch),
            "expected a don't-claim-done patch, got {props:?}"
        );
    }

    #[test]
    fn flags_heavy_deliberation_that_still_failed_normal() {
        let s = TurnSample {
            thinking_chars: 9_000,
            had_unrecovered_error: true,
            ..sample()
        };
        let props = HeuristicJudge.critique(&s);
        assert!(props.iter().any(|p| p.title.contains("Front-load")));
    }

    #[test]
    fn critique_turns_aggregates_across_samples_normal() {
        let samples = vec![
            TurnSample {
                reasoning: Some("assuming the path exists".to_owned()),
                followed_by_correction: true,
                ..sample()
            },
            TurnSample {
                output: "fixed".to_owned(),
                had_unrecovered_error: true,
                ..sample()
            },
        ];
        assert_eq!(critique_turns(&HeuristicJudge, &samples).len(), 2);
    }
}
