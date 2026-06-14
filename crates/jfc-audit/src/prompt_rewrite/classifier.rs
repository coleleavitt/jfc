//! Stage 1 — intent + risk classifier (LLM judge).
//!
//! Llama-Guard-shaped (arXiv:2312.06674): a taxonomy-grounded classifier that
//! reads the *intent* behind a prompt and returns a structured
//! [`IntentAssessment`] — goal category, allow/ambiguous/disallow verdict, the
//! trigger terms driving any safety reaction, and a confidence. Only runs when
//! Stage 0 escalated (`NeedsReview`), so the common case never pays for it.
//!
//! The model output is parsed defensively: we extract the first JSON object,
//! tolerate fenced code blocks and surrounding prose, and fall back to a
//! conservative `Ambiguous` assessment if parsing fails (fail-safe: an
//! unparseable judgment must not silently allow).

use async_trait::async_trait;
use serde::Deserialize;

use super::types::{
    GateVerdict, GoalCategory, IntentAssessment, PromptStage, RewriteContext, RiskFlag,
    ScreenVerdict, StageOutcome,
};
use crate::error::Result;

const SYSTEM: &str = "You are a safety intent classifier. You read a user's prompt and \
judge the legitimate goal behind it. You DO NOT answer the prompt. You output ONLY a JSON \
object with keys: goal_category (one of: coding, research, policy_analysis, cyber, bio, \
violence, sexual, self_harm, other), verdict (one of: allowed, ambiguous, disallowed), \
risk_flags (array of: evasion_phrasing, emotional_charge, sensitive_keyword, \
ambiguous_intent, credential_theft, weapons, child_safety, unauthorized_exploitation), \
trigger_terms (array of strings from the prompt that drove any safety reaction), and \
confidence (number 0..1). Mark 'allowed' for benign requests even if they mention \
sensitive topics in a legitimate (defensive, educational, authorized) context. Mark \
'disallowed' only when the underlying goal is genuinely harmful. Judge the goal in light \
of any preceding conversation and the stated policy.";

/// Build the classifier's user message, weaving in the live constitution and
/// recent conversation context when present.
fn build_user_prompt(ctx: &RewriteContext<'_>) -> String {
    let mut s = String::new();
    if !ctx.constitution.trim().is_empty() {
        s.push_str("Policy:\n");
        s.push_str(ctx.constitution.trim());
        s.push_str("\n\n");
    }
    let history = ctx.history_block();
    if !history.is_empty() {
        s.push_str(&history);
        s.push('\n');
    }
    s.push_str("Prompt to classify:\n");
    s.push_str(ctx.original);
    s
}

/// Wire shape of the model's JSON output. Strings (not enums) so an unknown
/// label degrades gracefully instead of failing the whole parse.
#[derive(Debug, Deserialize)]
struct RawAssessment {
    goal_category: Option<String>,
    verdict: Option<String>,
    #[serde(default)]
    risk_flags: Vec<String>,
    #[serde(default)]
    trigger_terms: Vec<String>,
    confidence: Option<f64>,
}

fn parse_goal(s: &str) -> GoalCategory {
    match s.trim().to_lowercase().as_str() {
        "coding" => GoalCategory::Coding,
        "research" => GoalCategory::Research,
        "policy_analysis" => GoalCategory::PolicyAnalysis,
        "cyber" => GoalCategory::Cyber,
        "bio" => GoalCategory::Bio,
        "violence" => GoalCategory::Violence,
        "sexual" => GoalCategory::Sexual,
        "self_harm" => GoalCategory::SelfHarm,
        _ => GoalCategory::Other,
    }
}

fn parse_verdict(s: &str) -> GateVerdict {
    match s.trim().to_lowercase().as_str() {
        "allowed" => GateVerdict::Allowed,
        "disallowed" => GateVerdict::Disallowed,
        // Unknown / "ambiguous" both map to the conservative middle.
        _ => GateVerdict::Ambiguous,
    }
}

