//! Heuristic intent classification gate.
//!
//! Classifies user messages into intent categories using keyword/pattern
//! matching. No LLM round-trip — must complete in <5ms.

/// Classified intent of a user message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Intent {
    Research,
    Implementation,
    Investigation,
    Fix,
    Evaluation,
    Chat,
}

/// Classification result with confidence.
#[derive(Debug, Clone)]
pub struct Classification {
    pub intent: Intent,
    pub confidence: f32,
}

/// Tool kind for availability mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolKind {
    Read,
    Write,
    Edit,
    Bash,
    Grep,
    Glob,
    Lsp,
}

/// Classify a user message into an intent category.
pub fn classify(message: &str) -> Classification {
    let lower = message.to_lowercase();
    let mut scores: [(Intent, f32); 6] = [
        (Intent::Research, 0.0),
        (Intent::Implementation, 0.0),
        (Intent::Investigation, 0.0),
        (Intent::Fix, 0.0),
        (Intent::Evaluation, 0.0),
        (Intent::Chat, 0.0),
    ];

    // Research keywords
    for kw in &[
        "find",
        "search",
        "where",
        "which file",
        "locate",
        "look up",
        "grep",
        "show me",
    ] {
        if lower.contains(kw) {
            scores[0].1 += 1.0;
        }
    }

    // Implementation keywords
    for kw in &[
        "create",
        "add",
        "implement",
        "build",
        "write",
        "make",
        "generate",
        "new",
    ] {
        if lower.contains(kw) {
            scores[1].1 += 1.0;
        }
    }

    // Investigation keywords
    for kw in &[
        "explain",
        "how does",
        "what does",
        "understand",
        "trace",
        "follow",
        "read",
    ] {
        if lower.contains(kw) {
            scores[2].1 += 1.0;
        }
    }

    // Fix keywords
    for kw in &[
        "fix", "bug", "error", "broken", "failing", "crash", "issue", "wrong",
    ] {
        if lower.contains(kw) {
            scores[3].1 += 1.0;
        }
    }

    // Evaluation keywords
    for kw in &[
        "review", "check", "audit", "evaluate", "assess", "quality", "test",
    ] {
        if lower.contains(kw) {
            scores[4].1 += 1.0;
        }
    }

    // Find best match
    let total: f32 = scores.iter().map(|(_, score)| score).sum();
    if total == 0.0 {
        return Classification {
            intent: Intent::Chat,
            confidence: 0.5,
        };
    }

    let (best_intent, best_score) = scores
        .iter()
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .unwrap();

    let confidence = best_score / total;
    if confidence < 0.4 {
        Classification {
            intent: Intent::Chat,
            confidence: 0.3,
        }
    } else {
        Classification {
            intent: *best_intent,
            confidence,
        }
    }
}

/// Get suggested tools for an intent (advisory, not enforcing).
pub fn suggested_tools(intent: Intent) -> Vec<ToolKind> {
    match intent {
        Intent::Research => vec![
            ToolKind::Grep,
            ToolKind::Read,
            ToolKind::Glob,
            ToolKind::Lsp,
        ],
        Intent::Implementation => vec![
            ToolKind::Edit,
            ToolKind::Write,
            ToolKind::Bash,
            ToolKind::Read,
            ToolKind::Grep,
            ToolKind::Glob,
            ToolKind::Lsp,
        ],
        Intent::Investigation => vec![
            ToolKind::Read,
            ToolKind::Grep,
            ToolKind::Lsp,
            ToolKind::Glob,
        ],
        Intent::Fix => vec![
            ToolKind::Edit,
            ToolKind::Bash,
            ToolKind::Lsp,
            ToolKind::Read,
            ToolKind::Grep,
        ],
        Intent::Evaluation => vec![
            ToolKind::Read,
            ToolKind::Grep,
            ToolKind::Glob,
            ToolKind::Lsp,
            ToolKind::Bash,
        ],
        Intent::Chat => vec![],
    }
}

/// Get tools that are discouraged for an intent (advisory).
pub fn discouraged_tools(intent: Intent) -> Vec<ToolKind> {
    match intent {
        Intent::Research => vec![ToolKind::Edit, ToolKind::Write],
        Intent::Investigation => vec![ToolKind::Write, ToolKind::Edit],
        Intent::Evaluation => vec![ToolKind::Edit, ToolKind::Write],
        _ => vec![],
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    #[test]
    fn test_classify_research() {
        let classification = classify("find where auth is handled");

        assert_eq!(classification.intent, Intent::Research);
    }

    #[test]
    fn test_classify_implementation() {
        let classification = classify("implement dark mode toggle");

        assert_eq!(classification.intent, Intent::Implementation);
    }

    #[test]
    fn test_classify_fix() {
        let classification = classify("fix the bug in login");

        assert_eq!(classification.intent, Intent::Fix);
    }

    #[test]
    fn test_classify_chat() {
        let classification = classify("hello how are you");

        assert_eq!(classification.intent, Intent::Chat);
    }

    #[test]
    fn test_classify_investigation() {
        let classification = classify("explain how the router works");

        assert_eq!(classification.intent, Intent::Investigation);
    }

    #[test]
    fn test_classify_evaluation() {
        let classification = classify("review the changes in auth module");

        assert_eq!(classification.intent, Intent::Evaluation);
    }

    #[test]
    fn test_suggested_tools_research() {
        let tools = suggested_tools(Intent::Research);

        assert!(tools.contains(&ToolKind::Grep));
        assert!(tools.contains(&ToolKind::Read));
        assert!(tools.contains(&ToolKind::Glob));
        assert!(!tools.contains(&ToolKind::Edit));
        assert!(!tools.contains(&ToolKind::Write));
    }

    #[test]
    fn test_discouraged_tools_research() {
        let tools = discouraged_tools(Intent::Research);

        assert!(tools.contains(&ToolKind::Edit));
        assert!(tools.contains(&ToolKind::Write));
    }

    #[test]
    fn test_classify_performance() {
        let start = Instant::now();

        for _ in 0..1_000 {
            let _ = classify("find where auth is handled and review the login implementation");
        }

        assert!(start.elapsed() < Duration::from_millis(50));
    }
}
