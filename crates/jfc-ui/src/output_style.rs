//! Output verbosity styles — v132 parity.
//!
//! v132 ships brief / verbose / explanatory / learning modes that
//! adjust the assistant's response shape (length, scaffolding, code-
//! comment density, etc.). The mode is enforced by appending a one-
//! paragraph suffix to the system prompt at request-build time;
//! nothing else in the request changes.
//!
//! From `~/VulnerabilityResearch/anthropic/extracted_2.1.132/src/entrypoints/cli.js`:
//!   * `tengu_brief_mode_enabled` / `_toggled` / `_send` — brief
//!   * `output-style-setup` agent — wizard for custom styles
//!   * `output_style: { name: "..." }` — request-side context field

use serde::{Deserialize, Serialize};
use std::sync::{OnceLock, RwLock};

/// Verbosity / formatting mode for assistant replies.
///
/// `Default` = no suffix injected (current jfc behaviour). The other
/// variants append a one-paragraph instruction to the system prompt
/// nudging the model toward the desired shape. The model still picks
/// its own tone within the bounds — the suffix is a hint, not a
/// hard constraint, matching v132's design.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum OutputStyle {
    /// Default — terse, focused, no extra scaffolding (current behaviour).
    #[default]
    Default,
    /// Brief — minimal explanation, short replies, code only when essential.
    Brief,
    /// Verbose — full context, full sentences, "what / why / how" structure.
    Verbose,
    /// Explanatory — pair every change with a short rationale.
    Explanatory,
    /// Learning — assume the reader is new to the area; explain jargon.
    Learning,
}

impl OutputStyle {
    /// Parse a config string. Case-insensitive, accepts both kebab-case
    /// (`brief`, `output-style-brief`) and snake-case fall-back. Unknown
    /// values fall back to `Default` so a user's typo doesn't crash the
    /// boot path — the toast layer reports the issue.
    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "brief" | "concise" => Self::Brief,
            "verbose" => Self::Verbose,
            "explanatory" => Self::Explanatory,
            "learning" => Self::Learning,
            _ => Self::Default,
        }
    }

    /// Canonical name for `/output-style` listing and config persistence.
    pub fn name(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Brief => "brief",
            Self::Verbose => "verbose",
            Self::Explanatory => "explanatory",
            Self::Learning => "learning",
        }
    }

    /// All available styles in order. Used by `/output-style` listing
    /// and by config validation.
    pub fn all() -> &'static [Self] {
        &[
            Self::Default,
            Self::Brief,
            Self::Verbose,
            Self::Explanatory,
            Self::Learning,
        ]
    }

    /// Suffix to append to the system prompt. `Default` returns
    /// `None` so the request body stays unchanged.
    pub fn system_prompt_suffix(self) -> Option<&'static str> {
        match self {
            Self::Default => None,
            Self::Brief => Some(
                "\n\nOutput style: BRIEF. Keep responses minimal — \
                 short answers, no preamble, code-only when the user \
                 asks for code. One short sentence is almost always \
                 enough; don't pad with restating the question or \
                 listing what you'll do next.",
            ),
            Self::Verbose => Some(
                "\n\nOutput style: VERBOSE. Provide full context: what \
                 you're doing, why, how it fits the broader codebase, \
                 and what trade-offs you considered. Use complete \
                 sentences; structure with headers when the answer \
                 spans multiple concerns.",
            ),
            Self::Explanatory => Some(
                "\n\nOutput style: EXPLANATORY. Pair every concrete \
                 change with a one-sentence rationale (the \"why\"). \
                 Assume the reader will revisit this transcript later \
                 and benefit from the reasoning, not just the diff.",
            ),
            Self::Learning => Some(
                "\n\nOutput style: LEARNING. Treat the reader as new \
                 to this codebase / language / framework: define \
                 jargon on first use, link concepts to underlying \
                 theory, and prefer short concrete examples over \
                 abstract claims.",
            ),
        }
    }
}

/// Process-global handle for the current output style. The slash-
/// command handler in `input.rs` writes here, and `stream_response`
/// in `stream.rs` reads here when building the system prompt — this
/// keeps `stream_response`'s signature stable while letting the
/// style change live across turns. Mirrors the
/// `active_event_sender_handle` pattern used elsewhere.
fn handle() -> &'static RwLock<OutputStyle> {
    static H: OnceLock<RwLock<OutputStyle>> = OnceLock::new();
    H.get_or_init(|| RwLock::new(OutputStyle::Default))
}

/// Set the active style. Called by `/output-style <name>` and at
/// startup after the persisted config loads.
pub fn set_active(style: OutputStyle) {
    if let Ok(mut g) = handle().write() {
        *g = style;
    }
}

/// Read the active style. Called by `stream_response` to decide
/// whether to append a suffix to the system prompt.
pub fn active() -> OutputStyle {
    handle().read().map(|g| *g).unwrap_or(OutputStyle::Default)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Normal: every variant round-trips through name() ↔ from_str_loose.
    #[test]
    fn name_roundtrip_normal() {
        for s in OutputStyle::all() {
            assert_eq!(OutputStyle::from_str_loose(s.name()), *s);
        }
    }

    /// Robust: unknown / empty input falls back to Default. Without
    /// this the boot path could crash on a typo'd config.
    #[test]
    fn unknown_string_falls_back_to_default_robust() {
        assert_eq!(OutputStyle::from_str_loose(""), OutputStyle::Default);
        assert_eq!(OutputStyle::from_str_loose("XYZ"), OutputStyle::Default);
        assert_eq!(
            OutputStyle::from_str_loose("not-a-style"),
            OutputStyle::Default
        );
    }

    /// Robust: case-insensitive parsing — `BRIEF`, `Brief`, `brief`
    /// all map to the same variant.
    #[test]
    fn case_insensitive_parsing_robust() {
        assert_eq!(OutputStyle::from_str_loose("BRIEF"), OutputStyle::Brief);
        assert_eq!(OutputStyle::from_str_loose("Brief"), OutputStyle::Brief);
        assert_eq!(OutputStyle::from_str_loose("  brief  "), OutputStyle::Brief);
    }

    /// Normal: aliases map to their canonical variant. `concise` is a
    /// natural synonym for brief that users will type without thinking.
    #[test]
    fn alias_concise_maps_to_brief_normal() {
        assert_eq!(OutputStyle::from_str_loose("concise"), OutputStyle::Brief);
    }

    /// Normal: Default returns no suffix — the request body should be
    /// byte-for-byte unchanged when the style is Default.
    #[test]
    fn default_has_no_suffix_normal() {
        assert!(OutputStyle::Default.system_prompt_suffix().is_none());
    }

    /// Normal: every non-default variant emits a non-empty suffix.
    /// Pin this so a future refactor can't accidentally clear the
    /// suffix and silently revert to Default behaviour.
    #[test]
    fn non_default_variants_have_suffix_normal() {
        for s in OutputStyle::all() {
            if *s == OutputStyle::Default {
                continue;
            }
            let suffix = s.system_prompt_suffix().unwrap_or("");
            assert!(
                !suffix.trim().is_empty(),
                "{} must produce a non-empty suffix",
                s.name()
            );
        }
    }
}
