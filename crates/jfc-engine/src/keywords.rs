//! User-input keyword detection and stripping.
//!
//! Certain "magic" keywords embedded in user messages trigger special
//! behaviour — injecting a system-reminder, swapping the model, bumping
//! reasoning effort, etc. The keyword is stripped from the user-visible
//! text so it doesn't clutter the conversation.
//!
//! Pattern mirrors Claude Code v146's `qI6(input, keyword)` regex
//! matcher: case-insensitive, whole-word boundary, strips the first
//! occurrence and returns whether a match was found.

use regex::Regex;
use std::sync::OnceLock;

/// Result of scanning a user message for magic keywords.
#[derive(Debug, Default)]
pub struct KeywordScanResult {
    /// The user text with any detected keywords stripped out.
    pub text: String,
    /// Whether the "ultrawork" keyword was detected.
    pub ultrawork: bool,
    /// Whether the "ultracode" keyword was detected — enables the standing,
    /// session-scoped workflow-by-default mode (vs `ultrawork`'s per-turn nudge).
    pub ultracode: bool,
    /// Whether the "ultrathink" keyword was detected.
    pub ultrathink: bool,
    /// Whether the explicit per-turn exploration marker `//explore` was detected.
    pub explore: bool,
    /// Per-turn reasoning-effort override from a `//effort <level>` marker
    /// (e.g. `//effort high`). Applies to this turn only, then reverts to the
    /// session default — Claude Code's `turnEffort`. `None` when absent or the
    /// level didn't parse.
    pub turn_effort: Option<crate::effort::ReasoningEffort>,
}

/// `//effort <level>` per-turn marker. Captures the level token so it can be
/// parsed via `ReasoningEffort::from_str_loose`.
fn turn_effort_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)(^|\s)//effort\s+([a-z-]+)").expect("turn-effort regex is valid")
    })
}

/// Case-insensitive whole-word match for "ultrawork".
fn ultrawork_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bultrawork\b").expect("ultrawork regex is valid"))
}

/// Case-insensitive whole-word match for "ultracode".
fn ultracode_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bultracode\b").expect("ultracode regex is valid"))
}

/// Case-insensitive whole-word match for "ultrathink".
fn ultrathink_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)\bultrathink\b").expect("ultrathink regex is valid"))
}

/// Explicit exploration marker. We intentionally do NOT reserve the plain word
/// "explore" because it is normal user prose; `//explore` is a command-like
/// marker that can be stripped safely.
fn explore_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?i)(^|\s)//explore\b").expect("explore regex is valid"))
}

/// Scan `input` for recognised magic keywords. Returns the cleaned text
/// (keywords removed, surrounding whitespace collapsed) and flags
/// indicating which keywords were found.
pub fn scan_and_strip(input: &str) -> KeywordScanResult {
    let mut text = input.to_owned();
    let mut ultrawork = false;
    let mut ultracode = false;
    let mut ultrathink = false;
    let mut explore = false;
    let mut turn_effort = None;

    // `//effort <level>` per-turn override — capture + strip the whole marker.
    if let Some(caps) = turn_effort_regex().captures(&text) {
        let whole = caps.get(0).expect("group 0 always present");
        let lead = caps.get(1).map(|m| m.as_str()).unwrap_or("");
        turn_effort = caps
            .get(2)
            .and_then(|m| crate::effort::ReasoningEffort::from_str_loose(m.as_str()));
        // Preserve a leading space (if the marker wasn't at the start) so words
        // don't get glued together, then collapse whitespace.
        let replacement = if lead.is_empty() { "" } else { " " };
        text = format!(
            "{}{replacement}{}",
            &text[..whole.start()],
            &text[whole.end()..]
        );
        text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        text = text.trim().to_owned();
    }

    // "ultracode" is checked before "ultrawork" so the more specific
    // session-mode keyword wins and isn't shadowed.
    if let Some(m) = ultracode_regex().find(&text) {
        ultracode = true;
        text = format!("{}{}", &text[..m.start()], &text[m.end()..]);
        text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        text = text.trim().to_owned();
    }

    if let Some(m) = ultrawork_regex().find(&text) {
        ultrawork = true;
        // Remove the keyword occurrence and collapse any resulting
        // double-space or leading/trailing whitespace.
        text = format!("{}{}", &text[..m.start()], &text[m.end()..]);
        // Collapse runs of whitespace left by the removal.
        text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        text = text.trim().to_owned();
    }

    if let Some(m) = ultrathink_regex().find(&text) {
        ultrathink = true;
        text = format!("{}{}", &text[..m.start()], &text[m.end()..]);
        text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        text = text.trim().to_owned();
    }

    if let Some(m) = explore_regex().find(&text) {
        explore = true;
        let prefix = &text[..m.start()];
        let suffix = &text[m.end()..];
        text = format!("{prefix}{suffix}");
        text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        text = text.trim().to_owned();
    }

    KeywordScanResult {
        text,
        ultrawork,
        ultracode,
        ultrathink,
        explore,
        turn_effort,
    }
}

/// The system-reminder body injected when "ultrawork" is detected.
pub const ULTRAWORK_REMINDER: &str = "The user included the keyword \"ultrawork\", which means you should \
     use the Workflow tool to fulfill their request.";

