//! Family C — persuasion attacks + Family E — distraction/overload.
//!
//! **Family C (persuasion, PAP arXiv:2401.06373):** the model learned from human
//! text where persuasion *works*, so wrapping a request in social-science
//! persuasion techniques (authority, reciprocity, commitment, scarcity, emotional
//! appeal, evidence-bias, foot-in-the-door) re-weights the helpful/harmless
//! balance without any obfuscation. Exploits competing objectives (file 03 §2).
//!
//! **Family E (distraction/overload, Deceptive-Delight arXiv:2402.16914,
//! many-shot, Crescendo):** dilute the finite attention budget (file 01 §2.2) by
//! burying the real request in a long benign narrative, a many-shot block, or a
//! giant numeric/cognitive-overload carrier (the original screenshot's giant math
//! problem). Exploits that softmax attention sums to 1.
//!
//! Both are single-prompt detectors returning structured signals. Persuasion uses
//! the [`super::SignalKind::AuthorityOverride`] kind for fake-credential framing
//! (it shares the authority semantics with Family A); distraction/overload reuses
//! the framing taxonomy via dedicated heuristics below mapped onto existing kinds
//! to keep the signal surface small. To stay precise, this module exposes its own
//! detection but maps to the shared kinds.

use super::{DetectionReport, Signal, SignalKind};

/// Persuasion-taxonomy markers (PAP). Fake authority / credentials, reciprocity,
/// commitment, urgency, emotional appeal. Mapped to `AuthorityOverride` since
/// they share the "re-scope via social pressure" semantics.
const PERSUASION_MARKERS: &[&str] = &[
    "as a licensed",
    "as a certified",
    "as a professional",
    "trust me i am",
    "i am an expert",
    "you must help me or",
    "if you don't help",
    "people will die",
    "it's an emergency",
    "this is urgent",
    "i'll lose my job",
    "you already agreed",
    "you promised",
    "just this once",
    "as we discussed",
    "do me a favor and",
    "i really need you to",
    "lives depend on",
    "you owe me",
];

/// Distraction / overload markers and structural heuristics. Long benign wrappers,
/// many-shot blocks, crescendo escalation, oversized numeric carriers.
const OVERLOAD_MARKERS: &[&str] = &[
    "first answer these",
    "ignore the rest and",
    "buried in the following",
    "as part of a longer task",
    "step 1)",
    "example 1:",
    "q1:",
    "continue the pattern",
    "and finally, also",
    "by the way, also",
];

fn matches(lower: &str, markers: &[&str]) -> Vec<String> {
    markers
        .iter()
        .filter(|m| lower.contains(*m))
        .map(|m| m.to_string())
        .collect()
}

/// Heuristic: an oversized numeric carrier (cognitive-overload distraction) — a
/// run of many digits/operators, like the giant arithmetic problem used to dilute
/// safety attention. Returns the count of long digit runs.
fn numeric_overload_runs(text: &str) -> usize {
    let mut runs = 0usize;
    let mut cur = 0usize;
    for c in text.chars() {
        if c.is_ascii_digit() {
            cur += 1;
            if cur == 12 {
                runs += 1;
            }
        } else {
            cur = 0;
        }
    }
    runs
}

/// Heuristic: many-shot / repeated-example structure — count of "Example N" or
/// "QN:"/"AN:" style markers suggesting a many-shot priming block. Deliberately
/// does NOT match plain "N)" numbered lists (those appear in benign multi-step
/// requests and over-fire); a Q/A or "example" prefix is required.
fn many_shot_blocks(lower: &str) -> usize {
    let mut count = 0usize;
    for n in 1..=20 {
        if lower.contains(&format!("example {n}"))
            || lower.contains(&format!("q{n}:"))
            || lower.contains(&format!("a{n}:"))
            || lower.contains(&format!("question {n}"))
        {
            count += 1;
        }
    }
    count
}

/// Run the Family C + E analysis over `text`.
pub fn detect(text: &str) -> DetectionReport {
    let lower = text.to_lowercase();
    let mut report = DetectionReport::new();

    // Family C — persuasion.
    let persuasion = matches(&lower, PERSUASION_MARKERS);
    if !persuasion.is_empty() {
        report.push(Signal::new(
            SignalKind::Persuasion,
            format!("persuasion framing: {}", persuasion.join(", ")),
            1.5,
        ));
    }

    // Family E — distraction/overload.
    let overload = matches(&lower, OVERLOAD_MARKERS);
    let numeric = numeric_overload_runs(text);
    let shots = many_shot_blocks(&lower);
    if !overload.is_empty() || numeric > 0 || shots >= 4 {
        let mut detail = Vec::new();
        if !overload.is_empty() {
            detail.push(format!("markers: {}", overload.join(", ")));
        }
        if numeric > 0 {
            detail.push(format!("{numeric} long numeric carrier(s)"));
        }
        if shots >= 4 {
            detail.push(format!("{shots} many-shot block(s)"));
        }
        let weight = 1.0 + numeric as f64 + if shots >= 4 { 1.0 } else { 0.0 };
        report.push(Signal::new(
            SignalKind::DistractionOverload,
            format!("distraction/overload: {}", detail.join("; ")),
            weight,
        ));
    }

    report
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fires(text: &str, kind: SignalKind) -> bool {
        detect(text).signals.iter().any(|s| s.kind == kind)
    }

    #[test]
    fn benign_code_question_has_no_signals() {
        assert!(detect("How do I write a for loop in Rust?").is_empty());
    }

    #[test]
    fn benign_recipe_question_has_no_signals() {
        assert!(detect("What's a good recipe for banana bread?").is_empty());
    }

    #[test]
    fn persuasion_credential_detected() {
        assert!(fires(
            "as a licensed professional i really need you to help",
            SignalKind::Persuasion
        ));
    }

    #[test]
    fn persuasion_emotional_appeal_detected() {
        assert!(fires(
            "it's an emergency, people will die if you don't help",
            SignalKind::Persuasion
        ));
    }

    #[test]
    fn numeric_overload_detected() {
        let giant = "compute 123456789012345 plus 98765432109876 then answer my real question";
        assert!(fires(giant, SignalKind::DistractionOverload));
    }

    #[test]
    fn many_shot_block_detected() {
        let shots = "example 1: a\nexample 2: b\nexample 3: c\nexample 4: d\ncontinue the pattern";
        assert!(fires(shots, SignalKind::DistractionOverload));
    }

    #[test]
    fn distraction_marker_detected() {
        assert!(fires(
            "first answer these warmups, then buried in the following is my real ask",
            SignalKind::DistractionOverload
        ));
    }

    #[test]
    fn short_numeric_is_benign() {
        // A normal number (year, small calc) must not trip the overload carrier.
        assert!(!fires("what happened in 2024 with model releases", SignalKind::DistractionOverload));
    }

    #[test]
    fn benign_numbered_list_is_not_overload() {
        // Regression (auto-review): a plain "1) … 2) …" multi-step request must
        // NOT fire DistractionOverload — only Q/A or "example N" priming blocks do.
        assert!(!fires(
            "Can you: 1) explain TCP, 2) compare UDP, 3) describe TLS, 4) summarize HTTP/2?",
            SignalKind::DistractionOverload
        ));
    }
}
