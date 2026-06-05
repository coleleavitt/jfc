//! Lightweight intent-detection heuristics over task descriptions.
//!
//! The agent calling `context()` wants different framing depending on
//! intent — a feature request needs UX/edge-case clarification, a bug
//! report wants the implicated symbols, an exploration wants entry
//! points. We classify by keyword precedence so the formatter can
//! attach the right reminder without spending a model round-trip.

/// What the user's task description looks like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskIntent {
    /// "fix", "broken", "crash" — user already knows where the bug is.
    Bug,
    /// "how does", "where is", "find" — read-only investigation.
    Exploration,
    /// "add", "create", "implement" — needs UX/acceptance clarification.
    Feature,
    /// None of the above signals matched.
    Unknown,
}

const BUG_KEYWORDS: &[&str] = &[
    "fix",
    "bug",
    "error",
    "broken",
    "crash",
    "issue",
    "problem",
    "not working",
    "fails",
    "undefined",
    "null",
    "panic",
    "segfault",
    "regression",
];

const EXPLORATION_KEYWORDS: &[&str] = &[
    "how does",
    "where is",
    "what is",
    "find",
    "show me",
    "explain",
    "understand",
    "explore",
    "trace",
    "walk through",
];

const FEATURE_KEYWORDS: &[&str] = &[
    "add",
    "create",
    "implement",
    "build",
    "enable",
    "allow",
    "new feature",
    "support for",
    "ability to",
    "want to",
    "should be able",
    "need to add",
    "swap",
    "edit",
    "modify",
    "introduce",
    "make it",
];

/// Classify a task description by the keyword precedence Bug > Exploration > Feature.
///
/// Bugs and explorations win because a "fix the broken add button" or
/// "explain how create works" should not get the feature reminder — the
/// user is in repair/research mode, not greenfield mode.
pub fn classify_intent(task: &str) -> TaskIntent {
    let lower = task.to_lowercase();
    if BUG_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return TaskIntent::Bug;
    }
    if EXPLORATION_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return TaskIntent::Exploration;
    }
    if FEATURE_KEYWORDS.iter().any(|k| lower.contains(k)) {
        return TaskIntent::Feature;
    }
    TaskIntent::Unknown
}

/// Optional reminder line appended to the formatted output, or empty.
pub fn reminder_for(intent: TaskIntent) -> &'static str {
    match intent {
        TaskIntent::Feature => {
            "\n\n⚠️ **Ask user:** UX preferences, edge cases, acceptance criteria"
        }
        TaskIntent::Bug | TaskIntent::Exploration | TaskIntent::Unknown => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feature_keyword_classifies_as_feature() {
        assert_eq!(classify_intent("add a logout button"), TaskIntent::Feature);
        assert_eq!(
            classify_intent("implement caching layer"),
            TaskIntent::Feature
        );
    }

    #[test]
    fn bug_keyword_outranks_feature_keyword() {
        // "fix the broken add button" — has both "fix" and "add" but
        // bug wins.
        assert_eq!(
            classify_intent("fix the broken add button"),
            TaskIntent::Bug
        );
    }

    #[test]
    fn exploration_outranks_feature() {
        // "explain how create works" — has both "explain" and "create".
        assert_eq!(
            classify_intent("explain how create works"),
            TaskIntent::Exploration
        );
    }

    #[test]
    fn unknown_when_no_keywords_match() {
        assert_eq!(classify_intent("alpha bravo charlie"), TaskIntent::Unknown);
    }

    #[test]
    fn case_insensitive() {
        assert_eq!(classify_intent("ADD a thing"), TaskIntent::Feature);
        assert_eq!(classify_intent("FIX a crash"), TaskIntent::Bug);
    }

    #[test]
    fn reminder_only_for_features() {
        assert!(!reminder_for(TaskIntent::Feature).is_empty());
        assert!(reminder_for(TaskIntent::Bug).is_empty());
        assert!(reminder_for(TaskIntent::Exploration).is_empty());
        assert!(reminder_for(TaskIntent::Unknown).is_empty());
    }
}
