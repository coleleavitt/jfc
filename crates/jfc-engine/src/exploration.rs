//! Exploration/sampling controls.
//!
//! The controller owns a single bounded "exploration level" and resolves it
//! to exactly one provider knob at request time:
//!   * thinking/adaptive Anthropic shapes -> `reasoning_effort`
//!   * non-thinking shapes -> `temperature`
//!
//! Explicit `/effort` and `/temp` pins still win. The adaptive level only
//! fills the gap when neither knob has been pinned by the user/config.

use std::sync::RwLock;

use jfc_provider::{StreamConvention, StreamOptions};

use crate::effort::ReasoningEffort;
use crate::slate::QueryClass;
use crate::types::ToolCall;

/// Process-global slot mirroring the active session's temperature pin.
static ACTIVE_TEMPERATURE: RwLock<Option<f64>> = RwLock::new(None);

/// Process-global adaptive exploration level for the in-flight turn. Kept
/// separate from explicit temperature/effort pins so `/temp` and `/effort`
/// remain hard overrides.
static ACTIVE_EXPLORATION_LEVEL: RwLock<Option<ExplorationLevel>> = RwLock::new(None);

const MIN_TEMPERATURE: f64 = 0.0;
const MAX_TEMPERATURE: f64 = 2.0;

pub fn active_temperature() -> Option<f64> {
    ACTIVE_TEMPERATURE.read().ok().and_then(|g| *g)
}

pub fn set_temperature_global(value: Option<f64>) {
    let mut guard = ACTIVE_TEMPERATURE
        .write()
        .unwrap_or_else(|e| e.into_inner());
    *guard = value;
}

pub fn active_exploration_level() -> Option<ExplorationLevel> {
    ACTIVE_EXPLORATION_LEVEL.read().ok().and_then(|g| *g)
}

pub fn set_exploration_level_global(value: Option<ExplorationLevel>) {
    let mut guard = ACTIVE_EXPLORATION_LEVEL
        .write()
        .unwrap_or_else(|e| e.into_inner());
    *guard = value;
}

pub fn parse_temperature(input: &str) -> Result<f64, String> {
    let trimmed = input.trim();
    let value = trimmed
        .parse::<f64>()
        .map_err(|_| format!("Couldn't parse `{trimmed}` as a temperature."))?;
    validate_temperature(value)
}

pub fn validate_temperature(value: f64) -> Result<f64, String> {
    if value.is_finite() && (MIN_TEMPERATURE..=MAX_TEMPERATURE).contains(&value) {
        Ok(value)
    } else {
        Err(format!(
            "Temperature must be between {MIN_TEMPERATURE:.1} and {MAX_TEMPERATURE:.1}."
        ))
    }
}