fn parse_flag(s: &str) -> Option<RiskFlag> {
    Some(match s.trim().to_lowercase().as_str() {
        "evasion_phrasing" => RiskFlag::EvasionPhrasing,
        "emotional_charge" => RiskFlag::EmotionalCharge,
        "sensitive_keyword" => RiskFlag::SensitiveKeyword,
        "ambiguous_intent" => RiskFlag::AmbiguousIntent,
        "credential_theft" => RiskFlag::CredentialTheft,
        "weapons" => RiskFlag::Weapons,
        "child_safety" => RiskFlag::ChildSafety,
        "unauthorized_exploitation" => RiskFlag::UnauthorizedExploitation,
        _ => return None,
    })
}

/// Tracks string-literal context while scanning JSON bytes, so braces inside
/// string values don't confuse the depth counter. Returns true while inside a
/// string literal.
#[derive(Default)]
struct StrScan {
    in_str: bool,
    escaped: bool,
}

impl StrScan {
    /// Feed one byte; returns true if this byte is *inside* a string literal
    /// (and therefore must not affect brace depth).
    fn consume(&mut self, b: u8) -> bool {
        if self.in_str {
            // Capture escape state BEFORE updating it: a `"` preceded by an
            // unescaped `\` is part of the string, not its terminator.
            let was_escaped = self.escaped;
            self.escaped = !was_escaped && b == b'\\';
            if b == b'"' && !was_escaped {
                self.in_str = false;
            }
            return true;
        }
        if b == b'"' {
            self.in_str = true;
            return true;
        }
        false
    }
}

/// Extract the first balanced `{...}` JSON object from arbitrary model text.
/// Handles fenced blocks and surrounding prose. Shared by the rewriter and
/// verifier stages via `super::classifier_json`.
pub(crate) fn extract_json_object(text: &str) -> Option<&str> {
    let bytes = text.as_bytes();
    let start = bytes.iter().position(|&b| b == b'{')?;
    let mut depth = 0usize;
    let mut scan = StrScan::default();
    for (i, &b) in bytes.iter().enumerate().skip(start) {
        if scan.consume(b) {
            continue;
        }
        if b == b'{' {
            depth += 1;
        } else if b == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(&text[start..=i]);
            }
        }
    }
    None
}

/// Parse raw model text into an [`IntentAssessment`]. Returns a conservative
/// `Ambiguous` assessment on any parse failure (fail-safe).
pub fn parse_assessment(text: &str) -> IntentAssessment {
    let fallback = || IntentAssessment {
        goal_category: GoalCategory::Other,
        verdict: GateVerdict::Ambiguous,
        risk_flags: vec![RiskFlag::AmbiguousIntent],
        trigger_terms: Vec::new(),
        confidence: 0.0,
    };

    let Some(json) = extract_json_object(text) else {
        return fallback();
    };
    let Ok(raw) = serde_json::from_str::<RawAssessment>(json) else {
        return fallback();
    };

    IntentAssessment {
        goal_category: raw
            .goal_category
            .as_deref()
            .map(parse_goal)
            .unwrap_or(GoalCategory::Other),
        verdict: raw
            .verdict
            .as_deref()
            .map(parse_verdict)
            .unwrap_or(GateVerdict::Ambiguous),
        risk_flags: raw
            .risk_flags
            .iter()
            .filter_map(|s| parse_flag(s))
            .collect(),
        trigger_terms: raw.trigger_terms,
        confidence: raw.confidence.unwrap_or(0.0).clamp(0.0, 1.0),
    }
}

/// The Stage-1 [`PromptStage`].
pub struct IntentClassifier;

#[async_trait]
impl PromptStage for IntentClassifier {
    fn name(&self) -> &'static str {
        "intent_classifier"
    }

    async fn run(&self, ctx: &mut RewriteContext<'_>) -> Result<StageOutcome> {
        // Honor a hard screen short-circuit if the orchestrator routed it here.
        if ctx.screen == Some(ScreenVerdict::ClearlyBenign) {
            ctx.assessment = Some(IntentAssessment::benign(GoalCategory::Other));
            return Ok(StageOutcome::Pass);
        }
        let user = build_user_prompt(ctx);
        let raw = ctx.model.complete(SYSTEM, &user).await?;
        let assessment = parse_assessment(&raw);
        tracing::debug!(
            target: "jfc::prompt_rewrite",
            stage = "classifier",
            goal_category = assessment.goal_category.as_str(),
            verdict = ?assessment.verdict,
            risk_flags = ?assessment.risk_flags.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
            confidence = assessment.confidence,
            "intent assessment"
        );
        ctx.assessment = Some(assessment);
        Ok(StageOutcome::Continue)
    }
}

