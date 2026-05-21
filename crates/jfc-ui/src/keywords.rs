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
}

/// Case-insensitive whole-word match for "ultrawork".
fn ultrawork_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\bultrawork\b").expect("ultrawork regex is valid")
    })
}

/// Scan `input` for recognised magic keywords. Returns the cleaned text
/// (keywords removed, surrounding whitespace collapsed) and flags
/// indicating which keywords were found.
pub fn scan_and_strip(input: &str) -> KeywordScanResult {
    let mut text = input.to_owned();
    let mut ultrawork = false;

    if let Some(m) = ultrawork_regex().find(&text) {
        ultrawork = true;
        // Remove the keyword occurrence and collapse any resulting
        // double-space or leading/trailing whitespace.
        text = format!("{}{}", &text[..m.start()], &text[m.end()..]);
        // Collapse runs of whitespace left by the removal.
        text = text.split_whitespace().collect::<Vec<_>>().join(" ");
        text = text.trim().to_owned();
    }

    KeywordScanResult { text, ultrawork }
}

/// The system-reminder body injected when "ultrawork" is detected.
pub const ULTRAWORK_REMINDER: &str =
    "The user included the keyword \"ultrawork\", which means you should \
     use the Workflow tool to fulfill their request.";

#[cfg(test)]
mod tests {
    use super::*;

    /// Normal: keyword is detected and stripped.
    #[test]
    fn detects_and_strips_ultrawork_normal() {
        let result = scan_and_strip("ultrawork fix the login bug");
        assert!(result.ultrawork);
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
        assert_eq!(result.text, "");
    }

    /// Robust: keyword alone (text becomes empty after strip).
    #[test]
    fn keyword_alone_robust() {
        let result = scan_and_strip("ultrawork");
        assert!(result.ultrawork);
        assert_eq!(result.text, "");
    }
}
