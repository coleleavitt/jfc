//! Reasoning effort control.
//!
//! Maps to Anthropic's `reasoning_effort` API parameter which controls
//! how much "thinking" budget the model uses. Lower effort = faster + cheaper,
//! higher effort = more thorough reasoning.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::sync::RwLock;

/// Process-global slot mirroring the active session's effort pin.
/// `stream_response` reads this every turn so the API param flows
/// through without threading state through every call site.
static ACTIVE_EFFORT: RwLock<Option<String>> = RwLock::new(None);

/// One-shot per-turn effort override. When set, it wins over the session pin
/// (`ACTIVE_EFFORT`) for exactly ONE outbound request, then is consumed back to
/// `None` — so the next turn reverts to the session default. Mirrors Claude
/// Code's `turnEffort` (a turn-scoped effort that doesn't change the standing
/// session setting). Set via a per-turn `//effort <level>` marker.
static TURN_EFFORT: RwLock<Option<String>> = RwLock::new(None);

/// Set the one-shot per-turn effort override (consumed by the next request).
pub fn set_turn_effort(level: Option<ReasoningEffort>) {
    let mut guard = TURN_EFFORT.write().unwrap_or_else(|e| e.into_inner());
    *guard = level.map(|e| e.api_value().to_owned());
}

/// Peek the per-turn effort override without consuming it (for status/UI).
pub fn peek_turn_effort() -> Option<String> {
    TURN_EFFORT.read().ok().and_then(|g| g.clone())
}

/// Consume the one-shot per-turn effort override, returning it and clearing the
/// slot. Returns `None` when no per-turn override is pending. Called once per
/// outbound request by the effort resolver so the override applies to exactly
/// one turn.
pub fn take_turn_effort() -> Option<String> {
    let mut guard = TURN_EFFORT.write().unwrap_or_else(|e| e.into_inner());
    guard.take()
}

/// The effective effort for the next request: a pending per-turn override wins
/// over the session pin. Consumes the per-turn override. Returns `None` when
/// neither is set (adaptive exploration then decides).
pub fn resolve_effort_for_request() -> Option<String> {
    if let Some(turn) = take_turn_effort() {
        return Some(turn);
    }
    active_global()
}

/// Process-global fast-mode flag. When true, `stream_response` adds the
/// `fast-mode-2026-02-01` value to the `anthropic-beta` header so requests
/// are routed to Anthropic's low-latency inference path.
/// Mirrors v2.1.139's `/fast` command (Alt+O keybind).
static ACTIVE_FAST_MODE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Read the process-global fast-mode flag.
pub fn active_fast_mode() -> bool {
    ACTIVE_FAST_MODE.load(std::sync::atomic::Ordering::Relaxed)
}

/// Write the process-global fast-mode flag.
pub fn set_fast_mode_global(enabled: bool) {
    ACTIVE_FAST_MODE.store(enabled, std::sync::atomic::Ordering::Relaxed);
}

/// Snapshot the global effort param, if any.
pub fn active_global() -> Option<String> {
    ACTIVE_EFFORT.read().ok().and_then(|g| g.clone())
}

/// Reasoning effort levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[derive(Default)]
pub enum ReasoningEffort {
    Low,
    #[default]
    Medium,
    High,
    XHigh,
    Max,
}

impl ReasoningEffort {
    const ORDERED: [Self; 5] = [Self::Low, Self::Medium, Self::High, Self::XHigh, Self::Max];

    /// Parse from user input string.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "low" | "l" | "1" | "fast" => Some(Self::Low),
            "medium" | "med" | "m" | "2" | "normal" => Some(Self::Medium),
            "high" | "h" | "3" | "thorough" => Some(Self::High),
            "xhigh" | "x-high" | "x_high" | "xh" | "4" | "deeper" => Some(Self::XHigh),
            "max" | "maximum" | "5" | "ultra" => Some(Self::Max),
            _ => None,
        }
    }

    pub fn next(self) -> Option<Self> {
        Self::ORDERED
            .iter()
            .position(|level| *level == self)
            .and_then(|idx| Self::ORDERED.get(idx + 1).copied())
    }

    pub fn previous(self) -> Option<Self> {
        Self::ORDERED
            .iter()
            .position(|level| *level == self)
            .and_then(|idx| idx.checked_sub(1))
            .and_then(|idx| Self::ORDERED.get(idx).copied())
    }

    /// The API string value to send.
    pub fn api_value(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    /// Human-readable description.
    pub fn description(self) -> &'static str {
        match self {
            Self::Low => "Fast responses, less reasoning depth",
            Self::Medium => "Balanced speed and reasoning",
            Self::High => "Deep reasoning, slower",
            Self::XHigh => "Extra high reasoning depth",
            Self::Max => "Maximum reasoning depth, highest token use",
        }
    }
}

