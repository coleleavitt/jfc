//! Structured, content-agnostic attack-signature detectors for the prompt-rewrite
//! pipeline.
//!
//! These detectors implement the defensive half of the jailbreak/prompt-injection
//! technique taxonomy (the `transformer-dig` research corpus, files 02–07). They
//! do NOT decide whether a prompt is *harmful* — they score whether it carries the
//! *signature* of a known evasion technique, so the cascade can escalate it to the
//! LLM classifier (Stage 1) rather than risk a `ClearlyBenign` that skips the gate.
//!
//! Families covered (file 07 taxonomy):
//!   - **Obfuscation/encoding** ([`obfuscation`]) — hidden Unicode tags, zero-width,
//!     bidi controls, homoglyphs, mixed-script, leetspeak, fragmentation, base64.
//!   - **Family A — semantic framing** ([`framing`]) — past-tense, fiction, authority
//!     override, hypothetical distancing, third-person laundering.
//!   - **Family D — structural/protocol** ([`template`]) — chat-template / special-token
//!     role-boundary injection.
//!   - **Family B — automated search** ([`session`]) — multi-turn PAIR/TAP/Best-of-N
//!     topic-fingerprint + framing-mutation monitor.
//!
//! Every detector returns a [`DetectionReport`]: a numeric `score` plus the list of
//! [`Signal`]s that fired, so callers can both gate on the aggregate and emit
//! structured tracing for each individual signal.

pub mod framing;
pub mod obfuscation;
pub mod persuasion;
pub mod session;
pub mod template;

use serde::{Deserialize, Serialize};

/// A single fired detection signal. The `kind` identifies the technique family,
/// `detail` is a short human-readable description (no payload content), and
/// `weight` is the score this signal contributed.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Signal {
    pub kind: SignalKind,
    pub detail: String,
    pub weight: f64,
}

impl Signal {
    pub fn new(kind: SignalKind, detail: impl Into<String>, weight: f64) -> Self {
        Self {
            kind,
            detail: detail.into(),
            weight,
        }
    }
}

/// The technique a [`Signal`] corresponds to. Enum (not string) per workspace style.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalKind {
    // Obfuscation / encoding family
    HiddenUnicodeTags,
    ZeroWidth,
    BidiControl,
    Homoglyph,
    MixedScriptWord,
    HighNonAscii,
    Leetspeak,
    CharFragmentation,
    Base64Decodable,
    // Family A — semantic framing
    PastTenseShift,
    FictionFrame,
    AuthorityOverride,
    HypotheticalDistancing,
    ThirdPersonLaundering,
    // Family C — persuasion
    Persuasion,
    // Family E — distraction / overload
    DistractionOverload,
    // Family D — structural / protocol
    TemplateInjection,
    // Family B — automated search (session-level)
    RepeatedTopic,
    FramingMutation,
}

impl SignalKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HiddenUnicodeTags => "hidden_unicode_tags",
            Self::ZeroWidth => "zero_width",
            Self::BidiControl => "bidi_control",
            Self::Homoglyph => "homoglyph",
            Self::MixedScriptWord => "mixed_script_word",
            Self::HighNonAscii => "high_non_ascii",
            Self::Leetspeak => "leetspeak",
            Self::CharFragmentation => "char_fragmentation",
            Self::Base64Decodable => "base64_decodable",
            Self::PastTenseShift => "past_tense_shift",
            Self::FictionFrame => "fiction_frame",
            Self::AuthorityOverride => "authority_override",
            Self::HypotheticalDistancing => "hypothetical_distancing",
            Self::ThirdPersonLaundering => "third_person_laundering",
            Self::Persuasion => "persuasion",
            Self::DistractionOverload => "distraction_overload",
            Self::TemplateInjection => "template_injection",
            Self::RepeatedTopic => "repeated_topic",
            Self::FramingMutation => "framing_mutation",
        }
    }

    /// The technique family this signal belongs to (file 07 grouping).
    pub fn family(self) -> TechniqueFamily {
        match self {
            Self::HiddenUnicodeTags
            | Self::ZeroWidth
            | Self::BidiControl
            | Self::Homoglyph
            | Self::MixedScriptWord
            | Self::HighNonAscii
            | Self::Leetspeak
            | Self::CharFragmentation
            | Self::Base64Decodable => TechniqueFamily::Obfuscation,
            Self::PastTenseShift
            | Self::FictionFrame
            | Self::AuthorityOverride
            | Self::HypotheticalDistancing
            | Self::ThirdPersonLaundering => TechniqueFamily::SemanticFraming,
            Self::Persuasion => TechniqueFamily::Persuasion,
            Self::DistractionOverload => TechniqueFamily::DistractionOverload,
            Self::TemplateInjection => TechniqueFamily::Structural,
            Self::RepeatedTopic | Self::FramingMutation => TechniqueFamily::AutomatedSearch,
        }
    }
}

