//! Reasoning effort control.
//!
//! Maps to Anthropic's `reasoning_effort` API parameter which controls
//! how much "thinking" budget the model uses. Lower effort = faster + cheaper,
//! higher effort = more thorough reasoning.

use std::fmt;
use serde::{Deserialize, Serialize};

/// Reasoning effort levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReasoningEffort {
    Low,
    Medium,
    High,
}

impl ReasoningEffort {
    /// Parse from user input string.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "low" | "l" | "1" | "fast" => Some(Self::Low),
            "medium" | "med" | "m" | "2" | "normal" => Some(Self::Medium),
            "high" | "h" | "3" | "max" | "thorough" => Some(Self::High),
            _ => None,
        }
    }

    /// The API string value to send.
    pub fn api_value(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    /// Human-readable description.
    pub fn description(self) -> &'static str {
        match self {
            Self::Low => "Fast responses, less reasoning depth",
            Self::Medium => "Balanced speed and reasoning",
            Self::High => "Maximum reasoning depth, slower",
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.api_value())
    }
}

impl Default for ReasoningEffort {
    fn default() -> Self {
        Self::Medium
    }
}

/// Mutable effort state for a session.
#[derive(Debug, Clone)]
pub struct EffortState {
    /// Current effort level. None means "don't send the parameter" (server default).
    pub current: Option<ReasoningEffort>,
}

impl EffortState {
    pub fn new() -> Self {
        Self { current: None }
    }

    /// Set effort level. Returns a status message.
    pub fn set(&mut self, level: ReasoningEffort) -> String {
        self.current = Some(level);
        format!("Reasoning effort set to: {} ({})", level, level.description())
    }

    /// Clear effort (use server default).
    pub fn clear(&mut self) -> String {
        self.current = None;
        "Reasoning effort cleared (using server default)".to_string()
    }

    /// Get the current effort as an API parameter value, or None.
    pub fn api_param(&self) -> Option<&'static str> {
        self.current.map(|e| e.api_value())
    }

    /// Format current status for display.
    pub fn status(&self) -> String {
        match self.current {
            Some(e) => format!("Reasoning effort: {} ({})", e, e.description()),
            None => "Reasoning effort: default (not set)".to_string(),
        }
    }
}

impl Default for EffortState {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_effort_levels() {
        assert_eq!(ReasoningEffort::from_str_loose("low"), Some(ReasoningEffort::Low));
        assert_eq!(ReasoningEffort::from_str_loose("HIGH"), Some(ReasoningEffort::High));
        assert_eq!(ReasoningEffort::from_str_loose("med"), Some(ReasoningEffort::Medium));
        assert_eq!(ReasoningEffort::from_str_loose("fast"), Some(ReasoningEffort::Low));
        assert_eq!(ReasoningEffort::from_str_loose("max"), Some(ReasoningEffort::High));
        assert_eq!(ReasoningEffort::from_str_loose("invalid"), None);
    }

    #[test]
    fn effort_state_lifecycle() {
        let mut state = EffortState::new();
        assert_eq!(state.api_param(), None);

        state.set(ReasoningEffort::High);
        assert_eq!(state.api_param(), Some("high"));

        state.set(ReasoningEffort::Low);
        assert_eq!(state.api_param(), Some("low"));

        state.clear();
        assert_eq!(state.api_param(), None);
    }

    #[test]
    fn api_values() {
        assert_eq!(ReasoningEffort::Low.api_value(), "low");
        assert_eq!(ReasoningEffort::Medium.api_value(), "medium");
        assert_eq!(ReasoningEffort::High.api_value(), "high");
    }
}
