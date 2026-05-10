//! Post-turn summary classification.
//!
//! After each agent turn, classifies the outcome into a structured status
//! for fleet dashboards, cron status reporting, and remote observers.

use serde::{Deserialize, Serialize};

/// Structured turn outcome.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TurnSummary {
    /// High-level status category.
    pub status: TurnStatus,
    /// Human-readable detail about what happened.
    pub detail: String,
    /// What action is needed (if any).
    pub needs_action: Option<String>,
    /// Tools used this turn.
    pub tools_used: Vec<String>,
    /// Token count for this turn.
    pub tokens_used: usize,
}

/// Turn status categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TurnStatus {
    /// Agent is actively working.
    Running,
    /// Agent completed successfully this turn.
    Completed,
    /// Agent is blocked waiting for something.
    Blocked,
    /// Agent's work is ready for review.
    ReviewReady,
    /// Agent is idle (waiting for input).
    Idle,
    /// Agent encountered an error.
    Error,
}

impl TurnStatus {
    /// Short display label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Blocked => "blocked",
            Self::ReviewReady => "review_ready",
            Self::Idle => "idle",
            Self::Error => "error",
        }
    }

    /// Emoji for TUI display.
    pub fn emoji(self) -> &'static str {
        match self {
            Self::Running => "🔄",
            Self::Completed => "✅",
            Self::Blocked => "🚫",
            Self::ReviewReady => "👀",
            Self::Idle => "💤",
            Self::Error => "❌",
        }
    }
}

/// Classify a turn based on what happened.
pub fn classify_turn(
    tools_used: &[String],
    had_error: bool,
    is_waiting_permission: bool,
    is_idle: bool,
    task_completed: bool,
    last_response_text: &str,
) -> TurnSummary {
    let status = if had_error {
        TurnStatus::Error
    } else if is_waiting_permission {
        TurnStatus::Blocked
    } else if task_completed {
        TurnStatus::Completed
    } else if is_idle {
        TurnStatus::Idle
    } else if looks_like_review_ready(last_response_text) {
        TurnStatus::ReviewReady
    } else {
        TurnStatus::Running
    };

    let detail = build_detail(&status, tools_used, last_response_text);
    let needs_action = build_needs_action(&status, is_waiting_permission);

    TurnSummary {
        status,
        detail,
        needs_action,
        tools_used: tools_used.to_vec(),
        tokens_used: 0, // Caller fills this in
    }
}

fn looks_like_review_ready(text: &str) -> bool {
    let lower = text.to_lowercase();
    lower.contains("ready for review")
        || lower.contains("please review")
        || lower.contains("changes are complete")
        || lower.contains("pr is ready")
        || lower.contains("implementation complete")
}

fn build_detail(status: &TurnStatus, tools: &[String], text: &str) -> String {
    match status {
        TurnStatus::Running => {
            if tools.is_empty() {
                "Thinking...".to_string()
            } else {
                format!("Used: {}", tools.join(", "))
            }
        }
        TurnStatus::Completed => "Task completed".to_string(),
        TurnStatus::Blocked => "Waiting for permission".to_string(),
        TurnStatus::ReviewReady => {
            // Extract first line as preview
            text.lines()
                .next()
                .unwrap_or("Ready for review")
                .to_string()
        }
        TurnStatus::Idle => "Waiting for input".to_string(),
        TurnStatus::Error => "Encountered an error".to_string(),
    }
}

fn build_needs_action(status: &TurnStatus, is_permission: bool) -> Option<String> {
    match status {
        TurnStatus::Blocked if is_permission => {
            Some("Approve or deny pending permission".to_string())
        }
        TurnStatus::Blocked => Some("Unblock the agent".to_string()),
        TurnStatus::ReviewReady => Some("Review the agent's work".to_string()),
        TurnStatus::Error => Some("Check error and retry or abort".to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_running() {
        let summary = classify_turn(
            &["bash".to_string(), "read".to_string()],
            false,
            false,
            false,
            false,
            "I'll check the tests next",
        );
        assert_eq!(summary.status, TurnStatus::Running);
        assert!(summary.detail.contains("bash"));
    }

    #[test]
    fn classify_blocked() {
        let summary = classify_turn(
            &["edit".to_string()],
            false,
            true,
            false,
            false,
            "Waiting for approval",
        );
        assert_eq!(summary.status, TurnStatus::Blocked);
        assert!(summary.needs_action.is_some());
    }

    #[test]
    fn classify_completed() {
        let summary = classify_turn(&[], false, false, false, true, "All done!");
        assert_eq!(summary.status, TurnStatus::Completed);
    }

    #[test]
    fn classify_review_ready() {
        let summary = classify_turn(
            &[],
            false,
            false,
            false,
            false,
            "The implementation is complete and ready for review.",
        );
        assert_eq!(summary.status, TurnStatus::ReviewReady);
    }

    #[test]
    fn status_labels() {
        assert_eq!(TurnStatus::Running.label(), "running");
        assert_eq!(TurnStatus::Blocked.emoji(), "🚫");
        assert_eq!(TurnStatus::Completed.emoji(), "✅");
    }
}
