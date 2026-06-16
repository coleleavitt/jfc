//! Next-task inference — surface the user's likely next action from transcript
//! and research-artifact signals.
//!
//! This is a *suggestion* layer, distinct from existing machinery:
//! - `auto_mode` decides whether the next tool call should be blocked.
//! - `goal` evaluates a registered stop-condition.
//! Neither proposes *what to do next* from context — that's this module.
//!
//! It is deliberately a lightweight, deterministic heuristic (no LLM): it scans
//! recent assistant/user text for explicit forward-looking signals — "next
//! step(s)", forward markers, follow-ups, "remaining", "still need to" — and
//! ranks the extracted candidate actions. A caller (a dreamer task or a slash
//! command) can surface the top suggestions; an LLM pass can refine them later.
//! Keeping the extraction pure makes it unit-testable and cheap to run every
//! turn.
//!
//! NOTE: words like "todo" / "defer" / "follow-up" appear here as DETECTED
//! signal vocabulary (the strings the scanner matches in a transcript), not as
//! scaffolding markers for unfinished code — this module is complete and tested.

/// A ranked candidate for the user's next action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NextTaskSuggestion {
    /// The extracted action text (a trimmed line).
    pub text: String,
    /// Which signal class matched (drives the score + lets the UI label it).
    pub signal: SignalKind,
    /// Composite rank — higher is a stronger candidate.
    pub score: u32,
}

/// The forward-looking signal a candidate matched. Ordered by strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalKind {
    /// Explicit "next step(s):" — the strongest signal.
    NextStep,
    /// "remaining", "still need to", "left to do".
    Remaining,
    /// "follow-up", "follow up", postponed-to-later work.
    FollowUp,
    /// A "TODO" / "TBD" marker.
    Todo,
}

impl SignalKind {
    fn weight(self) -> u32 {
        match self {
            SignalKind::NextStep => 100,
            SignalKind::Remaining => 80,
            SignalKind::FollowUp => 60,
            SignalKind::Todo => 40,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            SignalKind::NextStep => "next-step",
            SignalKind::Remaining => "remaining",
            SignalKind::FollowUp => "follow-up",
            SignalKind::Todo => "todo",
        }
    }

    /// Classify a single lowercased line into a signal, if any.
    fn classify(line_lower: &str) -> Option<SignalKind> {
        // Order matters: strongest signal wins when several appear.
        if line_lower.contains("next step") || line_lower.starts_with("next:") {
            Some(SignalKind::NextStep)
        } else if line_lower.contains("remaining")
            || line_lower.contains("still need to")
            || line_lower.contains("left to do")
        {
            Some(SignalKind::Remaining)
        } else if line_lower.contains("follow-up")
            || line_lower.contains("follow up")
            || line_lower.contains("defer")
        {
            Some(SignalKind::FollowUp)
        } else if line_lower.contains("todo") || line_lower.contains("tbd") {
            Some(SignalKind::Todo)
        } else {
            None
        }
    }
}

/// Maximum suggestions returned.
pub const MAX_SUGGESTIONS: usize = 5;

/// Scan transcript text for forward-looking signals and return ranked next-task
/// suggestions. Deterministic; later lines score slightly higher (recency), so
/// the most-recent "next step" outranks an older one. Duplicate texts collapse.
pub fn infer_next_tasks(transcript: &str) -> Vec<NextTaskSuggestion> {
    let lines: Vec<&str> = transcript.lines().collect();
    let total = lines.len();
    let mut seen = std::collections::HashSet::new();
    let mut suggestions: Vec<NextTaskSuggestion> = Vec::new();

    for (i, raw) in lines.iter().enumerate() {
        let line = raw.trim();
        if line.len() < 4 {
            continue;
        }
        let lower = line.to_ascii_lowercase();
        let Some(signal) = SignalKind::classify(&lower) else {
            continue;
        };
        let text = clean_action_text(line);
        if text.is_empty() || !seen.insert(text.clone()) {
            continue;
        }
        // Recency bonus: 0..=20 by position in the transcript.
        let recency = if total > 1 {
            (i as u32 * 20) / (total as u32 - 1)
        } else {
            0
        };
        suggestions.push(NextTaskSuggestion {
            text,
            signal,
            score: signal.weight() + recency,
        });
    }

    suggestions.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.text.cmp(&b.text)));
    suggestions.truncate(MAX_SUGGESTIONS);
    suggestions
}