impl fmt::Display for ReasoningEffort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.api_value())
    }
}

/// Mutable effort state for a session.
#[derive(Debug, Clone)]
pub struct EffortState {
    /// Current effort level. None means "don't send the parameter" (server default).
    pub current: Option<ReasoningEffort>,
    /// Session-scoped `ultracode` standing mode (Claude Code parity). When on,
    /// effort is coerced to XHigh AND a standing system reminder tells the model
    /// to use the Workflow tool for every substantive task by default. Unlike
    /// the per-turn `ultrawork` keyword, this persists across turns until
    /// `/effort` clears it.
    pub ultracode: bool,
}

impl EffortState {
    pub fn new() -> Self {
        Self {
            current: None,
            ultracode: false,
        }
    }

    /// Set effort level. Returns a status message. Clears `ultracode` since an
    /// explicit lower level is an intentional downgrade from the standing mode.
    pub fn set(&mut self, level: ReasoningEffort) -> String {
        self.current = Some(level);
        self.ultracode = false;
        self.publish_global();
        format!(
            "Reasoning effort set to: {} ({})",
            level,
            level.description()
        )
    }

    /// Enable session `ultracode` mode: coerce effort to XHigh and turn on the
    /// standing "use Workflow by default" instruction. Returns a status message.
    pub fn set_ultracode(&mut self) -> String {
        self.current = Some(ReasoningEffort::XHigh);
        self.ultracode = true;
        self.publish_global();
        "ultracode ON \u{2014} xhigh effort + workflow-by-default for this session. \
         Token cost is not a constraint; chain multi-phase workflows for substantive tasks. \
         Use `/effort clear` to turn off."
            .to_string()
    }

    /// Whether `ultracode` standing mode is active.
    pub fn is_ultracode(&self) -> bool {
        self.ultracode
    }

    /// Clear effort (use server default) and exit `ultracode` mode.
    pub fn clear(&mut self) -> String {
        self.current = None;
        self.ultracode = false;
        self.publish_global();
        "Reasoning effort cleared (using server default)".to_string()
    }