/// The standing system-reminder body injected on every turn while session
/// `ultracode` mode is active (set via the `ultracode` keyword or
/// `/effort ultracode`). Mirrors Claude Code's ultracode reminder.
pub const ULTRACODE_REMINDER: &str = "ultracode mode is active for this session. Use the Workflow tool \
     for every substantive task by default — chain multi-phase workflows (understand, design, \
     implement, review) rather than doing the work inline. Token cost is not a constraint here; \
     prioritize thoroughness and verification.";

/// The system-reminder body injected when "ultrathink" is detected.
pub const ULTRATHINK_REMINDER: &str = "The user included the keyword \"ultrathink\", requesting deeper reasoning on this turn. Reason as thoroughly as the task warrants.";

/// The system-reminder body injected when `//explore` is detected.
pub const EXPLORE_REMINDER: &str = "The user included the `//explore` marker, requesting broader exploration on this turn before narrowing to an answer or edit.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Normal: keyword is detected and stripped.
    #[test]
    fn detects_and_strips_ultrawork_normal() {
        let result = scan_and_strip("ultrawork fix the login bug");
        assert!(result.ultrawork);
        assert!(!result.ultrathink);
        assert_eq!(result.text, "fix the login bug");
    }

    /// Normal: keyword is case-insensitive.
    #[test]
    fn case_insensitive_detection_normal() {
        let result = scan_and_strip("ULTRAWORK do the thing");
        assert!(result.ultrawork);
        assert_eq!(result.text, "do the thing");
    }

    /// Normal: keyword in the middle of text.
    #[test]
    fn keyword_in_middle_normal() {
        let result = scan_and_strip("please ultrawork refactor auth");
        assert!(result.ultrawork);
        assert_eq!(result.text, "please refactor auth");
    }

    /// Normal: keyword at end of text.
    #[test]
    fn keyword_at_end_normal() {
        let result = scan_and_strip("refactor the auth system ultrawork");
        assert!(result.ultrawork);
        assert_eq!(result.text, "refactor the auth system");
    }

    /// Robust: no keyword present — text passes through unchanged.
    #[test]
    fn no_keyword_passthrough_robust() {
        let result = scan_and_strip("just a normal message");
        assert!(!result.ultrawork);
        assert_eq!(result.text, "just a normal message");
    }

    /// Robust: keyword as substring of another word is NOT matched
    /// (whole-word boundary).
    #[test]
    fn no_partial_match_robust() {
        let result = scan_and_strip("myultraworkflow is great");
        assert!(!result.ultrawork);
        assert_eq!(result.text, "myultraworkflow is great");
    }

    /// Robust: empty input.
    #[test]
    fn empty_input_robust() {
        let result = scan_and_strip("");
        assert!(!result.ultrawork);
        assert!(!result.ultrathink);
        assert_eq!(result.text, "");
    }

    /// Robust: keyword alone (text becomes empty after strip).
    #[test]
    fn keyword_alone_robust() {
        let result = scan_and_strip("ultrawork");
        assert!(result.ultrawork);
        assert_eq!(result.text, "");
    }

    #[test]
    fn detects_and_strips_ultracode_normal() {
        let result = scan_and_strip("ultracode rewrite the parser");
        assert!(result.ultracode);
        assert!(!result.ultrawork);
        assert_eq!(result.text, "rewrite the parser");
    }

    #[test]
    fn ultracode_is_case_insensitive_normal() {
        let result = scan_and_strip("ULTRACODE do the migration");
        assert!(result.ultracode);
        assert_eq!(result.text, "do the migration");
    }

    #[test]
    fn ultracode_not_partial_matched_robust() {
        let result = scan_and_strip("the ultracodebase is large");
        assert!(!result.ultracode);
        assert_eq!(result.text, "the ultracodebase is large");
    }

    #[test]
    fn detects_and_strips_ultrathink_normal() {
        let result = scan_and_strip("ultrathink debug the queue");
        assert!(result.ultrathink);
        assert_eq!(result.text, "debug the queue");
    }

    #[test]
    fn detects_and_strips_explicit_explore_marker_normal() {
        let result = scan_and_strip("please //explore scheduler.rs");
        assert!(result.explore);
        assert_eq!(result.text, "please scheduler.rs");
    }

    #[test]
    fn detects_and_strips_turn_effort_marker_normal() {
        let result = scan_and_strip("//effort high refactor the parser");
        assert_eq!(
            result.turn_effort,
            Some(crate::effort::ReasoningEffort::High)
        );
        assert_eq!(result.text, "refactor the parser");
    }

    #[test]
    fn turn_effort_marker_mid_text_normal() {
        let result = scan_and_strip("fix the bug //effort max please");
        assert_eq!(
            result.turn_effort,
            Some(crate::effort::ReasoningEffort::Max)
        );
        assert_eq!(result.text, "fix the bug please");
    }

    #[test]
    fn turn_effort_unknown_level_strips_but_no_effort_robust() {
        let result = scan_and_strip("//effort bogus do it");
        assert_eq!(result.turn_effort, None);
        // The marker is still stripped even when the level doesn't parse.
        assert_eq!(result.text, "do it");
    }

    #[test]
    fn plain_text_has_no_turn_effort_robust() {
        let result = scan_and_strip("the effort was high");
        assert_eq!(result.turn_effort, None);
        assert_eq!(result.text, "the effort was high");
    }

    #[test]
    fn plain_explore_is_not_magic_robust() {
        let result = scan_and_strip("please explore scheduler.rs");
        assert!(!result.explore);
        assert_eq!(result.text, "please explore scheduler.rs");
    }
}