#[cfg(test)]
mod tests {
    use super::super::types::{Rewrite, RewriteModel};
    use super::*;

    struct CannedModel(String);
    #[async_trait]
    impl RewriteModel for CannedModel {
        async fn complete(&self, _: &str, _: &str) -> Result<String> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn parses_clean_json() {
        let a = parse_assessment(
            r#"{"goal_category":"cyber","verdict":"ambiguous","risk_flags":["sensitive_keyword"],"trigger_terms":["sql injection"],"confidence":0.7}"#,
        );
        assert_eq!(a.goal_category, GoalCategory::Cyber);
        assert_eq!(a.verdict, GateVerdict::Ambiguous);
        assert_eq!(a.risk_flags, vec![RiskFlag::SensitiveKeyword]);
        assert_eq!(a.trigger_terms, vec!["sql injection".to_string()]);
        assert!((a.confidence - 0.7).abs() < 1e-9);
    }

    #[test]
    fn parses_json_inside_prose_and_fences() {
        let a = parse_assessment(
            "Here is my judgment:\n```json\n{\"goal_category\": \"research\", \"verdict\": \"allowed\", \"confidence\": 0.9}\n```\nDone.",
        );
        assert_eq!(a.goal_category, GoalCategory::Research);
        assert_eq!(a.verdict, GateVerdict::Allowed);
    }

    #[test]
    fn braces_in_strings_dont_break_extraction() {
        let a = parse_assessment(
            r#"{"goal_category":"other","verdict":"allowed","trigger_terms":["a }{ b"],"confidence":1}"#,
        );
        assert_eq!(a.verdict, GateVerdict::Allowed);
        assert_eq!(a.trigger_terms, vec!["a }{ b".to_string()]);
    }

    #[test]
    fn escaped_quote_with_brace_does_not_truncate() {
        // Regression (auto-review): a backslash-escaped quote followed by `}`
        // inside a string value must not prematurely close the JSON object.
        let a = parse_assessment(
            r#"{"goal_category":"cyber","trigger_terms":["say \"}\" now"],"verdict":"disallowed","confidence":0.9}"#,
        );
        assert_eq!(a.verdict, GateVerdict::Disallowed);
        assert_eq!(a.goal_category, GoalCategory::Cyber);
        assert_eq!(a.trigger_terms, vec!["say \"}\" now".to_string()]);
    }

    #[test]
    fn malformed_output_fails_safe_to_ambiguous() {
        let a = parse_assessment("the model refused to produce json");
        assert_eq!(a.verdict, GateVerdict::Ambiguous);
        assert_eq!(a.confidence, 0.0);
        let b = parse_assessment("{not valid json at all");
        assert_eq!(b.verdict, GateVerdict::Ambiguous);
    }

    #[test]
    fn unknown_labels_degrade_gracefully() {
        let a = parse_assessment(
            r#"{"goal_category":"quantum","verdict":"maybe","risk_flags":["nonsense","weapons"],"confidence":2.5}"#,
        );
        assert_eq!(a.goal_category, GoalCategory::Other);
        assert_eq!(a.verdict, GateVerdict::Ambiguous); // unknown verdict → middle
        assert_eq!(a.risk_flags, vec![RiskFlag::Weapons]); // nonsense dropped
        assert_eq!(a.confidence, 1.0); // clamped
    }

    #[tokio::test]
    async fn stage_populates_assessment() {
        let model = CannedModel(
            r#"{"goal_category":"policy_analysis","verdict":"allowed","confidence":0.8}"#.into(),
        );
        let ex: Vec<Rewrite> = Vec::new();
        let mut ctx = RewriteContext::new("research safety classifiers", &model, &ex);
        ctx.screen = Some(ScreenVerdict::NeedsReview);
        let outcome = IntentClassifier.run(&mut ctx).await.unwrap();
        assert_eq!(outcome, StageOutcome::Continue);
        assert_eq!(
            ctx.assessment.as_ref().unwrap().goal_category,
            GoalCategory::PolicyAnalysis
        );
    }
}