    /// Get the current effort as an API parameter value, or None.
    pub fn api_param(&self) -> Option<&'static str> {
        self.current.map(|e| e.api_value())
    }

    /// Mirror the current effort into a process-global slot so
    /// `stream_response` can read it without threading the EffortState
    /// through every call site.
    pub fn publish_global(&self) {
        let mut guard = ACTIVE_EFFORT.write().unwrap_or_else(|e| e.into_inner());
        *guard = self.api_param().map(str::to_owned);
    }

    /// Format current status for display.
    pub fn status(&self) -> String {
        if self.ultracode {
            return "Reasoning effort: ultracode (xhigh + workflow orchestration; this session only)"
                .to_string();
        }
        match self.current {
            Some(e) => format!("Reasoning effort: {} ({})", e, e.description()),
            None => "Reasoning effort: default (not set)".to_string(),
        }
    }

    /// Short status-bar label for the effort/ultracode state, or `None` when
    /// nothing is pinned.
    pub fn badge(&self) -> Option<String> {
        if self.ultracode {
            return Some("ultracode".to_string());
        }
        self.current.map(|e| format!("effort {e}"))
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
    use serial_test::serial;

    #[test]
    fn parse_effort_levels() {
        assert_eq!(
            ReasoningEffort::from_str_loose("low"),
            Some(ReasoningEffort::Low)
        );
        assert_eq!(
            ReasoningEffort::from_str_loose("HIGH"),
            Some(ReasoningEffort::High)
        );
        assert_eq!(
            ReasoningEffort::from_str_loose("med"),
            Some(ReasoningEffort::Medium)
        );
        assert_eq!(
            ReasoningEffort::from_str_loose("fast"),
            Some(ReasoningEffort::Low)
        );
        assert_eq!(
            ReasoningEffort::from_str_loose("max"),
            Some(ReasoningEffort::Max)
        );
        assert_eq!(
            ReasoningEffort::from_str_loose("x-high"),
            Some(ReasoningEffort::XHigh)
        );
        assert_eq!(ReasoningEffort::from_str_loose("invalid"), None);
    }

    // Serialized: EffortState reads/writes the process-global ACTIVE_EFFORT
    // slot, so the effort tests can't run concurrently without racing.
    #[test]
    #[serial]
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
    #[serial]
    fn ultracode_mode_sets_xhigh_and_flag_normal() {
        let mut state = EffortState::new();
        assert!(!state.is_ultracode());
        state.set_ultracode();
        assert!(state.is_ultracode());
        // ultracode pins xhigh effort.
        assert_eq!(state.api_param(), Some("xhigh"));
        assert_eq!(state.badge().as_deref(), Some("ultracode"));
        // Reset the process-global effort slot so this test doesn't leak state
        // into other tests that read ACTIVE_EFFORT (e.g. exploration tests).
        state.clear();
    }

    #[test]
    #[serial]
    fn explicit_effort_clears_ultracode_normal() {
        let mut state = EffortState::new();
        state.set_ultracode();
        assert!(state.is_ultracode());
        // An explicit `/effort high` is an intentional downgrade.
        state.set(ReasoningEffort::High);
        assert!(!state.is_ultracode());
        assert_eq!(state.api_param(), Some("high"));
        state.clear();
    }

    #[test]
    #[serial]
    fn clear_exits_ultracode_robust() {
        let mut state = EffortState::new();
        state.set_ultracode();
        state.clear();
        assert!(!state.is_ultracode());
        assert_eq!(state.api_param(), None);
        assert_eq!(state.badge(), None);
    }

    // Serialized: this mutates the process-global TURN_EFFORT / ACTIVE_EFFORT
    // statics, so it can't run concurrently with the other turn-effort test
    // (they raced and intermittently failed under parallel execution).
    #[test]
    #[serial]
    fn turn_effort_overrides_session_and_is_consumed_normal() {
        // Session pin = low; per-turn override = max. The override wins for one
        // request, then is consumed so the next request reverts to the pin.
        let mut state = EffortState::new();
        state.set(ReasoningEffort::Low);
        set_turn_effort(Some(ReasoningEffort::Max));
        assert_eq!(peek_turn_effort().as_deref(), Some("max"));

        // First request: override wins.
        assert_eq!(resolve_effort_for_request().as_deref(), Some("max"));
        // Consumed: next request falls back to the session pin (low).
        assert_eq!(peek_turn_effort(), None);
        assert_eq!(resolve_effort_for_request().as_deref(), Some("low"));

        // Cleanup global state for other tests.
        set_turn_effort(None);
        state.clear();
    }

    #[test]
    #[serial]
    fn turn_effort_without_session_pin_then_reverts_robust() {
        // No session pin; a per-turn override applies once then leaves None.
        set_turn_effort(None);
        let mut state = EffortState::new();
        state.clear(); // ensure ACTIVE_EFFORT is None
        set_turn_effort(Some(ReasoningEffort::High));
        assert_eq!(resolve_effort_for_request().as_deref(), Some("high"));
        assert_eq!(resolve_effort_for_request(), None);
    }

    #[test]
    fn api_values() {
        assert_eq!(ReasoningEffort::Low.api_value(), "low");
        assert_eq!(ReasoningEffort::Medium.api_value(), "medium");
        assert_eq!(ReasoningEffort::High.api_value(), "high");
        assert_eq!(ReasoningEffort::XHigh.api_value(), "xhigh");
        assert_eq!(ReasoningEffort::Max.api_value(), "max");
    }

    #[test]
    fn effort_step_helpers() {
        assert_eq!(ReasoningEffort::Low.next(), Some(ReasoningEffort::Medium));
        assert_eq!(ReasoningEffort::High.next(), Some(ReasoningEffort::XHigh));
        assert_eq!(ReasoningEffort::Max.next(), None);
        assert_eq!(ReasoningEffort::Low.previous(), None);
        assert_eq!(
            ReasoningEffort::XHigh.previous(),
            Some(ReasoningEffort::High)
        );
    }
}