pub fn format_temperature(value: f64) -> String {
    let mut s = format!("{value:.3}");
    while s.contains('.') && s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

pub fn temperature_from_env() -> Option<f64> {
    let raw = std::env::var("JFC_TEMPERATURE").ok()?;
    match parse_temperature(&raw) {
        Ok(value) => Some(value),
        Err(reason) => {
            tracing::warn!(
                target: "jfc::exploration",
                value = %raw,
                reason,
                "ignoring invalid JFC_TEMPERATURE"
            );
            None
        }
    }
}

/// Walk config to pick the temperature for the active model.
///
/// Precedence mirrors persisted effort:
///   1. `[agents.<exact-model-id>]`
///   2. `[agents.<bare-model-id>]`
///   3. `[default]`
pub fn resolve_temperature_for_model(cfg: &crate::config::Config, model: &str) -> Option<f64> {
    let bare = model.rsplit('/').next().unwrap_or(model);
    let candidates = [
        cfg.agents.get(model).and_then(|a| a.temperature),
        (bare != model)
            .then(|| cfg.agents.get(bare).and_then(|a| a.temperature))
            .flatten(),
        cfg.default.temperature,
    ];
    candidates
        .into_iter()
        .flatten()
        .find_map(|value| match validate_temperature(value) {
            Ok(value) => Some(value),
            Err(reason) => {
                tracing::warn!(
                    target: "jfc::exploration",
                    temperature = value,
                    reason,
                    "ignoring invalid configured temperature"
                );
                None
            }
        })
}

#[derive(Debug, Clone, Default)]
pub struct TemperatureState {
    pub current: Option<f64>,
}

impl TemperatureState {
    pub fn new() -> Self {
        Self { current: None }
    }

    pub fn set(&mut self, value: f64) -> String {
        let value = validate_temperature(value).expect("temperature validated before set");
        self.current = Some(value);
        self.publish_global();
        format!("Temperature set to: {}", format_temperature(value))
    }

    pub fn clear(&mut self) -> String {
        self.current = None;
        self.publish_global();
        "Temperature cleared (using provider/model default)".to_owned()
    }

    pub fn publish_global(&self) {
        set_temperature_global(self.current);
    }

    pub fn status(&self) -> String {
        match self.current {
            Some(value) => format!("Temperature: {}", format_temperature(value)),
            None => "Temperature: default (not set)".to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ExplorationPolicy {
    Fixed,
    #[default]
    Adaptive,
}

impl ExplorationPolicy {
    pub fn from_str_loose(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "fixed" | "off" | "manual" => Some(Self::Fixed),
            "adaptive" | "auto" | "on" => Some(Self::Adaptive),
            _ => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Fixed => "fixed",
            Self::Adaptive => "adaptive",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct ExplorationLevel(u8);

impl ExplorationLevel {
    pub const MIN: Self = Self(0);
    pub const MAX: Self = Self(4);

    pub fn new(value: u8) -> Self {
        Self(value.min(Self::MAX.0))
    }

    pub fn as_u8(self) -> u8 {
        self.0
    }

    pub fn step_by(self, delta: i8) -> Self {
        let raw = (self.0 as i16 + delta as i16).clamp(Self::MIN.0 as i16, Self::MAX.0 as i16);
        Self(raw as u8)
    }

    pub fn clamp(self, min: Self, max: Self) -> Self {
        Self(self.0.clamp(min.0.min(max.0), min.0.max(max.0)))
    }

    pub fn to_effort(self) -> ReasoningEffort {
        match self.0 {
            0 => ReasoningEffort::Low,
            1 => ReasoningEffort::Medium,
            2 => ReasoningEffort::High,
            3 => ReasoningEffort::XHigh,
            _ => ReasoningEffort::Max,
        }
    }

    pub fn from_effort(effort: ReasoningEffort) -> Self {
        match effort {
            ReasoningEffort::Low => Self(0),
            ReasoningEffort::Medium => Self(1),
            ReasoningEffort::High => Self(2),
            ReasoningEffort::XHigh => Self(3),
            ReasoningEffort::Max => Self(4),
        }
    }

    pub fn from_temperature(value: f64) -> Self {
        let normalized = (value.clamp(0.0, 1.0) * 4.0).round() as u8;
        Self::new(normalized)
    }

    pub fn to_temperature(self) -> f64 {
        f64::from(self.0) * 0.25
    }
}

impl std::fmt::Display for ExplorationLevel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExplorationSignal {
    StreamRetry,
    AssistantStall,
    RepeatedToolCall,
    ToolFailures,
}

impl ExplorationSignal {
    fn label(self) -> &'static str {
        match self {
            Self::StreamRetry => "stream-retry",
            Self::AssistantStall => "assistant-stall",
            Self::RepeatedToolCall => "repeated-tool-call",
            Self::ToolFailures => "tool-failures",
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct ExplorationSettings {
    pub policy: ExplorationPolicy,
    pub min_level: ExplorationLevel,
    pub max_level: ExplorationLevel,
    pub decay: u8,
}

impl ExplorationSettings {
    pub fn from_config(cfg: &crate::config::Config) -> Self {
        let mut policy = cfg
            .exploration
            .as_ref()
            .and_then(|c| c.policy.as_deref())
            .and_then(ExplorationPolicy::from_str_loose)
            .unwrap_or_default();
        if let Ok(raw) = std::env::var("JFC_EXPLORATION_POLICY") {
            match ExplorationPolicy::from_str_loose(&raw) {
                Some(p) => policy = p,
                None => tracing::warn!(
                    target: "jfc::exploration",
                    value = %raw,
                    "ignoring invalid JFC_EXPLORATION_POLICY"
                ),
            }
        }
        let cfg_exploration = cfg.exploration.as_ref();
        let min_level = cfg_exploration
            .and_then(|c| c.min_level)
            .map(ExplorationLevel::new)
            .unwrap_or(ExplorationLevel::MIN);
        let max_level = cfg_exploration
            .and_then(|c| c.max_level)
            .map(ExplorationLevel::new)
            .unwrap_or(ExplorationLevel::MAX);
        let decay = cfg_exploration.and_then(|c| c.decay).unwrap_or(1).max(1);
        Self {
            policy,
            min_level,
            max_level,
            decay,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ExplorationState {
    pub policy: ExplorationPolicy,
    pub baseline: ExplorationLevel,
    pub current: ExplorationLevel,
    pub sticky_delta: i8,
    adaptive_delta: i8,
    min_level: ExplorationLevel,
    max_level: ExplorationLevel,
    decay: u8,
    force_next_level: Option<ExplorationLevel>,
    last_tool_signature: Option<String>,
    repeated_tool_count: u8,
    tool_failures_this_turn: u8,
}

impl Default for ExplorationState {
    fn default() -> Self {
        Self::new()
    }
}

impl ExplorationState {
    pub fn new() -> Self {
        Self {
            policy: ExplorationPolicy::default(),
            baseline: ExplorationLevel::new(1),
            current: ExplorationLevel::new(1),
            sticky_delta: 0,
            adaptive_delta: 0,
            min_level: ExplorationLevel::MIN,
            max_level: ExplorationLevel::MAX,
            decay: 1,
            force_next_level: None,
            last_tool_signature: None,
            repeated_tool_count: 0,
            tool_failures_this_turn: 0,
        }
    }

    pub fn configure(&mut self, settings: ExplorationSettings) {
        self.policy = settings.policy;
        self.min_level = settings.min_level;
        self.max_level = settings.max_level;
        self.decay = settings.decay;
        self.current = self.current.clamp(self.min_level, self.max_level);
        self.publish_global();
    }

    pub fn begin_turn(&mut self, user_text: &str, cfg: &crate::config::Config) -> QueryClass {
        self.configure(ExplorationSettings::from_config(cfg));
        let class = QueryClass::from_query(user_text);
        self.baseline = baseline_for_query_class(class, cfg).clamp(self.min_level, self.max_level);
        let forced = self.force_next_level.take();
        let mut target = self
            .baseline
            .step_by(self.sticky_delta.saturating_add(self.adaptive_delta))
            .clamp(self.min_level, self.max_level);
        if let Some(level) = forced {
            target = target.max(level.clamp(self.min_level, self.max_level));
        }
        self.current = target;
        self.last_tool_signature = None;
        self.repeated_tool_count = 0;
        self.tool_failures_this_turn = 0;
        self.publish_global();
        tracing::info!(
            target: "jfc::exploration",
            policy = self.policy.label(),
            class = ?class,
            baseline = self.baseline.as_u8(),
            current = self.current.as_u8(),
            sticky_delta = self.sticky_delta,
            adaptive_delta = self.adaptive_delta,
            "exploration turn baseline resolved"
        );
        class
    }

    pub fn force_next(&mut self, level: ExplorationLevel) {
        self.force_next_level = Some(level);
    }

    pub fn adjust_sticky(&mut self, delta: i8) -> String {
        self.policy = ExplorationPolicy::Adaptive;
        self.sticky_delta = (self.sticky_delta + delta).clamp(-4, 4);
        self.current = self
            .baseline
            .step_by(self.sticky_delta.saturating_add(self.adaptive_delta))
            .clamp(self.min_level, self.max_level);
        self.publish_global();
        format!(
            "Exploration {} to level {} (sticky {:+}).",
            if delta >= 0 { "raised" } else { "lowered" },
            self.current,
            self.sticky_delta
        )
    }

    pub fn clear_adjustments(&mut self) -> String {
        self.sticky_delta = 0;
        self.adaptive_delta = 0;
        self.force_next_level = None;
        self.current = self.baseline.clamp(self.min_level, self.max_level);
        self.publish_global();
        "Exploration adjustments cleared.".to_owned()
    }

    pub fn bump_for_signal(&mut self, signal: ExplorationSignal) -> bool {
        if self.policy != ExplorationPolicy::Adaptive {
            return false;
        }
        let before = self.current;
        self.adaptive_delta = (self.adaptive_delta + 1).clamp(0, 4);
        self.current = self
            .baseline
            .step_by(self.sticky_delta.saturating_add(self.adaptive_delta))
            .clamp(self.min_level, self.max_level);
        self.publish_global();
        let changed = self.current != before;
        tracing::info!(
            target: "jfc::exploration",
            signal = signal.label(),
            before = before.as_u8(),
            current = self.current.as_u8(),
            adaptive_delta = self.adaptive_delta,
            changed,
            "exploration signal processed"
        );
        changed
    }

    pub fn decay_after_progress(&mut self) -> bool {
        if self.policy != ExplorationPolicy::Adaptive || self.adaptive_delta <= 0 {
            return false;
        }
        let before = self.current;
        self.adaptive_delta = (self.adaptive_delta - self.decay as i8).max(0);
        self.current = self
            .baseline
            .step_by(self.sticky_delta.saturating_add(self.adaptive_delta))
            .clamp(self.min_level, self.max_level);
        self.publish_global();
        tracing::info!(
            target: "jfc::exploration",
            before = before.as_u8(),
            current = self.current.as_u8(),
            adaptive_delta = self.adaptive_delta,
            "exploration decayed after progress"
        );
        self.current != before
    }

    pub fn record_tool_call(&mut self, tool: &ToolCall) -> bool {
        let signature = format!("{}:{}", tool.kind.label(), tool.input.to_value());
        if self.last_tool_signature.as_deref() == Some(signature.as_str()) {
            self.repeated_tool_count = self.repeated_tool_count.saturating_add(1);
        } else {
            self.last_tool_signature = Some(signature);
            self.repeated_tool_count = 1;
        }
        if self.repeated_tool_count >= 3 {
            self.repeated_tool_count = 0;
            self.bump_for_signal(ExplorationSignal::RepeatedToolCall)
        } else {
            false
        }
    }

    pub fn record_tool_result(&mut self, is_error: bool) -> bool {
        if is_error {
            self.tool_failures_this_turn = self.tool_failures_this_turn.saturating_add(1);
            if self.tool_failures_this_turn >= 2 {
                self.tool_failures_this_turn = 0;
                return self.bump_for_signal(ExplorationSignal::ToolFailures);
            }
        }
        false
    }

    pub fn publish_global(&self) {
        if self.policy == ExplorationPolicy::Adaptive {
            set_exploration_level_global(Some(self.current));
        } else {
            set_exploration_level_global(None);
        }
    }

    pub fn status(&self) -> String {
        format!(
            "Exploration: {} level {} (baseline {}, sticky {:+}, adaptive {:+})",
            self.policy.label(),
            self.current,
            self.baseline,
            self.sticky_delta,
            self.adaptive_delta
        )
    }
}

fn baseline_for_query_class(class: QueryClass, cfg: &crate::config::Config) -> ExplorationLevel {
    if let Some(configured) = cfg.categories.get(class.slug())
        && let Some(level) = configured_category_level(configured)
    {
        return level;
    }
    match class {
        QueryClass::Trivial => ExplorationLevel::new(0),
        QueryClass::CodeEdit | QueryClass::Research => ExplorationLevel::new(1),
        QueryClass::Refactor | QueryClass::LongContext => ExplorationLevel::new(2),
        QueryClass::Exploration => ExplorationLevel::new(3),
    }
}

fn configured_category_level(category: &crate::config::CategoryConfig) -> Option<ExplorationLevel> {
    if let Some(effort) = category
        .reasoning_effort
        .as_deref()
        .and_then(ReasoningEffort::from_str_loose)
    {
        return Some(ExplorationLevel::from_effort(effort));
    }
    category
        .temperature
        .and_then(|t| validate_temperature(t).ok())
        .map(ExplorationLevel::from_temperature)
}

pub fn apply_to_stream_options(
    mut opts: StreamOptions,
    provider_name: &str,
    convention: StreamConvention,
) -> StreamOptions {
    let has_anthropic_thinking = matches!(convention, StreamConvention::AnthropicNative)
        && (opts.adaptive_thinking || opts.thinking_budget.is_some());
    let oauth_locked_temperature = provider_name == "anthropic-oauth";
    // A pending per-turn effort override wins over the session pin and is
    // consumed here (one request only); otherwise fall back to the session pin.
    let manual_effort = crate::effort::resolve_effort_for_request();
    let manual_temperature = active_temperature();

    if let Some(effort) = manual_effort {
        opts = opts.reasoning_effort(effort);
    }
    if let Some(temperature) = manual_temperature {
        if has_anthropic_thinking || oauth_locked_temperature {
            tracing::debug!(
                target: "jfc::exploration",
                temperature,
                has_anthropic_thinking,
                oauth_locked_temperature,
                "temperature pin not applied for this request shape"
            );
        } else {
            opts = opts.temperature(temperature);
        }
    }

    if opts.reasoning_effort.is_some() || opts.temperature.is_some() {
        return opts;
    }
    let Some(level) = active_exploration_level() else {
        return opts;
    };
    if has_anthropic_thinking {
        opts = opts.reasoning_effort(level.to_effort().api_value());
        tracing::debug!(
            target: "jfc::exploration",
            level = level.as_u8(),
            effort = %level.to_effort(),
            "adaptive exploration resolved to reasoning_effort"
        );
    } else if !oauth_locked_temperature {
        let temperature = level.to_temperature();
        opts = opts.temperature(temperature);
        tracing::debug!(
            target: "jfc::exploration",
            level = level.as_u8(),
            temperature,
            "adaptive exploration resolved to temperature"
        );
    }
    opts
}

#[cfg(test)]
mod tests {
    use super::*;

    struct ExplorationGlobalGuard;

    impl ExplorationGlobalGuard {
        fn new() -> Self {
            set_temperature_global(None);
            set_exploration_level_global(None);
            crate::effort::EffortState::new().publish_global();
            Self
        }
    }

    impl Drop for ExplorationGlobalGuard {
        fn drop(&mut self) {
            set_temperature_global(None);
            set_exploration_level_global(None);
            crate::effort::EffortState::new().publish_global();
        }
    }

    #[test]
    fn parse_temperature_accepts_range_normal() {
        assert_eq!(parse_temperature("0").unwrap(), 0.0);
        assert_eq!(parse_temperature("0.7").unwrap(), 0.7);
        assert_eq!(parse_temperature("2").unwrap(), 2.0);
    }

    #[test]
    fn parse_temperature_rejects_out_of_range_robust() {
        assert!(parse_temperature("-0.1").is_err());
        assert!(parse_temperature("2.1").is_err());
        assert!(parse_temperature("NaN").is_err());
        assert!(parse_temperature("hot").is_err());
    }

    #[test]
    fn resolve_temperature_for_model_uses_exact_bare_default_precedence_normal() {
        let mut cfg = crate::config::Config::default();
        cfg.default.temperature = Some(0.2);
        cfg.agents.insert(
            "claude-opus-4-8".to_owned(),
            crate::config::AgentConfig {
                temperature: Some(0.4),
                ..Default::default()
            },
        );
        cfg.agents.insert(
            "anthropic/claude-opus-4-8".to_owned(),
            crate::config::AgentConfig {
                temperature: Some(0.6),
                ..Default::default()
            },
        );

        assert_eq!(
            resolve_temperature_for_model(&cfg, "anthropic/claude-opus-4-8"),
            Some(0.6)
        );
        assert_eq!(
            resolve_temperature_for_model(&cfg, "anthropic/claude-sonnet-4-6"),
            Some(0.2)
        );
    }

    #[test]
    #[serial_test::serial]
    fn temperature_state_lifecycle_normal() {
        let _guard = ExplorationGlobalGuard::new();
        let mut state = TemperatureState::new();
        assert_eq!(state.current, None);

        state.set(0.9);
        assert_eq!(state.current, Some(0.9));
        assert_eq!(active_temperature(), Some(0.9));

        state.clear();
        assert_eq!(state.current, None);
        assert_eq!(active_temperature(), None);
    }

    #[test]
    fn exploration_level_maps_to_single_knobs_normal() {
        let low = ExplorationLevel::new(0);
        let max = ExplorationLevel::new(4);
        assert_eq!(low.to_effort(), ReasoningEffort::Low);
        assert_eq!(max.to_effort(), ReasoningEffort::Max);
        assert_eq!(low.to_temperature(), 0.0);
        assert_eq!(max.to_temperature(), 1.0);
    }

    #[test]
    #[serial_test::serial]
    fn exploration_state_bumps_and_decays_bounded_normal() {
        let _guard = ExplorationGlobalGuard::new();
        let cfg = crate::config::Config::default();
        let mut state = ExplorationState::new();
        state.begin_turn("explain the architecture", &cfg);
        assert_eq!(state.baseline, ExplorationLevel::new(3));
        assert_eq!(state.current, ExplorationLevel::new(3));

        state.bump_for_signal(ExplorationSignal::AssistantStall);
        assert_eq!(state.current, ExplorationLevel::new(4));

        state.bump_for_signal(ExplorationSignal::StreamRetry);
        assert_eq!(state.current, ExplorationLevel::new(4));

        state.decay_after_progress();
        assert_eq!(state.current, ExplorationLevel::new(4));
        state.decay_after_progress();
        assert_eq!(state.current, ExplorationLevel::new(3));
    }

    #[test]
    #[serial_test::serial]
    fn category_config_overrides_builtin_baseline_normal() {
        let _guard = ExplorationGlobalGuard::new();
        let mut cfg = crate::config::Config::default();
        cfg.categories.insert(
            "refactor".to_owned(),
            crate::config::CategoryConfig {
                reasoning_effort: Some("max".to_owned()),
                ..Default::default()
            },
        );
        let mut state = ExplorationState::new();
        state.begin_turn("refactor all async code", &cfg);
        assert_eq!(state.baseline, ExplorationLevel::new(4));
    }

    #[test]
    #[serial_test::serial]
    fn adaptive_resolves_to_effort_for_thinking_regression() {
        let _guard = ExplorationGlobalGuard::new();
        set_temperature_global(None);
        set_exploration_level_global(Some(ExplorationLevel::new(2)));
        let opts = StreamOptions::new("claude-opus-4-8").adaptive();

        let opts = apply_to_stream_options(opts, "anthropic", StreamConvention::AnthropicNative);

        assert_eq!(opts.reasoning_effort.as_deref(), Some("high"));
        assert_eq!(opts.temperature, None);
    }

    #[test]
    #[serial_test::serial]
    fn adaptive_resolves_to_temperature_without_thinking_normal() {
        let _guard = ExplorationGlobalGuard::new();
        set_temperature_global(None);
        set_exploration_level_global(Some(ExplorationLevel::new(3)));
        let opts = StreamOptions::new("claude-haiku-4-5");

        let opts = apply_to_stream_options(opts, "anthropic", StreamConvention::AnthropicNative);

        assert_eq!(opts.reasoning_effort, None);
        assert_eq!(opts.temperature, Some(0.75));
    }
}