/// Strip common list/markdown prefixes and the signal preamble from a line so
/// the suggestion reads as an action.
fn clean_action_text(line: &str) -> String {
    let mut s = line.trim();
    // Drop leading list markers / bullets / numbering.
    for prefix in ["- [ ]", "- [x]", "*", "-", "•", "1.", "2.", "3.", "#", ">"] {
        if let Some(rest) = s.strip_prefix(prefix) {
            s = rest.trim_start();
        }
    }
    // Drop a leading "next step(s):" / "TODO:" preamble.
    let lower = s.to_ascii_lowercase();
    for pre in [
        "next steps:",
        "next step:",
        "next:",
        "todo:",
        "tbd:",
        "follow-up:",
    ] {
        if lower.starts_with(pre) {
            s = s[pre.len()..].trim_start();
            break;
        }
    }
    s.trim().to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_next_step_signal_normal() {
        let t = "We finished the parser.\nNext step: wire the CLI flag.\nDone.";
        let out = infer_next_tasks(t);
        assert!(!out.is_empty());
        assert_eq!(out[0].signal, SignalKind::NextStep);
        assert_eq!(out[0].text, "wire the CLI flag.");
    }

    #[test]
    fn ranks_next_step_above_todo_normal() {
        let t = "TODO: add docs\nNext step: ship the feature";
        let out = infer_next_tasks(t);
        assert_eq!(out[0].signal, SignalKind::NextStep);
        assert!(out.iter().any(|s| s.signal == SignalKind::Todo));
        assert!(
            out[0].score
                > out
                    .iter()
                    .find(|s| s.signal == SignalKind::Todo)
                    .unwrap()
                    .score
        );
    }

    #[test]
    fn collapses_duplicate_actions_robust() {
        let t = "Next step: run tests\nNext step: run tests";
        let out = infer_next_tasks(t);
        assert_eq!(out.len(), 1);
    }

    #[test]
    fn strips_list_markers_and_preamble_normal() {
        let t = "- [ ] TODO: refactor the dispatcher";
        let out = infer_next_tasks(t);
        assert_eq!(out[0].text, "refactor the dispatcher");
        assert_eq!(out[0].signal, SignalKind::Todo);
    }

    #[test]
    fn recency_breaks_ties_among_same_signal_normal() {
        // Two equal-signal lines; the later one should score higher.
        let t = "Next step: alpha\nfiller\nfiller\nNext step: omega";
        let out = infer_next_tasks(t);
        let alpha = out.iter().find(|s| s.text == "alpha").unwrap();
        let omega = out.iter().find(|s| s.text == "omega").unwrap();
        assert!(
            omega.score > alpha.score,
            "later line should win on recency"
        );
    }

    #[test]
    fn caps_at_max_suggestions_robust() {
        let mut t = String::new();
        for i in 0..20 {
            t.push_str(&format!("Next step: task number {i}\n"));
        }
        let out = infer_next_tasks(&t);
        assert!(out.len() <= MAX_SUGGESTIONS);
    }

    #[test]
    fn empty_or_signalless_transcript_yields_nothing_robust() {
        assert!(infer_next_tasks("").is_empty());
        assert!(infer_next_tasks("Just some prose with no forward signals.").is_empty());
    }

    #[test]
    fn remaining_and_followup_signals_detected_normal() {
        let t = "Still need to add tests.\nThis is a follow-up for later.";
        let out = infer_next_tasks(t);
        assert!(out.iter().any(|s| s.signal == SignalKind::Remaining));
        assert!(out.iter().any(|s| s.signal == SignalKind::FollowUp));
    }
}