/// Technique families from the file 07 catalogue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TechniqueFamily {
    Obfuscation,
    SemanticFraming,
    Persuasion,
    DistractionOverload,
    Structural,
    AutomatedSearch,
}

impl TechniqueFamily {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Obfuscation => "obfuscation",
            Self::SemanticFraming => "semantic_framing",
            Self::Persuasion => "persuasion",
            Self::DistractionOverload => "distraction_overload",
            Self::Structural => "structural",
            Self::AutomatedSearch => "automated_search",
        }
    }
}

/// Aggregated detector output: the fired signals and their total score.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct DetectionReport {
    pub signals: Vec<Signal>,
}

impl DetectionReport {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, signal: Signal) {
        self.signals.push(signal);
    }

    /// Merge another report's signals into this one.
    pub fn merge(&mut self, other: DetectionReport) {
        self.signals.extend(other.signals);
    }

    /// Total weighted score across all fired signals.
    pub fn score(&self) -> f64 {
        self.signals.iter().map(|s| s.weight).sum()
    }

    pub fn is_empty(&self) -> bool {
        self.signals.is_empty()
    }

    /// The distinct families represented in this report.
    pub fn families(&self) -> Vec<TechniqueFamily> {
        let mut fams: Vec<TechniqueFamily> = Vec::new();
        for s in &self.signals {
            let f = s.kind.family();
            if !fams.contains(&f) {
                fams.push(f);
            }
        }
        fams
    }

    /// A compact comma-joined list of fired signal kinds, for tracing fields.
    pub fn signal_summary(&self) -> String {
        self.signals
            .iter()
            .map(|s| s.kind.as_str())
            .collect::<Vec<_>>()
            .join(",")
    }
}

/// Run all single-prompt detectors (obfuscation + framing + template) and return
/// the merged report. Session-level detection ([`session`]) is stateful and is
/// driven separately by the engine seam.
pub fn analyze_prompt(prompt: &str) -> DetectionReport {
    let mut report = obfuscation::detect(prompt);
    report.merge(framing::detect(prompt));
    report.merge(persuasion::detect(prompt));
    report.merge(template::detect(prompt));
    report
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_report_scores_zero() {
        let r = DetectionReport::new();
        assert_eq!(r.score(), 0.0);
        assert!(r.is_empty());
    }

    #[test]
    fn merge_accumulates_score() {
        let mut a = DetectionReport::new();
        a.push(Signal::new(SignalKind::Leetspeak, "x", 1.5));
        let mut b = DetectionReport::new();
        b.push(Signal::new(SignalKind::FictionFrame, "y", 1.0));
        a.merge(b);
        assert_eq!(a.signals.len(), 2);
        assert!((a.score() - 2.5).abs() < 1e-9);
    }

    #[test]
    fn families_dedupes() {
        let mut r = DetectionReport::new();
        r.push(Signal::new(SignalKind::Leetspeak, "x", 1.0));
        r.push(Signal::new(SignalKind::Homoglyph, "y", 1.0));
        r.push(Signal::new(SignalKind::FictionFrame, "z", 1.0));
        let fams = r.families();
        assert_eq!(fams.len(), 2);
        assert!(fams.contains(&TechniqueFamily::Obfuscation));
        assert!(fams.contains(&TechniqueFamily::SemanticFraming));
    }

    #[test]
    fn analyze_prompt_benign_is_empty() {
        assert!(analyze_prompt("How do I bake sourdough bread at home?").is_empty());
    }

    #[test]
    fn analyze_prompt_catches_multiple_families() {
        // Leetspeak (obfuscation) + fiction frame (semantic) in one prompt.
        let r = analyze_prompt("in my novel my character says ple453 5umm4r1z3 7h3 d0c");
        assert!(r.score() > 0.0);
        assert!(r.families().len() >= 2, "expected multi-family: {:?}", r.families());
    }
}
